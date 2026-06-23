//! 用户态 Vault 全文关键词检索：多文件、多命中、行号与预览；与 AI 用的 `search_workspace_context` 分离。

use crate::note_privacy;
use crate::vault_config;
use crate::vault_context_search::{enumerate_nonoverlapping_ci_byte_ranges, enumerate_nonoverlapping_cs_byte_ranges, rel_path_from_root, walk_markdown_files};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_MAX_FILES: usize = 8000;
const DEFAULT_MAX_BYTES_PER_FILE: usize = 4 * 1024 * 1024;
/// 单文件参与匹配的最大字节数（仅读磁盘前缀），与 `max_bytes_per_file`（元数据跳过阈值）配合控制峰值内存
const DEFAULT_MAX_SCAN_BYTES_PER_FILE: usize = 512 * 1024;
const DEFAULT_MAX_HITS_PER_FILE: usize = 50;
const DEFAULT_MAX_TOTAL_HITS: usize = 500;
const DEFAULT_DEADLINE_MS: u64 = 15_000;
const PREVIEW_MAX_CHARS: usize = 240;

fn redact_private_snippets(ai: &vault_config::AiConfig) -> bool {
    ai.should_redact_private()
}

/// 1-based 行号与列号（列按 Unicode 标量计数）
fn line_col_1_based_at_byte(content: &str, byte_off: usize) -> (usize, usize) {
    let byte_off = byte_off.min(content.len());
    let prefix = &content[..byte_off];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let last_nl = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = content[last_nl..byte_off].chars().count() + 1;
    (line, col)
}

/// 命中行及其上下各一行，截断宽度
fn build_preview(content: &str, hit_line_1: usize) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let idx0 = hit_line_1.saturating_sub(1);
    let start = idx0.saturating_sub(1);
    let end = (idx0 + 2).min(lines.len());
    let slice = lines[start..end].join("\n");
    if slice.chars().count() <= PREVIEW_MAX_CHARS {
        slice
    } else {
        slice.chars().take(PREVIEW_MAX_CHARS).collect::<String>() + "…"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceTextArgs {
    pub query: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub max_files_to_scan: Option<usize>,
    #[serde(default)]
    pub max_bytes_per_file: Option<usize>,
    /// 每文件最多读入并参与关键词扫描的字节数（磁盘前缀）；小于文件大小时不扫描尾部
    #[serde(default)]
    pub max_scan_bytes_per_file: Option<usize>,
    #[serde(default)]
    pub max_hits_per_file: Option<usize>,
    #[serde(default)]
    pub max_total_hits: Option<usize>,
    #[serde(default)]
    pub max_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTextSearchHit {
    pub rel_path: String,
    pub line: usize,
    pub column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default)]
    pub private_omitted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTextSearchMeta {
    pub scanned_files: usize,
    pub hit_count: usize,
    pub truncated: bool,
    pub stopped_early_deadline: bool,
    pub stopped_early_max_hits: bool,
    pub skipped_large_files: usize,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub omitted_private_previews: usize,
    /// 实际字节长度大于扫描前缀上限的文件数（这些文件尾部未被检索）
    #[serde(default)]
    pub files_scanned_as_prefix_only: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTextSearchResponse {
    pub hits: Vec<WorkspaceTextSearchHit>,
    pub meta: WorkspaceTextSearchMeta,
}

pub fn search_workspace_text_blocking(
    canonical_root: &Path,
    args: SearchWorkspaceTextArgs,
) -> Result<WorkspaceTextSearchResponse, String> {
    let started = Instant::now();
    let needle = args.query.trim();
    if needle.is_empty() {
        return Ok(WorkspaceTextSearchResponse {
            hits: Vec::new(),
            meta: WorkspaceTextSearchMeta {
                scanned_files: 0,
                hit_count: 0,
                truncated: false,
                stopped_early_deadline: false,
                stopped_early_max_hits: false,
                skipped_large_files: 0,
                elapsed_ms: 0,
                omitted_private_previews: 0,
                files_scanned_as_prefix_only: 0,
            },
        });
    }

    let ai = vault_config::load_ai_config_internal(canonical_root)?;
    let redact_private = redact_private_snippets(&ai);

    let max_files = args.max_files_to_scan.unwrap_or(DEFAULT_MAX_FILES).max(1);
    let max_bytes = args.max_bytes_per_file.unwrap_or(DEFAULT_MAX_BYTES_PER_FILE).max(4096);
    let max_scan = args
        .max_scan_bytes_per_file
        .unwrap_or(DEFAULT_MAX_SCAN_BYTES_PER_FILE)
        .max(4096)
        .min(max_bytes);
    let max_per_file = args.max_hits_per_file.unwrap_or(DEFAULT_MAX_HITS_PER_FILE).max(1);
    let max_total = args.max_total_hits.unwrap_or(DEFAULT_MAX_TOTAL_HITS).max(1);
    let deadline_ms = args.max_duration_ms.unwrap_or(DEFAULT_DEADLINE_MS).max(500);

    let mut paths: Vec<PathBuf> = Vec::new();
    walk_markdown_files(canonical_root, canonical_root, &mut paths, max_files)?;
    let stopped_early_collection = paths.len() >= max_files;

    let mut hits: Vec<WorkspaceTextSearchHit> = Vec::new();
    let mut scanned = 0usize;
    let mut skipped_large = 0usize;
    let mut prefix_only_files = 0usize;
    let mut omitted_private_previews = 0usize;
    let mut stopped_deadline = false;
    let mut stopped_max_hits = false;

    'outer: for abs in paths {
        if started.elapsed().as_millis() as u64 >= deadline_ms {
            stopped_deadline = true;
            break;
        }
        scanned += 1;
        let Some(rel) = rel_path_from_root(canonical_root, &abs) else {
            continue;
        };
        let meta = fs::symlink_metadata(&abs).map_err(|e| crate::sanitize_io_error(e, "reading file metadata"))?;
        let file_len = meta.len() as usize;
        if file_len > max_bytes {
            skipped_large += 1;
            continue;
        }
        if file_len > max_scan {
            prefix_only_files += 1;
        }
        let read_cap = max_scan.min(file_len);
        let mut buf = Vec::new();
        if read_cap > 0 {
            let mut taken = fs::File::open(&abs)
                .map_err(|e| crate::sanitize_io_error(e, "opening markdown for text search"))?
                .take(read_cap as u64);
            taken
                .read_to_end(&mut buf)
                .map_err(|e| crate::sanitize_io_error(e, "reading markdown prefix for text search"))?;
        }
        truncate_utf8_prefix_in_place(&mut buf);
        let content: String = String::from_utf8_lossy(&buf).into_owned();

        let ranges = if args.case_sensitive {
            enumerate_nonoverlapping_cs_byte_ranges(&content, needle)
        } else {
            enumerate_nonoverlapping_ci_byte_ranges(&content, needle)
        };

        let is_private = note_privacy::peek_kf_private_from_md_file(&abs);
        let hide_preview = is_private && redact_private;

        let mut file_hits = 0usize;
        for (start_byte, _end_byte) in ranges {
            if started.elapsed().as_millis() as u64 >= deadline_ms {
                stopped_deadline = true;
                break 'outer;
            }
            if hits.len() >= max_total {
                stopped_max_hits = true;
                break 'outer;
            }
            if file_hits >= max_per_file {
                break;
            }
            let (line, col) = line_col_1_based_at_byte(&content, start_byte);
            let preview = if hide_preview {
                omitted_private_previews += 1;
                None
            } else {
                Some(build_preview(&content, line))
            };
            hits.push(WorkspaceTextSearchHit {
                rel_path: rel.clone(),
                line,
                column: col,
                preview,
                private_omitted: hide_preview,
            });
            file_hits += 1;
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let truncated = stopped_early_collection || stopped_deadline || stopped_max_hits;
    let hit_count = hits.len();

    Ok(WorkspaceTextSearchResponse {
        hits,
        meta: WorkspaceTextSearchMeta {
            scanned_files: scanned,
            hit_count,
            truncated,
            stopped_early_deadline: stopped_deadline,
            stopped_early_max_hits: stopped_max_hits,
            skipped_large_files: skipped_large,
            elapsed_ms,
            omitted_private_previews,
            files_scanned_as_prefix_only: prefix_only_files,
        },
    })
}

/// 丢弃末尾可能不完整的 UTF-8 码点，避免跨 `read_cap` 截断导致无意义匹配
fn truncate_utf8_prefix_in_place(buf: &mut Vec<u8>) {
    while !buf.is_empty() && std::str::from_utf8(buf.as_slice()).is_err() {
        buf.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_md(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn tmp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "workspace_text_search_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn finds_multiple_lines_and_case_insensitive() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        write_md(&root, "a.md", "Line one\nFooBar here\nend FOOBAR\n");

        let res = search_workspace_text_blocking(
            &root,
            SearchWorkspaceTextArgs {
                query: "foobar".to_string(),
                case_sensitive: false,
                max_files_to_scan: Some(100),
                max_bytes_per_file: Some(1_000_000),
                max_scan_bytes_per_file: None,
                max_hits_per_file: Some(20),
                max_total_hits: Some(100),
                max_duration_ms: Some(10_000),
            },
        )
        .unwrap();
        assert_eq!(res.hits.len(), 2);
        assert_eq!(res.hits[0].line, 2);
        assert_eq!(res.hits[1].line, 3);
        assert!(!res.hits[0].private_omitted);
        assert!(res.hits[0].preview.as_ref().unwrap().contains("FooBar"));
    }

    #[test]
    fn case_sensitive_distinct() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        write_md(&root, "b.md", "only lower abc end");

        let ci = search_workspace_text_blocking(
            &root,
            SearchWorkspaceTextArgs {
                query: "Abc".to_string(),
                case_sensitive: false,
                max_files_to_scan: Some(50),
                max_bytes_per_file: Some(100_000),
                max_scan_bytes_per_file: None,
                max_hits_per_file: Some(10),
                max_total_hits: Some(20),
                max_duration_ms: Some(10_000),
            },
        )
        .unwrap();
        assert_eq!(ci.hits.len(), 1);

        let cs = search_workspace_text_blocking(
            &root,
            SearchWorkspaceTextArgs {
                query: "Abc".to_string(),
                case_sensitive: true,
                max_files_to_scan: Some(50),
                max_bytes_per_file: Some(100_000),
                max_scan_bytes_per_file: None,
                max_hits_per_file: Some(10),
                max_total_hits: Some(20),
                max_duration_ms: Some(10_000),
            },
        )
        .unwrap();
        assert_eq!(cs.hits.len(), 0);
    }

    #[test]
    fn prefix_scan_skips_tail_matches() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        let mut head = vec![b'x'; 6000];
        head.extend_from_slice(b"\nTAIL_MARKER_ONLY\n");
        let p = root.join("big.md");
        fs::write(&p, &head).unwrap();

        let res = search_workspace_text_blocking(
            &root,
            SearchWorkspaceTextArgs {
                query: "TAIL_MARKER_ONLY".to_string(),
                case_sensitive: true,
                max_files_to_scan: Some(20),
                max_bytes_per_file: Some(1_000_000),
                max_scan_bytes_per_file: Some(5000),
                max_hits_per_file: Some(10),
                max_total_hits: Some(20),
                max_duration_ms: Some(10_000),
            },
        )
        .unwrap();
        assert_eq!(res.hits.len(), 0);
        assert!(res.meta.files_scanned_as_prefix_only >= 1);
    }
}
