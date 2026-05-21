use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolError, ToolErrorCode, ToolManifest, ToolMetrics,
    ToolResult,
};

// ─── 辅助函数：递归收集 .md 文件相对路径 ────────────────────────────────────────

fn collect_md_files(dir: &Path, root: &Path, result: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        // 跳过符号链接
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if path.is_dir() {
            // 跳过以 . 开头的隐藏目录
            if !entry
                .file_name()
                .to_string_lossy()
                .starts_with('.')
            {
                collect_md_files(&path, root, result)?;
            }
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            if let Ok(rel) = path.strip_prefix(root) {
                result.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

// ─── NoteListTool ──────────────────────────────────────────────────────────────

pub struct NoteListTool {
    manifest: ToolManifest,
}

impl NoteListTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.list".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "列出工作区内所有 Markdown 笔记文件的相对路径".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "include_private": { "type": "boolean" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" } },
                        "total": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["note".to_string(), "list".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteListTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();
        let include_private = input
            .get("include_private")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let root = ctx.workspace_root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
            let mut all_paths = Vec::new();
            collect_md_files(&root, &root, &mut all_paths)
                .map_err(|e| format!("failed to list notes: {e}"))?;

            if include_private {
                Ok(all_paths)
            } else {
                let filtered: Vec<String> = all_paths
                    .into_iter()
                    .filter(|rel| {
                        let full_path = root.join(rel);
                        !crate::note_privacy::peek_kf_private_from_md_file(&full_path)
                    })
                    .collect();
                Ok(filtered)
            }
        })
        .await;

        let paths = match result {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e,
                        retryable: true,
                        cause: None,
                    },
                }
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e.to_string(),
                        retryable: true,
                        cause: None,
                    },
                }
            }
        };

        let total = paths.len();
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: serde_json::json!({ "paths": paths, "total": total }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}

// ─── NoteReadTool ──────────────────────────────────────────────────────────────

pub struct NoteReadTool {
    manifest: ToolManifest,
}

impl NoteReadTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "note.read".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "读取指定 Markdown 笔记的完整内容".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path"],
                    "properties": {
                        "rel_path": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rel_path": { "type": "string" },
                        "content": { "type": "string" },
                        "size_bytes": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["note".to_string(), "read".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for NoteReadTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "rel_path is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        // 路径安全性校验
        if let Err(e) = crate::note_privacy::validate_workspace_rel_path(&rel_path) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: e,
                    retryable: false,
                    cause: None,
                },
            };
        }

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();

        let result =
            tauri::async_runtime::spawn_blocking(move || -> Result<(String, usize), String> {
                let full_path = root.join(&rel);

                // 确认文件存在
                if !full_path.exists() {
                    return Err("note not found".to_string());
                }

                // 检查是否私密
                if crate::note_privacy::peek_kf_private_from_md_file(&full_path) {
                    return Err("__PRIVACY_BLOCKED__".to_string());
                }

                let content = std::fs::read_to_string(&full_path)
                    .map_err(|e| format!("failed to read file: {e}"))?;
                let size_bytes = content.len();
                Ok((content, size_bytes))
            })
            .await;

        let (content, size_bytes) = match result {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) if e == "__PRIVACY_BLOCKED__" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::PrivacyBlocked,
                        message: "note is marked as private".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) if e == "note not found" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NotFound,
                        message: format!("note not found: {rel_path}"),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e,
                        retryable: true,
                        cause: None,
                    },
                }
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e.to_string(),
                        retryable: true,
                        cause: None,
                    },
                }
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: serde_json::json!({
                "rel_path": rel_path,
                "content": content,
                "size_bytes": size_bytes
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}
