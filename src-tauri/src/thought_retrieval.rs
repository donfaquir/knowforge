//! Vault 内理解区块检索：关键词对侧车 SQLite 中 `body` 打分（正文 SSOT）。
//!
//! 2 秒超时后前端不展示邀请区。

use crate::join_under_root;
use crate::note_privacy;
use crate::thought_parser;
use crate::vault_context_search;
use crate::vault_thoughts_db;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

// --- IPC 类型 ---

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchThoughtArgs {
    pub query: String,
    /// 排除当前编辑中的笔记，避免刚写的 thought 立即被检索到
    #[serde(default)]
    pub exclude_rel_paths: Vec<String>,
    /// 关键词检索返回条数上限（先答后邀默认 1；回顾通道可取 3–20）
    #[serde(default = "default_search_max_results")]
    pub max_results: usize,
}

fn default_search_max_results() -> usize {
    1
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtRetrievalResult {
    pub rel_path: String,
    pub thought_id: String,
    pub excerpt: String,
    pub maturity: thought_parser::ThoughtMaturity,
    pub score: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_omitted: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchThoughtResponse {
    /// 兼容：取 `thoughts` 第一条；新客户端优先读 `thoughts`
    pub thought: Option<ThoughtRetrievalResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thoughts: Vec<ThoughtRetrievalResult>,
    pub meta: SearchThoughtMeta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchThoughtMeta {
    pub scanned_files: usize,
    pub stopped_early: bool,
    pub elapsed_ms: u64,
    /// 非空表示检索未能走通侧车（与「查过但无命中」区分）；前端可提示重建索引或检查权限
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

// --- 检索逻辑 ---

const DEADLINE_MS: u64 = 2000;

/// `SearchThoughtMeta.error_code`：侧车 SQLite 无法打开或初始化，空结果不代表「无命中」
pub const SEARCH_THOUGHT_ERROR_SIDECAR_UNAVAILABLE: &str = "sidecar_unavailable";

/// Vault 内一条 thought 的摘要（用于回顾排期与统计，不含关键词分）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultThoughtEntry {
    pub rel_path: String,
    pub thought_id: String,
    pub excerpt: String,
    pub maturity: thought_parser::ThoughtMaturity,
    pub created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reviewed_at: Option<String>,
    pub challenge_pass_count: u32,
    pub temporary: bool,
    pub private_omitted: bool,
}

fn clip_thought_body_preview(body: &str, max_chars: usize) -> String {
    let t = body.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    let mut end = max_chars;
    while !t.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &t[..end])
}

fn push_top_thoughts(top: &mut Vec<ThoughtRetrievalResult>, cand: ThoughtRetrievalResult, max: usize) {
    if cand.score == 0 {
        return;
    }
    top.push(cand);
    top.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
            .then_with(|| a.thought_id.cmp(&b.thought_id))
    });
    top.truncate(max);
}

pub fn search_thought_blocking(
    canonical_root: &Path,
    args: SearchThoughtArgs,
) -> Result<SearchThoughtResponse, String> {
    let started = Instant::now();

    let max_results = args.max_results.max(1).min(20);

    let tokens = vault_context_search::tokenize_query(&args.query);
    if tokens.is_empty() {
        return Ok(SearchThoughtResponse {
            thought: None,
            thoughts: Vec::new(),
            meta: SearchThoughtMeta {
                scanned_files: 0,
                stopped_early: false,
                elapsed_ms: started.elapsed().as_millis() as u64,
                error_code: None,
            },
        });
    }

    let exclude: std::collections::HashSet<String> = args
        .exclude_rel_paths
        .iter()
        .filter_map(|p| {
            note_privacy::validate_workspace_rel_path(p)
                .ok()
                .map(|_| p.replace('\\', "/"))
        })
        .collect();

    let mut top: Vec<ThoughtRetrievalResult> = Vec::new();
    let mut scanned = 0usize;
    let mut stopped_early = false;

    let conn = match vault_thoughts_db::open_thoughts_db(canonical_root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[search_thought] sidecar DB unavailable ({}): {e}",
                SEARCH_THOUGHT_ERROR_SIDECAR_UNAVAILABLE
            );
            return Ok(SearchThoughtResponse {
                thought: None,
                thoughts: Vec::new(),
                meta: SearchThoughtMeta {
                    scanned_files: 0,
                    stopped_early: false,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    error_code: Some(SEARCH_THOUGHT_ERROR_SIDECAR_UNAVAILABLE.to_string()),
                },
            });
        }
    };

    let rows = vault_thoughts_db::list_all_thought_rows_for_scan(&conn)?;
    for (rel, thought_id, body, mat_str) in rows {
        if started.elapsed().as_millis() as u64 > DEADLINE_MS {
            stopped_early = true;
            break;
        }
        scanned += 1;
        if exclude.contains(&rel) {
            continue;
        }
        let Ok(joined) = join_under_root(canonical_root, &rel) else {
            continue;
        };
        if !joined.exists() {
            continue;
        }
        if note_privacy::peek_kf_private_from_md_file(&joined) {
            continue;
        }
        let score = vault_context_search::score_prefix(&body, &tokens);
        if score == 0 {
            continue;
        }
        let maturity = thought_parser::thought_maturity_from_storage(&mat_str);
        let excerpt = clip_thought_body_preview(&body, 240);
        push_top_thoughts(
            &mut top,
            ThoughtRetrievalResult {
                rel_path: rel,
                thought_id,
                excerpt,
                maturity,
                score,
                private_omitted: None,
            },
            max_results,
        );
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let thought = top.first().cloned();
    Ok(SearchThoughtResponse {
        thought,
        thoughts: top,
        meta: SearchThoughtMeta {
            scanned_files: scanned,
            stopped_early: stopped_early || elapsed_ms >= DEADLINE_MS,
            elapsed_ms,
            error_code: None,
        },
    })
}

const REVIEW_QUEUE_DEADLINE_MS: u64 = 12_000;

/// 扫描侧车中非临时 thought（用于回顾排期）。
pub fn enumerate_vault_thought_entries_blocking(
    canonical_root: &Path,
) -> Result<(Vec<VaultThoughtEntry>, SearchThoughtMeta), String> {
    let started = Instant::now();
    let conn = vault_thoughts_db::open_thoughts_db(canonical_root)?;
    let rows = vault_thoughts_db::list_thought_rows_for_review(&conn)?;

    let mut out = Vec::new();
    let mut scanned = 0usize;
    let mut stopped_early = false;

    for (
        rel,
        thought_id,
        body,
        mat_str,
        temporary,
        created_at,
        _updated_at,
        cpc,
        last_reviewed_at,
    ) in rows
    {
        if started.elapsed().as_millis() as u64 > REVIEW_QUEUE_DEADLINE_MS {
            stopped_early = true;
            break;
        }
        scanned += 1;
        if temporary || thought_id.is_empty() {
            continue;
        }
        let Ok(joined) = join_under_root(canonical_root, &rel) else {
            continue;
        };
        if !joined.exists() {
            continue;
        }
        let is_private = note_privacy::peek_kf_private_from_md_file(&joined);
        out.push(VaultThoughtEntry {
            rel_path: rel,
            thought_id,
            excerpt: if is_private {
                String::new()
            } else {
                clip_thought_body_preview(&body, 240)
            },
            maturity: thought_parser::thought_maturity_from_storage(&mat_str),
            created: created_at,
            last_reviewed_at,
            challenge_pass_count: cpc.max(0) as u32,
            temporary,
            private_omitted: is_private,
        });
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let meta = SearchThoughtMeta {
        scanned_files: scanned,
        stopped_early: stopped_early || elapsed_ms >= REVIEW_QUEUE_DEADLINE_MS,
        elapsed_ms,
        error_code: None,
    };
    Ok((out, meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn seed_thought(root: &Path, rel: &str, note_id: &str, thought_id: &str, body: &str) {
        let md = format!(
            "---\nkfVaultNoteId: {note_id}\nkf-thoughts:\n- id: {thought_id}\n  maturity: seedling\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n---\n# hi\n"
        );
        write_md(root, rel, &md);
        let conn = vault_thoughts_db::open_thoughts_db(root).unwrap();
        vault_thoughts_db::upsert_thought_body(
            &conn,
            thought_id,
            note_id,
            rel,
            body,
            None,
            "seedling",
            false,
            false,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            0,
            None,
        )
        .unwrap();
    }

    fn write_md(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn tmp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "thought_retr_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn sidecar_body_search_is_ascii_case_insensitive() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        seed_thought(
            &root,
            "mixed.md",
            "nid-m",
            "t-m",
            "CaseInsensitiveNeedle in body\n",
        );
        let res = search_thought_blocking(
            &root,
            SearchThoughtArgs {
                query: "caseinsensitiveneedle".to_string(),
                exclude_rel_paths: vec![],
                max_results: 1,
            },
        )
        .unwrap();
        assert!(res.thought.is_some());
    }

    #[test]
    fn finds_most_relevant_thought() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        seed_thought(
            &root,
            "a.md",
            "nid-a",
            "t-a",
            "Rust编程 is awesome\n",
        );
        seed_thought(
            &root,
            "b.md",
            "nid-b",
            "t-b",
            "Python 编程 is fun too\n",
        );
        let res = search_thought_blocking(
            &root,
            SearchThoughtArgs {
                query: "Rust编程".to_string(),
                exclude_rel_paths: vec![],
                max_results: 1,
            },
        )
        .unwrap();
        assert!(res.thought.is_some());
        let t = res.thought.unwrap();
        assert_eq!(t.rel_path, "a.md");
        assert!(t.excerpt.contains("Rust编程"));
    }

    #[test]
    fn no_thought_returns_none() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        write_md(&root, "x.md", "# Just a note\nNo thought blocks here.\n");
        let res = search_thought_blocking(
            &root,
            SearchThoughtArgs {
                query: "something".to_string(),
                exclude_rel_paths: vec![],
                max_results: 1,
            },
        )
        .unwrap();
        assert!(res.thought.is_none());
    }

    #[test]
    fn excludes_specified_paths() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        seed_thought(&root, "only.md", "nid-o", "t-o", "uniqueterm xyz\n");
        let res = search_thought_blocking(
            &root,
            SearchThoughtArgs {
                query: "uniqueterm".to_string(),
                exclude_rel_paths: vec!["only.md".to_string()],
                max_results: 1,
            },
        )
        .unwrap();
        assert!(res.thought.is_none());
    }

    #[test]
    fn private_file_skipped_in_search() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        seed_thought(
            &root,
            "secret.md",
            "nid-s",
            "t-s",
            "秘密keyword here\n",
        );
        let secret_path = root.join("secret.md");
        let mut raw = fs::read_to_string(&secret_path).unwrap();
        raw = raw.replacen("---\n", "---\nkf-private: true\n", 1);
        fs::write(&secret_path, raw).unwrap();
        let res = search_thought_blocking(
            &root,
            SearchThoughtArgs {
                query: "秘密keyword".to_string(),
                exclude_rel_paths: vec![],
                max_results: 1,
            },
        )
        .unwrap();
        assert!(res.thought.is_none());
    }
}
