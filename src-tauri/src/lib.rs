use notify_debouncer_mini::{
    new_debouncer,
    notify::{RecursiveMode, RecommendedWatcher},
    DebounceEventResult, Debouncer,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

mod ai_conversations;
mod challenge_review;
mod depth_decisions;
mod cognitive_report;
mod knowforge_analytics;
mod llm;
mod note_privacy;
mod passive_highlight;
mod writing_coach;
mod thought_parser;
mod thought_reconcile;
mod thought_retrieval;
mod vault_config;
mod vault_thoughts_db;
mod vault_context_search;
mod builtin_embed;
mod rebuild_progress;
mod semantic_index;
mod workspace_text_search;
mod understanding_graph;
mod link_recommendation;
mod topic_network;
mod tools;
mod skills;

/// 供前端展示的文件树节点（目录带 children，Markdown 文件为叶子）
#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub name: String,
    /// 相对根目录的路径，统一使用 `/` 分隔
    pub rel_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    /// 仅叶子：由文件头解析 `kf-private`；目录为 `None`。
    #[serde(rename = "kfPrivate", skip_serializing_if = "Option::is_none")]
    pub kf_private: Option<bool>,
}

/// 推送到前端的磁盘变更事件载荷（camelCase 与 TS 对齐）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkdownDiskChangedPayload {
    rel_path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkdownFileSignature {
    size_bytes: u64,
    modified_ns: String,
}

struct WorkspaceState {
    /// 当前已授权工作区根目录，仅允许命令访问这里
    root: Mutex<Option<PathBuf>>,
    /// 本应用已成功写入的路径与时间，用于忽略紧随其后的 notify
    last_self_write: Arc<Mutex<Option<(String, Instant)>>>,
    /// 每个已打开 Tab 对应一个 debounced 文件监听（非活动标签也能感知外部修改）
    markdown_watchers: Mutex<HashMap<String, Debouncer<RecommendedWatcher>>>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            root: Mutex::new(None),
            last_self_write: Arc::new(Mutex::new(None)),
            markdown_watchers: Mutex::new(HashMap::new()),
        }
    }
}

pub(crate) fn is_markdown_path(path: &Path) -> bool {
    path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "md" || e == "markdown"
        })
        .unwrap_or(false)
}

fn rel_path_components_ok(rel_path: &str) -> Result<(), String> {
    if rel_path.is_empty() {
        return Err("empty relative path".to_string());
    }
    for part in rel_path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("invalid relative path".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tree_node_tests {
    use super::TreeNode;
    use serde_json::json;

    #[test]
    fn tree_node_serializes_kf_private_camel() {
        let n = TreeNode {
            name: "x.md".into(),
            rel_path: "d/x.md".into(),
            children: None,
            kf_private: Some(true),
        };
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v.get("kfPrivate"), Some(&json!(true)));
    }
}

pub(crate) fn join_under_root(canonical_root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    rel_path_components_ok(rel_path)?;
    let mut out = canonical_root.to_path_buf();
    for part in rel_path.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    Ok(out)
}

fn sha256_hex_bytes(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// 校验前端传入的 SHA-256 十六进制串（小写比较）
fn normalize_client_sha256_hex(s: &str) -> Result<String, String> {
    let t = s.trim().to_ascii_lowercase();
    if t.len() != 64 {
        return Err("DISK_BASELINE_INVALID: expected 64 hex characters".to_string());
    }
    if !t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err("DISK_BASELINE_INVALID: non-hex character in sha256".to_string());
    }
    Ok(t)
}


/// 递归构建 Markdown 工作区树：Markdown 文件为叶子；目录始终入树（含无子项的空文件夹），便于新建文件夹后立即可见
fn build_md_tree(canonical_root: &Path, dir: &Path) -> Result<Vec<TreeNode>, String> {
    let mut nodes = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| sanitize_io_error(e, "listing workspace directory"))?;

    for entry in entries {
        let entry = entry.map_err(|e| sanitize_io_error(e, "reading directory entry"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
        // 为避免越界与循环，工作区树中跳过符号链接入口
        if meta.file_type().is_symlink() {
            continue;
        }
        let rel = path
            .strip_prefix(canonical_root)
            .map_err(|_| "entry not under root".to_string())?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if meta.is_dir() {
            let children = build_md_tree(canonical_root, &path)?;
            nodes.push(TreeNode {
                name,
                rel_path: rel_str,
                children: Some(children),
                kf_private: None,
            });
        } else if meta.is_file() && is_markdown_path(&path) {
            let kf_private = Some(note_privacy::peek_kf_private_from_md_file(&path));
            nodes.push(TreeNode {
                name,
                rel_path: rel_str,
                children: None,
                kf_private,
            });
        }
    }

    nodes.sort_by(|a, b| {
        let a_dir = a.children.is_some();
        let b_dir = b.children.is_some();
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    Ok(nodes)
}

/// 将 I/O 错误转为前端可展示的英文文案；权限拒绝时附带操作上下文便于排查
pub(crate) fn sanitize_io_error(err: std::io::Error, permission_context: &'static str) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "File not found".to_string(),
        std::io::ErrorKind::PermissionDenied => {
            format!("Permission denied ({permission_context})")
        }
        std::io::ErrorKind::AlreadyExists => "File already exists".to_string(),
        _ => "Operation failed".to_string(),
    }
}

fn lock_workspace_root(state: &tauri::State<'_, WorkspaceState>) -> Result<PathBuf, String> {
    let guard = match state.root.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            eprintln!("[workspace] mutex was poisoned, recovering inner data");
            poisoned.into_inner()
        }
    };
    guard
        .clone()
        .ok_or_else(|| "workspace is not initialized".to_string())
}

#[tauri::command]
async fn open_workspace(
    root: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<TreeNode>, String> {
    {
        {
            let mut w = match state.markdown_watchers.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    eprintln!("[workspace] markdown_watchers mutex was poisoned, recovering");
                    poisoned.into_inner()
                }
            };
            w.clear();
        }
        let mut guard = match state.root.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                eprintln!("[workspace] mutex was poisoned, recovering inner data");
                poisoned.into_inner()
            }
        };
        *guard = None;
    }

    let root = root.trim().to_string();
    let (canonical_root, nodes) = tauri::async_runtime::spawn_blocking(move || {
        let root_path = PathBuf::from(root);
        let canonical_root =
            fs::canonicalize(&root_path).map_err(|e| sanitize_io_error(e, "resolving workspace path"))?;
        let meta = fs::metadata(&canonical_root).map_err(|e| sanitize_io_error(e, "reading workspace folder"))?;
        if !meta.is_dir() {
            return Err("root is not a directory".to_string());
        }
        let nodes = build_md_tree(&canonical_root, &canonical_root)?;
        Ok::<(PathBuf, Vec<TreeNode>), String>((canonical_root, nodes))
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut guard = match state.root.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            eprintln!("[workspace] mutex was poisoned, recovering inner data");
            poisoned.into_inner()
        }
    };
    *guard = Some(canonical_root.clone());
    let reconcile_root = canonical_root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = thought_reconcile::reconcile_thought_rel_paths_blocking(&reconcile_root) {
            eprintln!("[thought_reconcile] skipped: {e}");
        }
    });
    Ok(nodes)
}

// 不改变已打开工作区与文件监听，仅重新扫描磁盘构建 Markdown 树（新建文件后刷新列表用）
#[tauri::command]
async fn refresh_md_tree(state: tauri::State<'_, WorkspaceState>) -> Result<Vec<TreeNode>, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || build_md_tree(&canonical_root, &canonical_root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn read_markdown_file(
    rel_path: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<String, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        let canonical_file =
            fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        if !is_markdown_path(&canonical_file) {
            return Err("not a markdown file".to_string());
        }
        fs::read_to_string(&canonical_file).map_err(|e| sanitize_io_error(e, "reading file"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_markdown_file_signature(
    rel_path: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<MarkdownFileSignature, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        let canonical_file =
            fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        if !is_markdown_path(&canonical_file) {
            return Err("not a markdown file".to_string());
        }
        let meta = fs::metadata(&canonical_file)
            .map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
        let modified = meta
            .modified()
            .map_err(|e| sanitize_io_error(e, "reading file modified time"))?;
        let modified_ns = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "file modified time is before unix epoch".to_string())?
            .as_nanos()
            .to_string();
        Ok(MarkdownFileSignature {
            size_bytes: meta.len(),
            modified_ns,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn write_markdown_file(
    rel_path: String,
    content: String,
    disk_baseline_sha256: Option<String>,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    let last_self_write = Arc::clone(&state.last_self_write);
    let re_root = canonical_root.clone();
    let re_rel = rel_path.clone();
    let app_re = app_handle.clone();
    let promotions = tauri::async_runtime::spawn_blocking({
        let rel_path_for_key = rel_path.clone();
        move || {
            let inner = (|| -> Result<Vec<thought_parser::ThoughtMaturityChangedPayload>, String> {
                let joined = join_under_root(&canonical_root, &rel_path)?;

                let mut old_content: Option<String> = None;

                if joined.exists() {
                    // Existing file: canonicalize to resolve symlinks and verify safety
                    let canonical_file = fs::canonicalize(&joined)
                        .map_err(|e| sanitize_io_error(e, "resolving file path"))?;
                    if !canonical_file.starts_with(&canonical_root) {
                        return Err("path escapes root".to_string());
                    }
                    if !is_markdown_path(&canonical_file) {
                        return Err("not a markdown file".to_string());
                    }
                    let disk_bytes =
                        fs::read(&canonical_file).map_err(|e| sanitize_io_error(e, "reading file"))?;
                    let actual_hash = sha256_hex_bytes(&disk_bytes);
                    let baseline_raw = disk_baseline_sha256.as_deref().map(str::trim).filter(|s| !s.is_empty());
                    let expected_hash = baseline_raw
                        .ok_or_else(|| {
                            "DISK_BASELINE_REQUIRED: diskBaselineSha256 is required when overwriting an existing file"
                                .to_string()
                        })
                        .and_then(normalize_client_sha256_hex)?;
                    if actual_hash != expected_hash {
                        return Err(
                            "DISK_CONFLICT: file changed on disk before save; reload the note and merge if needed"
                                .to_string(),
                        );
                    }
                    let old_s = String::from_utf8(disk_bytes)
                        .map_err(|_| "DISK_READ_UTF8: file is not valid UTF-8".to_string())?;
                    old_content = Some(old_s);
                    fs::write(&canonical_file, &content)
                        .map_err(|e| sanitize_io_error(e, "writing file"))?;
                } else {
                    // New file: canonicalize parent to prevent symlink escape
                    if !is_markdown_path(&joined) {
                        return Err("not a markdown file".to_string());
                    }
                    if let Some(parent) = joined.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| sanitize_io_error(e, "creating parent directories"))?;
                        let canonical_parent = fs::canonicalize(parent)
                            .map_err(|e| sanitize_io_error(e, "resolving parent directory path"))?;
                        if !canonical_parent.starts_with(&canonical_root) {
                            return Err("path escapes root".to_string());
                        }
                        let file_name = joined
                            .file_name()
                            .ok_or_else(|| "invalid file name".to_string())?;
                        let canonical_file = canonical_parent.join(file_name);
                        fs::write(&canonical_file, &content)
                            .map_err(|e| sanitize_io_error(e, "writing file"))?;
                    } else {
                        return Err("invalid path".to_string());
                    }
                }

                let promos = if let Some(old) = old_content {
                    thought_parser::detect_thought_maturity_promotions(&rel_path, &old, &content)
                } else {
                    Vec::new()
                };
                Ok(promos)
            })();

            if inner.is_ok() {
                let mut lw = match last_self_write.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                *lw = Some((rel_path_for_key, Instant::now()));
            }

            inner
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    tauri::async_runtime::spawn(async move {
        let _ = tauri::async_runtime::spawn_blocking(move || {
            semantic_index::incremental_reindex_note(&re_root, &app_re, &re_rel)
        })
        .await;
    });

    for p in promotions {
        let _ = app_handle.emit("thought-maturity-changed", p);
    }
    Ok(())
}

/// 在 `parent_rel` 下创建单层文件夹；`parent_rel` 为空表示工作区根
#[tauri::command]
async fn create_workspace_folder(
    parent_rel: String,
    folder_name: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let parent_rel = parent_rel.trim();
    let folder_name = folder_name.trim();
    if folder_name.is_empty() {
        return Err("empty folder name".to_string());
    }
    if folder_name.contains('/') || folder_name.contains('\\') {
        return Err("invalid folder name".to_string());
    }
    if folder_name == "." || folder_name == ".." {
        return Err("invalid folder name".to_string());
    }
    let rel_path = if parent_rel.is_empty() {
        folder_name.to_string()
    } else {
        rel_path_components_ok(parent_rel)?;
        format!("{}/{}", parent_rel.trim_end_matches('/'), folder_name)
    };
    rel_path_components_ok(&rel_path)?;

    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        if joined.exists() {
            return Err("File already exists".to_string());
        }
        let parent = joined
            .parent()
            .ok_or_else(|| "invalid path".to_string())?;
        fs::create_dir_all(parent).map_err(|e| sanitize_io_error(e, "creating parent directories"))?;
        let canon_parent =
            fs::canonicalize(parent).map_err(|e| sanitize_io_error(e, "resolving parent directory path"))?;
        if !canon_parent.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        fs::create_dir(&joined).map_err(|e| sanitize_io_error(e, "creating folder"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 递归检查目录树下是否存在普通文件（不含目录）；存在文件或符号链接则 Err
fn assert_dir_tree_has_no_files(dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| sanitize_io_error(e, "listing workspace directory"))?;
    for entry in entries {
        let entry = entry.map_err(|e| sanitize_io_error(e, "reading directory entry"))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
        if meta.file_type().is_symlink() {
            return Err("Folder contains a symlink; remove it before deleting".to_string());
        }
        if meta.is_file() {
            return Err("Folder contains files; delete or move them first".to_string());
        }
        if meta.is_dir() {
            assert_dir_tree_has_no_files(&path)?;
        }
    }
    Ok(())
}

/// 删除空目录树（无普通文件）；`rel_path` 为相对工作区根的目录路径
fn delete_workspace_folder_sync(canonical_root: &Path, rel_path: &str) -> Result<(), String> {
    rel_path_components_ok(rel_path)?;
    let joined = join_under_root(canonical_root, rel_path)?;
    let meta = fs::symlink_metadata(&joined).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
    if meta.file_type().is_symlink() {
        return Err("cannot delete symlink".to_string());
    }
    if !meta.is_dir() {
        return Err("not a directory".to_string());
    }
    let canon = fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving folder path"))?;
    if !canon.starts_with(canonical_root) {
        return Err("path escapes root".to_string());
    }
    assert_dir_tree_has_no_files(&canon)?;
    fs::remove_dir_all(&canon).map_err(|e| sanitize_io_error(e, "deleting folder"))
}

/// 删除不含文件的文件夹（子目录可为空目录树）
#[tauri::command]
async fn delete_workspace_folder(rel_path: String, state: tauri::State<'_, WorkspaceState>) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || delete_workspace_folder_sync(&canonical_root, &rel_path))
        .await
        .map_err(|e| e.to_string())?
}

/// 在工作区内移动/重命名 Markdown 文件（`to_rel` 的父目录不存在时会创建）
fn move_workspace_path_sync(
    canonical_root: &Path,
    from_rel: &str,
    to_rel: &str,
) -> Result<(), String> {
    rel_path_components_ok(from_rel)?;
    rel_path_components_ok(to_rel)?;
    if from_rel == to_rel {
        return Ok(());
    }

    let from_joined = join_under_root(canonical_root, from_rel)?;
    let to_joined = join_under_root(canonical_root, to_rel)?;

    let from_meta = fs::symlink_metadata(&from_joined).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
    if from_meta.file_type().is_symlink() {
        return Err("cannot move symlink".to_string());
    }
    if !from_meta.is_file() {
        return Err("not a file".to_string());
    }
    if !is_markdown_path(&from_joined) || !is_markdown_path(&to_joined) {
        return Err("not a markdown file".to_string());
    }
    if to_joined.exists() {
        return Err("File already exists".to_string());
    }

    let from_canon =
        fs::canonicalize(&from_joined).map_err(|e| sanitize_io_error(e, "resolving source file path"))?;
    if !from_canon.starts_with(canonical_root) {
        return Err("path escapes root".to_string());
    }

    if let Some(parent) = to_joined.parent() {
        fs::create_dir_all(parent).map_err(|e| sanitize_io_error(e, "creating parent directories"))?;
        let canon_parent =
            fs::canonicalize(parent).map_err(|e| sanitize_io_error(e, "resolving parent directory path"))?;
        if !canon_parent.starts_with(canonical_root) {
            return Err("path escapes root".to_string());
        }
    }

    fs::rename(&from_joined, &to_joined).map_err(|e| sanitize_io_error(e, "moving file"))?;
    let _ = thought_reconcile::refresh_note_rel_path_after_file_move(canonical_root, to_rel);
    Ok(())
}

/// 在工作区内重命名目录（非符号链接）；`to_rel` 不存在时 `rename` 整棵子树
fn rename_workspace_folder_sync(canonical_root: &Path, from_rel: &str, to_rel: &str) -> Result<(), String> {
    rel_path_components_ok(from_rel)?;
    rel_path_components_ok(to_rel)?;
    if from_rel == to_rel {
        return Ok(());
    }

    let from_joined = join_under_root(canonical_root, from_rel)?;
    let to_joined = join_under_root(canonical_root, to_rel)?;

    let from_meta = fs::symlink_metadata(&from_joined).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
    if from_meta.file_type().is_symlink() {
        return Err("cannot rename symlink".to_string());
    }
    if !from_meta.is_dir() {
        return Err("not a directory".to_string());
    }
    if to_joined.exists() {
        return Err("File already exists".to_string());
    }

    let from_canon =
        fs::canonicalize(&from_joined).map_err(|e| sanitize_io_error(e, "resolving source folder path"))?;
    if !from_canon.starts_with(canonical_root) {
        return Err("path escapes root".to_string());
    }

    if let Some(parent) = to_joined.parent() {
        fs::create_dir_all(parent).map_err(|e| sanitize_io_error(e, "creating parent directories"))?;
        let canon_parent =
            fs::canonicalize(parent).map_err(|e| sanitize_io_error(e, "resolving parent directory path"))?;
        if !canon_parent.starts_with(canonical_root) {
            return Err("path escapes root".to_string());
        }
    }

    fs::rename(&from_joined, &to_joined).map_err(|e| sanitize_io_error(e, "renaming folder"))
}

#[tauri::command]
async fn rename_workspace_folder(
    from_rel: String,
    to_rel: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let from_rel = from_rel.trim().to_string();
    let to_rel = to_rel.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || rename_workspace_folder_sync(&canonical_root, &from_rel, &to_rel))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn move_workspace_entry(
    from_rel: String,
    to_rel: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let from_rel = from_rel.trim().to_string();
    let to_rel = to_rel.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        move_workspace_path_sync(&canonical_root, &from_rel, &to_rel)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_markdown_file(
    rel_path: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        let meta = fs::symlink_metadata(&joined).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
        if meta.file_type().is_symlink() {
            return Err("cannot delete symlink".to_string());
        }
        if !meta.is_file() {
            return Err("not a file".to_string());
        }
        if !is_markdown_path(&joined) {
            return Err("not a markdown file".to_string());
        }
        let canon = fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canon.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        fs::remove_file(&joined).map_err(|e| sanitize_io_error(e, "deleting file"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 为单个相对路径创建 debounced watcher；失败时返回 Err（调用方可跳过该路径）
fn spawn_markdown_file_watcher(
    rel_emit: String,
    canonical_root: &Path,
    app: AppHandle,
    last_arc: Arc<Mutex<Option<(String, Instant)>>>,
) -> Result<Debouncer<RecommendedWatcher>, String> {
    let joined = join_under_root(canonical_root, &rel_emit)?;
    let meta = fs::symlink_metadata(&joined).map_err(|e| sanitize_io_error(e, "reading file metadata"))?;
    if meta.file_type().is_symlink() {
        return Err("cannot watch symlink target".to_string());
    }
    if !meta.is_file() {
        return Err("not a file".to_string());
    }
    if !is_markdown_path(&joined) {
        return Err("not a markdown file".to_string());
    }
    let canonical_file =
        fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
    if !canonical_file.starts_with(canonical_root) {
        return Err("path escapes root".to_string());
    }

    let watch_target = canonical_file.clone();
    let rel_for_cb = rel_emit.clone();
    let app_h = app.clone();

    let mut debouncer = new_debouncer(Duration::from_millis(220), move |res: DebounceEventResult| {
        let Ok(events) = res else {
            return;
        };
        for e in events {
            if e.path != watch_target {
                continue;
            }
            let ignore = {
                let guard = match last_arc.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some((ref p, t)) = *guard {
                    p == &rel_for_cb && t.elapsed() < Duration::from_millis(1600)
                } else {
                    false
                }
            };
            if ignore {
                continue;
            }
            let payload = MarkdownDiskChangedPayload {
                rel_path: rel_for_cb.clone(),
            };
            let _ = app_h.emit("markdown-disk-changed", payload);
        }
    })
    .map_err(|e| e.to_string())?;

    debouncer
        .watcher()
        .watch(&canonical_file, RecursiveMode::NonRecursive)
        .map_err(|e| e.to_string())?;

    Ok(debouncer)
}

/// 与当前已打开 Tab 列表对齐：多文件监听；`rel_paths` 为空时全部停止
#[tauri::command]
fn sync_open_markdown_watchers(
    rel_paths: Vec<String>,
    app: AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let wanted: HashSet<String> = rel_paths
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if wanted.is_empty() {
        let mut guard = match state.markdown_watchers.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                eprintln!("[workspace] markdown_watchers mutex was poisoned, recovering");
                poisoned.into_inner()
            }
        };
        guard.clear();
        return Ok(());
    }

    let canonical_root = lock_workspace_root(&state)?;
    let last_arc = Arc::clone(&state.last_self_write);

    let mut guard = match state.markdown_watchers.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            eprintln!("[workspace] markdown_watchers mutex was poisoned, recovering");
            poisoned.into_inner()
        }
    };

    let stale_keys: Vec<String> = guard
        .keys()
        .filter(|k| !wanted.contains(*k))
        .cloned()
        .collect();
    for k in stale_keys {
        guard.remove(&k);
    }

    for rel in wanted {
        if guard.contains_key(&rel) {
            continue;
        }
        match spawn_markdown_file_watcher(rel.clone(), &canonical_root, app.clone(), Arc::clone(&last_arc)) {
            Ok(d) => {
                guard.insert(rel, d);
            }
            Err(e) => {
                eprintln!("[workspace] skip watcher for {rel}: {e}");
            }
        }
    }

    Ok(())
}

#[tauri::command]
async fn get_vault_config_for_ui(state: tauri::State<'_, WorkspaceState>) -> Result<vault_config::VaultConfigForUi, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || vault_config::load_for_ui(&canonical_root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_vault_config_patch(
    patch: vault_config::VaultConfigPatch,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || vault_config::save_patch(&canonical_root, patch))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_ai_conversations(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::ListAiConversationsResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::list_ai_conversations_blocking(&canonical_root))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn create_ai_conversation(
    args: ai_conversations::CreateAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::CreateAiConversationResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::create_ai_conversation_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn load_ai_conversation(
    args: ai_conversations::LoadAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::ConversationBodyOut, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::load_ai_conversation_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_ai_conversation(
    args: ai_conversations::SaveAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::save_ai_conversation_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_active_ai_conversation(
    args: ai_conversations::SetActiveAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::set_active_ai_conversation_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_ai_conversation(
    args: ai_conversations::DeleteAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || ai_conversations::delete_ai_conversation_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_thought_mgmt_ai_conversations(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::ListAiConversationsResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::list_thought_mgmt_ai_conversations_blocking(&canonical_root)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn create_thought_mgmt_ai_conversation(
    args: ai_conversations::CreateAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::CreateAiConversationResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::create_thought_mgmt_ai_conversation_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn load_thought_mgmt_ai_conversation(
    args: ai_conversations::LoadAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<ai_conversations::ConversationBodyOut, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::load_thought_mgmt_ai_conversation_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_thought_mgmt_ai_conversation(
    args: ai_conversations::SaveAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::save_thought_mgmt_ai_conversation_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn set_active_thought_mgmt_ai_conversation(
    args: ai_conversations::SetActiveAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::set_active_thought_mgmt_ai_conversation_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_thought_mgmt_ai_conversation(
    args: ai_conversations::DeleteAiConversationArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai_conversations::delete_thought_mgmt_ai_conversation_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn search_workspace_context(
    args: vault_context_search::SearchWorkspaceContextArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<vault_context_search::SearchWorkspaceContextResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || vault_context_search::search_workspace_context_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn parse_note_thoughts(
    rel_path: String,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<thought_parser::ParseNoteThoughtsResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        let canonical_file =
            fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        if !is_markdown_path(&canonical_file) {
            return Err("not a markdown file".to_string());
        }
        let content =
            fs::read_to_string(&canonical_file).map_err(|e| sanitize_io_error(e, "reading file"))?;
        Ok(thought_parser::parse_note_thoughts_for_workspace(
            &canonical_root,
            &rel_path,
            &content,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn insert_thought_to_note(
    args: thought_parser::InsertThoughtArgs,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<thought_parser::InsertThoughtResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = args.rel_path.trim().to_string();
    let content = args.content;
    let temporary = args.temporary;
    let after_line = args.after_line;

    let rel_for_emit = rel_path.clone();
    let resp = tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        if !is_markdown_path(&joined) {
            return Err("not a markdown file".to_string());
        }
        let existing = if joined.exists() {
            let canonical_file = fs::canonicalize(&joined)
                .map_err(|e| sanitize_io_error(e, "resolving file path"))?;
            if !canonical_file.starts_with(&canonical_root) {
                return Err("path escapes root".to_string());
            }
            fs::read_to_string(&canonical_file)
                .map_err(|e| sanitize_io_error(e, "reading file"))?
        } else {
            String::new()
        };
        let existing_resp = thought_parser::parse_note_thoughts_for_workspace(
            &canonical_root,
            &rel_path,
            &existing,
        );
        let count = existing_resp.meta.len().max(existing_resp.blocks.len());
        let (new_markdown, resp) = thought_parser::insert_thought_into_markdown(
            &canonical_root,
            &rel_path,
            &existing,
            &content,
            temporary,
            after_line,
            count,
        )?;
        if joined.exists() {
            let canonical_file = fs::canonicalize(&joined)
                .map_err(|e| sanitize_io_error(e, "resolving file path"))?;
            fs::write(&canonical_file, &new_markdown)
                .map_err(|e| sanitize_io_error(e, "writing file"))?;
        } else if let Some(parent) = joined.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| sanitize_io_error(e, "creating parent directories"))?;
            let canonical_parent = fs::canonicalize(parent)
                .map_err(|e| sanitize_io_error(e, "resolving parent directory path"))?;
            if !canonical_parent.starts_with(&canonical_root) {
                return Err("path escapes root".to_string());
            }
            let file_name = joined.file_name().ok_or_else(|| "invalid file name".to_string())?;
            let canonical_file = canonical_parent.join(file_name);
            fs::write(&canonical_file, &new_markdown)
                .map_err(|e| sanitize_io_error(e, "writing file"))?;
        } else {
            return Err("invalid path".to_string());
        }
        Ok(resp)
    })
    .await
    .map_err(|e| e.to_string())??;

    // 通知前端编辑器刷新文件内容（不设置 last_self_write，因为这不是编辑器自身的保存）
    let _ = app_handle.emit(
        "markdown-disk-changed",
        MarkdownDiskChangedPayload { rel_path: rel_for_emit },
    );

    Ok(resp)
}

#[tauri::command]
async fn apply_challenge_pass_to_thought(
    args: challenge_review::ApplyChallengePassArgs,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_for_emit = args.rel_path.trim().to_string();
    let maturity_change = tauri::async_runtime::spawn_blocking(move || {
        challenge_review::apply_challenge_pass_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = app_handle.emit(
        "markdown-disk-changed",
        MarkdownDiskChangedPayload {
            rel_path: rel_for_emit.clone(),
        },
    );

    if let Some(core) = maturity_change {
        let payload = thought_parser::ThoughtMaturityChangedPayload {
            rel_path: rel_for_emit.clone(),
            thought_id: core.thought_id,
            from_maturity: core.from_maturity,
            to_maturity: core.to_maturity,
            start_line: core.start_line,
        };
        let _ = app_handle.emit("thought-maturity-changed", payload);
    }
    Ok(())
}

#[tauri::command]
async fn append_ai_thought_reference(
    args: thought_parser::AppendAiThoughtReferenceArgs,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = args.rel_path.trim().to_string();
    let thought_id = args.thought_id.trim().to_string();
    let context = args.context;
    let relevance = args.relevance;
    let rel_for_emit = rel_path.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let joined = join_under_root(&canonical_root, &rel_path)?;
        if !is_markdown_path(&joined) {
            return Err("not a markdown file".to_string());
        }
        let canonical_file =
            fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        let existing = fs::read_to_string(&canonical_file)
            .map_err(|e| sanitize_io_error(e, "reading file"))?;
        let new_md = thought_parser::append_ai_thought_reference_to_markdown(
            &existing,
            &thought_id,
            &context,
            &relevance,
        )?;
        if new_md != existing {
            fs::write(&canonical_file, new_md).map_err(|e| sanitize_io_error(e, "writing file"))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;

    let _ = app_handle.emit(
        "markdown-disk-changed",
        MarkdownDiskChangedPayload {
            rel_path: rel_for_emit,
        },
    );
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListVaultThoughtsArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default = "default_vault_thought_list_limit")]
    limit: usize,
    /// 分页偏移，与 `limit` 配合；默认 0
    #[serde(default)]
    offset: usize,
    /// `all` | `standalone` | `linked` | `temporary`
    #[serde(default)]
    filter: Option<String>,
}

fn default_vault_thought_list_limit() -> usize {
    400
}

#[tauri::command]
async fn list_vault_thoughts(
    args: ListVaultThoughtsArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<vault_thoughts_db::VaultThoughtListPage, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let q = args.query.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let limit = args.limit;
    let offset = args.offset;
    let filt = args
        .filter
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    tauri::async_runtime::spawn_blocking(move || {
        let conn = vault_thoughts_db::open_thoughts_db(&canonical_root)?;
        vault_thoughts_db::list_vault_thought_rows_paged(
            &conn,
            limit,
            offset,
            q.as_deref(),
            filt.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateStandaloneThoughtArgs {
    body: String,
    #[serde(default)]
    summary: Option<String>,
}

#[tauri::command]
async fn create_standalone_thought(
    args: CreateStandaloneThoughtArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<String, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let conn = vault_thoughts_db::open_thoughts_db(&canonical_root)?;
        vault_thoughts_db::create_standalone_thought(&conn, args.body.trim(), args.summary.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateThoughtBodyArgs {
    thought_id: String,
    body: String,
    #[serde(default)]
    summary: Option<String>,
}

#[tauri::command]
async fn update_thought_body(
    args: UpdateThoughtBodyArgs,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let canonical_root = lock_workspace_root(&state)?;
    let tid = args.thought_id.trim().to_string();
    if tid.is_empty() {
        return Err("thought_id is empty".to_string());
    }
    let body = args.body;
    let summary = args.summary;
    let rel_emit = tauri::async_runtime::spawn_blocking(move || {
        let conn = vault_thoughts_db::open_thoughts_db(&canonical_root)?;
        let detail = vault_thoughts_db::get_thought_detail(&conn, &tid)?
            .ok_or_else(|| "找不到该想法".to_string())?;
        vault_thoughts_db::update_thought_body_by_id(&conn, &tid, &body, summary.as_deref())?;
        if detail.standalone {
            return Ok::<Option<String>, String>(None);
        }
        let rel_path = detail.note_rel_path.clone();
        let joined = join_under_root(&canonical_root, &rel_path)?;
        if !is_markdown_path(&joined) {
            return Err("not a markdown file".to_string());
        }
        let canonical_file = fs::canonicalize(&joined)
            .map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        let existing = fs::read_to_string(&canonical_file)
            .map_err(|e| sanitize_io_error(e, "reading file"))?;
        let new_md = thought_parser::bump_kf_thought_updated_in_markdown(&existing, &tid)?;
        if new_md != existing {
            fs::write(&canonical_file, &new_md).map_err(|e| sanitize_io_error(e, "writing file"))?;
        }
        Ok(Some(rel_path))
    })
    .await
    .map_err(|e| e.to_string())??;

    if let Some(rel_path) = rel_emit {
        let _ = app_handle.emit(
            "markdown-disk-changed",
            MarkdownDiskChangedPayload { rel_path },
        );
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteThoughtResponse {
    deleted: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    orphan_callout_may_remain: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteThoughtArgs {
    thought_id: String,
}

#[tauri::command]
async fn delete_thought(
    args: DeleteThoughtArgs,
    state: tauri::State<'_, WorkspaceState>,
    app_handle: tauri::AppHandle,
) -> Result<DeleteThoughtResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let tid = args.thought_id.trim().to_string();
    if tid.is_empty() {
        return Err("thought_id is empty".to_string());
    }
    let (rel_emit, row_deleted, orphan) = tauri::async_runtime::spawn_blocking(move || {
        let conn = vault_thoughts_db::open_thoughts_db(&canonical_root)?;
        let detail = match vault_thoughts_db::get_thought_detail(&conn, &tid)? {
            Some(d) => d,
            None => {
                return Ok::<(Option<String>, bool, bool), String>((None, false, false));
            }
        };
        if detail.standalone {
            let deleted = vault_thoughts_db::delete_thought_by_id(&conn, &tid)?;
            return Ok((None, deleted, false));
        }
        let rel_path = detail.note_rel_path.clone();
        let joined = join_under_root(&canonical_root, &rel_path)?;
        if !is_markdown_path(&joined) {
            return Err("not a markdown file".to_string());
        }
        let canonical_file = fs::canonicalize(&joined)
            .map_err(|e| sanitize_io_error(e, "resolving file path"))?;
        if !canonical_file.starts_with(&canonical_root) {
            return Err("path escapes root".to_string());
        }
        let existing = fs::read_to_string(&canonical_file)
            .map_err(|e| sanitize_io_error(e, "reading file"))?;
        let outcome = thought_parser::remove_thought_from_markdown(&existing, &tid)?;
        fs::write(&canonical_file, &outcome.markdown)
            .map_err(|e| sanitize_io_error(e, "writing file"))?;
        let deleted = vault_thoughts_db::delete_thought_by_id(&conn, &tid)?;
        Ok((
            Some(rel_path),
            deleted,
            outcome.orphan_callout_may_remain,
        ))
    })
    .await
    .map_err(|e| e.to_string())??;

    if let Some(rel_path) = rel_emit {
        let _ = app_handle.emit(
            "markdown-disk-changed",
            MarkdownDiskChangedPayload { rel_path },
        );
    }
    Ok(DeleteThoughtResponse {
        deleted: row_deleted,
        orphan_callout_may_remain: orphan,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetThoughtDetailArgs {
    thought_id: String,
}

#[tauri::command]
async fn get_thought_detail(
    args: GetThoughtDetailArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Option<vault_thoughts_db::ThoughtDetail>, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let tid = args.thought_id.trim().to_string();
    if tid.is_empty() {
        return Err("thought_id is empty".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = vault_thoughts_db::open_thoughts_db(&canonical_root)?;
        vault_thoughts_db::get_thought_detail(&conn, &tid)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn search_thought_for_invite(
    args: thought_retrieval::SearchThoughtArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<thought_retrieval::SearchThoughtResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        thought_retrieval::search_thought_blocking(&canonical_root, args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn search_workspace_text(
    args: workspace_text_search::SearchWorkspaceTextArgs,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<workspace_text_search::WorkspaceTextSearchResponse, String> {
    let canonical_root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || workspace_text_search::search_workspace_text_blocking(&canonical_root, args))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_depth_decisions(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<depth_decisions::DepthDecisionEntry>, String> {
    let canonical_root = lock_workspace_root(&state)?;
    Ok(depth_decisions::read_recent_decisions(&canonical_root, 50))
}

#[tauri::command]
async fn rebuild_embeddings(
    resume: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<semantic_index::IndexBuildResult, String> {
    let root = lock_workspace_root(&state)?;
    let resume = resume.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || semantic_index::rebuild_index(&root, &app, resume))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_embedding_rebuild_progress(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Option<rebuild_progress::RebuildProgress>, String> {
    let root = lock_workspace_root(&state)?;
    Ok(semantic_index::read_embedding_rebuild_progress(&root))
}

#[tauri::command]
async fn get_embedding_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<semantic_index::IndexStatus, String> {
    let root = lock_workspace_root(&state)?;
    let cache = semantic_index::default_model_cache_dir();
    let bundle = semantic_index::resolve_bundle_model_dir(&app);
    tauri::async_runtime::spawn_blocking(move || semantic_index::get_index_status(&root, &cache, &bundle))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn semantic_search(
    args: semantic_index::SemanticSearchArgs,
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<semantic_index::SemanticSearchHit>, String> {
    let root = lock_workspace_root(&state)?;
    let cache = semantic_index::default_model_cache_dir();
    let bundle = semantic_index::resolve_bundle_model_dir(&app);
    tauri::async_runtime::spawn_blocking(move || semantic_index::run_semantic_search(&root, &cache, &bundle, args))
        .await
        .map_err(|e| e.to_string())?
}

/// 语义链接推荐：无有效文档向量索引时由 `link_recommendation::suggest_related_notes` 返回明确错误（无关键词兜底）。
#[tauri::command]
async fn suggest_related_notes(
    rel_path: String,
    max_results: Option<usize>,
    include_reasons: Option<bool>,
    editor_markdown_override: Option<String>,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<Vec<link_recommendation::LinkRecommendation>, String> {
    let canonical_root = lock_workspace_root(&state)?;
    let rel_path = rel_path.trim().to_string();
    if rel_path.is_empty() {
        return Err("rel_path is empty".to_string());
    }
    let max_results = max_results.unwrap_or(5).min(50).max(1);
    let include_reasons = include_reasons.unwrap_or(false);

    let root = canonical_root.clone();
    let rel = rel_path.clone();
    let md_override = editor_markdown_override.clone();
    let mut out = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<link_recommendation::LinkRecommendation>, String> {
        let emb = semantic_index::open_embedding_db(&root)?;
        let thoughts = vault_thoughts_db::open_thoughts_db(&root)?;
        link_recommendation::suggest_related_notes(
            &root,
            &rel,
            &emb,
            &thoughts,
            max_results,
            md_override.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    if include_reasons && !out.is_empty() {
        let ai = vault_config::load_ai_config_internal(&canonical_root)?;
        let root2 = canonical_root.clone();
        let rel2 = rel_path.clone();
        let excerpt = tauri::async_runtime::spawn_blocking(move || {
            link_recommendation::load_note_excerpt_for_reasons(&root2, &rel2)
        })
        .await
        .map_err(|e| e.to_string())??;
        link_recommendation::enrich_recommendations_with_reasons(&mut out, &excerpt, &ai).await?;
    }

    Ok(out)
}

#[tauri::command]
async fn build_topic_network(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<topic_network::TopicNetworkForUi, String> {
    let root = lock_workspace_root(&state)?;
    topic_network::build_topic_network(&root, &app).await
}

#[tauri::command]
async fn get_topic_cache_status(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<topic_network::TopicCacheStatus, String> {
    let root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let conn = topic_network::open_topic_db(&root)?;
        topic_network::get_topic_cache_status(&conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_topic_index_markdown(
    state: tauri::State<'_, WorkspaceState>,
) -> Result<topic_network::TopicMarkdownExportSummary, String> {
    let root = lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let conn = topic_network::open_topic_db(&root)?;
        topic_network::export_topic_index_markdown(&root, &conn)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_manual_topic_semantic(
    display_name: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceState>,
) -> Result<topic_network::AddManualTopicResult, String> {
    let root = lock_workspace_root(&state)?;
    let name = display_name.trim().to_string();
    if name.is_empty() {
        return Err("display_name is empty".to_string());
    }
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || topic_network::add_manual_topic_semantic_blocking(&root, &app2, &name))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(WorkspaceState::default())
        .manage(Arc::new(llm::LlmSessionState::default()))
        .manage(Arc::new(llm::approval::ToolApprovalState::new()))
        .manage(Arc::new(tools::ToolRegistry::new()))
        .manage(Arc::new(skills::SkillRegistry::new()))
        .manage(Arc::new(tokio::sync::Semaphore::new(skills::SKILL_CONCURRENCY)))
        .manage({
            let audit_sink: Arc<dyn tools::context::AuditSink> = Arc::new(
                tools::audit::NullAuditSink
            );
            let privacy_filter: Arc<dyn tools::context::PrivacyFilter> = Arc::new(
                tools::privacy::KfPrivateFilter
            );
            Arc::new(tools::ToolContextFactory::new(audit_sink, privacy_filter))
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            open_workspace,
            refresh_md_tree,
            read_markdown_file,
            get_markdown_file_signature,
            write_markdown_file,
            move_workspace_entry,
            delete_markdown_file,
            sync_open_markdown_watchers,
            create_workspace_folder,
            delete_workspace_folder,
            rename_workspace_folder,
            get_vault_config_for_ui,
            save_vault_config_patch,
            llm::list_ollama_models,
            llm::start_ollama_chat_stream,
            llm::abort_llm_stream,
            llm::respond_tool_approval,
            llm::clear_conversation_approvals,
            list_ai_conversations,
            create_ai_conversation,
            load_ai_conversation,
            save_ai_conversation,
            set_active_ai_conversation,
            delete_ai_conversation,
            list_thought_mgmt_ai_conversations,
            create_thought_mgmt_ai_conversation,
            load_thought_mgmt_ai_conversation,
            save_thought_mgmt_ai_conversation,
            set_active_thought_mgmt_ai_conversation,
            delete_thought_mgmt_ai_conversation,
            search_workspace_context,
            search_workspace_text,
            parse_note_thoughts,
            list_vault_thoughts,
            create_standalone_thought,
            update_thought_body,
            delete_thought,
            get_thought_detail,
            insert_thought_to_note,
            apply_challenge_pass_to_thought,
            append_ai_thought_reference,
            cognitive_report::generate_cognitive_report,
            understanding_graph::scan_understanding_graph,
            challenge_review::generate_challenge_question,
            challenge_review::evaluate_challenge_answer,
            challenge_review::list_review_queue,
            challenge_review::count_vault_thoughts_for_review,
            search_thought_for_invite,
            list_depth_decisions,
            passive_highlight::detect_passive_highlight,
            passive_highlight::increment_passive_highlight_inaccuracy,
            writing_coach::analyze_writing_coach,
            knowforge_analytics::append_knowforge_analytics,
            rebuild_embeddings,
            get_embedding_rebuild_progress,
            get_embedding_status,
            semantic_search,
            suggest_related_notes,
            build_topic_network,
            get_topic_cache_status,
            export_topic_index_markdown,
            add_manual_topic_semantic,
            tools::commands::list_tools,
            tools::commands::invoke_tool,
            skills::commands::list_skills,
            skills::commands::invoke_skill
        ])
        .setup(|app| {
            use tauri::Manager;
            let registry = app.state::<Arc<tools::ToolRegistry>>();
            tools::register_builtin_tools(&registry).expect("failed to register builtin tools");
            let skill_registry = app.state::<Arc<skills::SkillRegistry>>();
            skills::register_builtin_skills(&skill_registry, &registry)
                .expect("failed to register builtin skills");
            // Iter 5 #4: register `skill.<id>` tool wrappers AFTER skills + tools
            // are populated, so the main agent loop can auto-invoke them.
            let semaphore = app.state::<Arc<tokio::sync::Semaphore>>();
            skills::register_skill_tools(
                &app.handle().clone(),
                &skill_registry,
                &registry,
                Arc::clone(&semaphore),
            )
            .expect("failed to register skill tool wrappers");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
