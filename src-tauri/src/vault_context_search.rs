//! Vault 级关键词检索（任务 08 MVP）：与 `build_md_tree` 相同遍历规则，供 AI 上下文摘录。
//!
//! Private hits are tagged `privateOmitted` at search time; `assemble_messages` re-derives excerpts from disk to avoid trusting frontend payloads.

use crate::note_privacy;
use crate::vault_config;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 与 `lib::build_md_tree` 一致：仅 `.md` / `.markdown`。
fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "md" || e == "markdown"
        })
        .unwrap_or(false)
}

/// 递归收集 Markdown 文件绝对路径（跳过点开头条目与符号链接）；与 `build_md_tree` 对齐。
pub(crate) fn walk_markdown_files(
    canonical_root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    max_files: usize,
) -> Result<(), String> {
    if out.len() >= max_files {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| crate::sanitize_io_error(e, "listing workspace directory"))?;
    for entry in entries {
        let entry = entry.map_err(|e| crate::sanitize_io_error(e, "reading directory entry"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| crate::sanitize_io_error(e, "reading file metadata"))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_markdown_files(canonical_root, &path, out, max_files)?;
        } else if meta.is_file() && is_markdown_path(&path) {
            out.push(path);
            if out.len() >= max_files {
                return Ok(());
            }
        }
    }
    Ok(())
}

pub(crate) fn rel_path_from_root(canonical_root: &Path, abs: &Path) -> Option<String> {
    abs.strip_prefix(canonical_root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// 无空格中文问句等：整句作为一个 token 时，正文里往往只有「工号」等子串，需用双字片段参与命中。
fn is_cjk_dense_token(s: &str) -> bool {
    let mut n = 0usize;
    let mut han = 0usize;
    for c in s.chars() {
        n += 1;
        if ('\u{4e00}'..='\u{9fff}').contains(&c) {
            han += 1;
        }
    }
    n >= 4 && han * 2 >= n
}

pub(crate) fn tokenize_query(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in query.split_whitespace() {
        let t = w.trim();
        if t.chars().count() >= 2 {
            out.push(t.to_lowercase());
        }
    }
    if out.is_empty() {
        let t = query.trim();
        if !t.is_empty() {
            out.push(t.to_lowercase());
        }
    }
    let base_len = out.len();
    let mut seen: HashSet<String> = out.iter().cloned().collect();
    for i in 0..base_len {
        let t = &out[i];
        if t.chars().count() >= 4 && is_cjk_dense_token(t) {
            let chars: Vec<char> = t.chars().collect();
            let mut added = 0usize;
            for w in chars.windows(2) {
                if added >= 48 {
                    break;
                }
                let pair: String = w.iter().collect();
                if seen.insert(pair.clone()) {
                    out.push(pair);
                    added += 1;
                }
            }
        }
    }
    out
}

/// 逐字符大小写无关比较（比较各字符 `to_lowercase()` 迭代器是否相等）。
/// 不在「整段 `to_lowercase()` 新串」上 `find` 再映射回原文：Unicode 下扩长（如 ß→ss）会导致字节下标与原文错位。
fn substring_match_ci(haystack_chars: &[char], needle_chars: &[char], start: usize) -> bool {
    needle_chars.iter().enumerate().all(|(j, &nc)| {
        haystack_chars
            .get(start + j)
            .copied()
            .is_some_and(|hc| hc.to_lowercase().eq(nc.to_lowercase()))
    })
}

/// 第 i 个 Unicode 标量在 `s` 中的 UTF-8 起始字节；`len == s.chars().count() + 1`，末元为 `s.len()`（尾后字节）。
/// 避免对每个匹配重复 `chars().take(i).map(len_utf8).sum()` 的 O(i) 扫描。
fn utf8_char_start_byte_offsets(s: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(s.chars().count().saturating_add(1));
    let mut off = 0usize;
    offsets.push(0);
    for c in s.chars() {
        off += c.len_utf8();
        offsets.push(off);
    }
    offsets
}

/// 在 `haystack` 中找 needle 的**首次**出现，返回 `[start_byte, end_byte)`（均为 `haystack` 的 UTF-8 字节下标）。
fn find_case_insensitive_substring_first_range(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    let hchars: Vec<char> = haystack.chars().collect();
    let nchars: Vec<char> = needle.chars().collect();
    let nlen = nchars.len();
    if nlen == 0 || nlen > hchars.len() {
        return None;
    }
    let offsets = utf8_char_start_byte_offsets(haystack);
    debug_assert_eq!(offsets.len(), hchars.len() + 1);
    for i in 0..=hchars.len() - nlen {
        if substring_match_ci(&hchars, &nchars, i) {
            return Some((offsets[i], offsets[i + nlen]));
        }
    }
    None
}

/// 取所有 token 在文档中**最先出现**（最小起始字节）的那一处匹配区间。
fn find_first_token_match_byte_range(content: &str, tokens: &[String]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for tok in tokens {
        if tok.is_empty() {
            continue;
        }
        if let Some(r) = find_case_insensitive_substring_first_range(content, tok) {
            best = Some(match best {
                None => r,
                Some(b) if r.0 < b.0 => r,
                Some(b) => b,
            });
        }
    }
    best
}

/// 大小写无关、**非重叠**子串，枚举每次命中的 UTF-8 字节区间 `[start, end)`。
pub(crate) fn enumerate_nonoverlapping_ci_byte_ranges(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let hchars: Vec<char> = haystack.chars().collect();
    let nchars: Vec<char> = needle.chars().collect();
    let nlen = nchars.len();
    if nlen == 0 || nlen > hchars.len() {
        return Vec::new();
    }
    let offsets = utf8_char_start_byte_offsets(haystack);
    debug_assert_eq!(offsets.len(), hchars.len() + 1);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + nlen <= hchars.len() {
        if substring_match_ci(&hchars, &nchars, i) {
            out.push((offsets[i], offsets[i + nlen]));
            i += nlen;
        } else {
            i += 1;
        }
    }
    out
}

/// 大小写敏感、非重叠子串，UTF-8 字节区间。
pub(crate) fn enumerate_nonoverlapping_cs_byte_ranges(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Vec::new();
    }
    let hchars: Vec<char> = haystack.chars().collect();
    let nchars: Vec<char> = needle.chars().collect();
    let nlen = nchars.len();
    if nlen == 0 || nlen > hchars.len() {
        return Vec::new();
    }
    let offsets = utf8_char_start_byte_offsets(haystack);
    debug_assert_eq!(offsets.len(), hchars.len() + 1);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + nlen <= hchars.len() {
        if hchars[i..i + nlen] == nchars[..] {
            out.push((offsets[i], offsets[i + nlen]));
            i += nlen;
        } else {
            i += 1;
        }
    }
    out
}

/// 大小写无关、**非重叠**子串计数（与旧 `str::matches` 子串语义接近，但在原始 UTF-8 上比较）。
fn count_nonoverlapping_ci(haystack: &str, needle: &str) -> usize {
    let needle = needle.trim();
    if needle.is_empty() {
        return 0;
    }
    let hchars: Vec<char> = haystack.chars().collect();
    let nchars: Vec<char> = needle.chars().collect();
    let nlen = nchars.len();
    if nlen == 0 || nlen > hchars.len() {
        return 0;
    }
    let mut i = 0usize;
    let mut count = 0usize;
    while i + nlen <= hchars.len() {
        if substring_match_ci(&hchars, &nchars, i) {
            count += 1;
            i += nlen;
        } else {
            i += 1;
        }
    }
    count
}

pub(crate) fn score_prefix(content: &str, tokens: &[String]) -> usize {
    tokens
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| count_nonoverlapping_ci(content, t))
        .sum()
}

/// 以「所有 token 中起始字节最小」的首次命中为中心，按 **Unicode 标量个数** 取 `max_chars` 宽窗口；`chars().skip/take` 保证边界合法。
fn extract_snippet_window(content: &str, tokens: &[String], max_chars: usize) -> Option<String> {
    let max_chars = max_chars.max(1);
    let (m_start_byte, m_end_byte) = find_first_token_match_byte_range(content, tokens)?;
    let match_start_char = content.get(..m_start_byte)?.chars().count();
    let match_len_chars = content.get(m_start_byte..m_end_byte)?.chars().count();
    let mid = match_start_char + match_len_chars / 2;
    let half = max_chars / 2;
    let win_start = mid.saturating_sub(half);
    Some(content.chars().skip(win_start).take(max_chars).collect())
}

fn redact_private_snippets(ai: &vault_config::AiConfig) -> bool {
    ai.should_redact_private()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VaultSnippetKind {
    Excerpt,
    PrivateOmitted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSnippetRecord {
    pub rel_path: String,
    pub kind: VaultSnippetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceLimits {
    #[serde(default)]
    pub max_files_to_scan: Option<usize>,
    #[serde(default)]
    pub max_snippets: Option<usize>,
    #[serde(default)]
    pub max_chars_per_snippet: Option<usize>,
    #[serde(default)]
    pub max_total_chars: Option<usize>,
    #[serde(default)]
    pub read_bytes_per_file: Option<usize>,
    #[serde(default)]
    pub max_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceContextArgs {
    pub query: String,
    #[serde(default)]
    pub exclude_rel_paths: Vec<String>,
    #[serde(default)]
    pub limits: Option<SearchWorkspaceLimits>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceContextMeta {
    pub scanned_files: usize,
    pub stopped_early: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWorkspaceContextResponse {
    pub snippets: Vec<VaultSnippetRecord>,
    pub meta: SearchWorkspaceContextMeta,
}

/// 在工作区内做关键词扫描；磁盘 I/O 仅在 `spawn_blocking` 中调用本函数。
pub fn search_workspace_context_blocking(
    canonical_root: &Path,
    args: SearchWorkspaceContextArgs,
) -> Result<SearchWorkspaceContextResponse, String> {
    let started = Instant::now();
    let ai = vault_config::load_ai_config_internal(canonical_root)?;
    let redact_private = redact_private_snippets(&ai);

    let lim = args.limits.as_ref();
    let max_files = lim.and_then(|l| l.max_files_to_scan).unwrap_or(400).max(1);
    let max_snippets = lim.and_then(|l| l.max_snippets).unwrap_or(8).max(1);
    let max_chars_snippet = lim.and_then(|l| l.max_chars_per_snippet).unwrap_or(1200).max(200);
    let read_bytes = lim.and_then(|l| l.read_bytes_per_file).unwrap_or(96 * 1024).max(4096);
    let deadline_ms = lim.and_then(|l| l.max_duration_ms).unwrap_or(8000).max(500);
    let max_total_excerpt_chars = lim.and_then(|l| l.max_total_chars).unwrap_or(24_000).max(800);

    let mut exclude: HashSet<String> = HashSet::new();
    for p in &args.exclude_rel_paths {
        if note_privacy::validate_workspace_rel_path(p).is_ok() {
            exclude.insert(p.replace('\\', "/"));
        }
    }

    let mut paths: Vec<PathBuf> = Vec::new();
    walk_markdown_files(canonical_root, canonical_root, &mut paths, max_files)?;
    let stopped_early_collection = paths.len() >= max_files;

    let tokens = tokenize_query(&args.query);
    let mut scored: Vec<(String, PathBuf, usize)> = Vec::new();

    let mut scanned = 0usize;
    for abs in paths {
        if started.elapsed().as_millis() as u64 > deadline_ms {
            break;
        }
        scanned += 1;
        let Some(rel) = rel_path_from_root(canonical_root, &abs) else {
            continue;
        };
        if exclude.contains(&rel) {
            continue;
        }
        let bytes = fs::read(&abs).map_err(|e| crate::sanitize_io_error(e, "reading markdown for search"))?;
        let take = bytes.len().min(read_bytes);
        let head = String::from_utf8_lossy(&bytes[..take]);
        let score = score_prefix(head.as_ref(), &tokens);
        if score > 0 {
            scored.push((rel, abs, score));
        }
    }

    scored.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(max_snippets);

    let mut snippets: Vec<VaultSnippetRecord> = Vec::new();
    let mut excerpt_chars_used = 0usize;
    for (rel, abs, _) in scored {
        if started.elapsed().as_millis() as u64 > deadline_ms {
            break;
        }
        let is_private = note_privacy::peek_kf_private_from_md_file(&abs);
        if is_private && redact_private {
            let row_cost = rel.chars().count() + 80;
            if excerpt_chars_used + row_cost > max_total_excerpt_chars {
                break;
            }
            excerpt_chars_used += row_cost;
            snippets.push(VaultSnippetRecord {
                rel_path: rel.clone(),
                kind: VaultSnippetKind::PrivateOmitted,
                excerpt: None,
            });
            continue;
        }
        let bytes = fs::read(&abs).map_err(|e| crate::sanitize_io_error(e, "reading markdown for excerpt"))?;
        let take = bytes.len().min(read_bytes);
        let head = String::from_utf8_lossy(&bytes[..take]).into_owned();
        let excerpt = extract_snippet_window(&head, &tokens, max_chars_snippet).unwrap_or_else(|| {
            head.chars().take(max_chars_snippet).collect()
        });
        let n = excerpt.chars().count() + rel.chars().count() + 40;
        if excerpt_chars_used + n > max_total_excerpt_chars {
            break;
        }
        excerpt_chars_used += n;
        snippets.push(VaultSnippetRecord {
            rel_path: rel,
            kind: VaultSnippetKind::Excerpt,
            excerpt: Some(excerpt),
        });
    }

    let elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(SearchWorkspaceContextResponse {
        snippets,
        meta: SearchWorkspaceContextMeta {
            scanned_files: scanned,
            stopped_early: stopped_early_collection || started.elapsed().as_millis() as u64 >= deadline_ms,
            elapsed_ms,
        },
    })
}

/// 根据磁盘重算摘录，**忽略前端传入的 excerpt 正文**（防篡改）；保留传入顺序与条数上限。
pub fn rebuild_vault_snippets_for_llm(
    canonical_root: &Path,
    ai: &vault_config::AiConfig,
    incoming: &[VaultSnippetRecord],
    user_query: &str,
    max_chars_snippet: usize,
    read_bytes: usize,
) -> Vec<VaultSnippetRecord> {
    let redact_private = redact_private_snippets(ai);
    let tokens = tokenize_query(user_query);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<VaultSnippetRecord> = Vec::new();

    for s in incoming {
        if note_privacy::validate_workspace_rel_path(&s.rel_path).is_err() {
            continue;
        }
        let rel = s.rel_path.replace('\\', "/");
        if !seen.insert(rel.clone()) {
            continue;
        }
        let Ok(abs) = crate::join_under_root(canonical_root, &rel) else {
            continue;
        };
        if !abs.is_file() || !is_markdown_path(&abs) {
            continue;
        }
        let is_private = note_privacy::peek_kf_private_from_md_file(&abs);
        if is_private && redact_private {
            out.push(VaultSnippetRecord {
                rel_path: rel,
                kind: VaultSnippetKind::PrivateOmitted,
                excerpt: None,
            });
            continue;
        }
        let bytes = match fs::read(&abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let take = bytes.len().min(read_bytes);
        let head = String::from_utf8_lossy(&bytes[..take]).into_owned();
        let excerpt = extract_snippet_window(&head, &tokens, max_chars_snippet)
            .unwrap_or_else(|| head.chars().take(max_chars_snippet).collect());
        if excerpt.trim().is_empty() {
            continue;
        }
        out.push(VaultSnippetRecord {
            rel_path: rel,
            kind: VaultSnippetKind::Excerpt,
            excerpt: Some(excerpt),
        });
    }
    out
}

/// 将 Vault 摘录拼成一条 system 消息正文；`max_chars` 已含包装文案预算。
/// 返回值：`(system_block, truncated, used_rel_paths)`，第三项仅包含实际拼进 block 的文档路径。
pub fn build_vault_context_system_block(
    snippets: &[VaultSnippetRecord],
    max_chars: usize,
) -> Option<(String, bool, Vec<String>)> {
    if snippets.is_empty() || max_chars < 80 {
        return None;
    }
    let mut body = String::from(
        "The following are excerpts from other Markdown notes in this workspace (disk snapshot; unsaved edits may be missing). ",
    );
    let mut used = body.chars().count();
    let mut truncated = false;
    let mut used_rel_paths: Vec<String> = Vec::new();

    for s in snippets {
        if used >= max_chars.saturating_sub(40) {
            body.push_str("\n\n… (vault context truncated)");
            truncated = true;
            return Some((body, truncated, used_rel_paths));
        }
        let block = match (&s.kind, &s.excerpt) {
            (VaultSnippetKind::Excerpt, Some(ex)) => {
                format!(
                    "\n\n### `{}`\n```markdown\n{}\n```",
                    s.rel_path,
                    ex.trim_end()
                )
            }
            (VaultSnippetKind::PrivateOmitted, _) => {
                format!(
                    "\n\n### `{}`\n(Private note: kf-private. Full text is not included.)",
                    s.rel_path
                )
            }
            _ => continue,
        };
        let n = block.chars().count();
        if used + n > max_chars {
            body.push_str("\n\n… (vault context truncated)");
            truncated = true;
            return Some((body, truncated, used_rel_paths));
        }
        body.push_str(&block);
        used += n;
        used_rel_paths.push(s.rel_path.clone());
    }
    Some((body, truncated, used_rel_paths))
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
            "vault_ctx_search_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn search_finds_keyword_and_excludes_path() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        write_md(&root, "a.md", "# hello uniquealphaxyz beta\n");
        write_md(&root, "sub/b.md", "nothing here");
        write_md(&root, "sub/c.md", "uniquealphaxyz in body\n");

        let res = search_workspace_context_blocking(
            &root,
            SearchWorkspaceContextArgs {
                query: "uniquealphaxyz".to_string(),
                exclude_rel_paths: vec!["a.md".to_string()],
                limits: Some(SearchWorkspaceLimits {
                    max_files_to_scan: Some(100),
                    max_snippets: Some(5),
                    max_chars_per_snippet: Some(500),
                    max_total_chars: None,
                    read_bytes_per_file: Some(8192),
                    max_duration_ms: Some(5000),
                }),
            },
        )
        .unwrap();
        assert_eq!(res.snippets.len(), 1);
        assert_eq!(res.snippets[0].rel_path, "sub/c.md");
        assert!(matches!(res.snippets[0].kind, VaultSnippetKind::Excerpt));
    }

    #[test]
    fn score_prefix_is_case_insensitive_on_original_text() {
        let tokens = tokenize_query("Foo");
        assert_eq!(score_prefix("x Foo foo FOo y", &tokens), 3);
    }

    #[test]
    fn extract_snippet_preserves_original_casing() {
        let tokens = tokenize_query("BaR");
        let s = extract_snippet_window("prefix BaR suffix tail here", &tokens, 30).expect("snippet");
        assert!(s.contains("BaR"));
    }

    #[test]
    fn extract_snippet_handles_cjk() {
        let tokens = tokenize_query("关键词");
        let body = "前言一段\n\n关键词🔥在后\n";
        let ex = extract_snippet_window(body, &tokens, 24).expect("snippet");
        assert!(ex.contains("关键词"));
    }

    #[test]
    fn tokenize_cjk_question_includes_bigrams_for_partial_match() {
        let toks = tokenize_query("我的工号是什么");
        assert!(toks.iter().any(|t| t == "工号"));
        let score = score_prefix("个人信息\n\n工号：9527\n", &toks);
        assert!(score >= 1, "expected 工号 bigram to match");
    }

    #[test]
    fn private_file_omitted_when_redact() {
        let dir = tmp_root();
        fs::create_dir_all(&dir).unwrap();
        let root = fs::canonicalize(&dir).unwrap();
        write_md(
            &root,
            "secret.md",
            "---\nkf-private: true\n---\nfindme keywordzz\n",
        );

        let res = search_workspace_context_blocking(
            &root,
            SearchWorkspaceContextArgs {
                query: "keywordzz".to_string(),
                exclude_rel_paths: vec![],
                limits: Some(SearchWorkspaceLimits {
                    max_files_to_scan: Some(50),
                    max_snippets: Some(3),
                    max_chars_per_snippet: Some(400),
                    max_total_chars: None,
                    read_bytes_per_file: Some(8192),
                    max_duration_ms: Some(5000),
                }),
            },
        )
        .unwrap();
        assert_eq!(res.snippets.len(), 1);
        assert!(matches!(
            res.snippets[0].kind,
            VaultSnippetKind::PrivateOmitted
        ));
    }
}
