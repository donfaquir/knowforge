//! 理解网络：以 Vault 内 Markdown 为节点、无别名 `[[wikilink]]` 为边（文档 6.F）。
//! 含随手想法的笔记由侧车 SQLite 聚合 `thought_count` / 最高成熟度；无想法的笔记仍入图。
//! 私密笔记不入图；节点上限 200，与认知报告共用 MAX_FILES / READ_CAP。

use crate::note_privacy;
use crate::thought_parser::ThoughtMaturity;
use crate::vault_context_search;
use crate::vault_thoughts_db;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

const MAX_FILES: usize = 600;
const READ_CAP: usize = 512 * 1024;
const MAX_NODES: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnderstandingGraphNode {
    pub rel_path: String,
    pub thought_count: usize,
    pub max_maturity: String,
    pub last_updated: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnderstandingGraphEdge {
    pub from_rel_path: String,
    pub to_rel_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnderstandingGraphForUi {
    pub nodes: Vec<UnderstandingGraphNode>,
    pub edges: Vec<UnderstandingGraphEdge>,
    /// 至少含 1 条随手想法的笔记数（截断前，与 `nodes` 中 `thought_count>0` 的集合一致）
    pub candidate_thought_note_count: usize,
    /// 本次 walk 收集到的 Markdown 路径数（≤ MAX_FILES）
    pub indexed_markdown_count: usize,
    /// 因 200 上限未展示的节点数
    pub hidden_node_count: usize,
}

fn dirname_rel(rel_path: &str) -> String {
    let n = rel_path.replace('\\', "/").trim_end_matches('/').to_string();
    let i = n.rfind('/');
    match i {
        None | Some(0) => String::new(),
        Some(i) => n[..i].to_string(),
    }
}

fn strip_md_suffix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".md") {
        name[..name.len() - 3].to_string()
    } else {
        name.to_string()
    }
}

/// 将 Vault 内笔记相对路径规范为带小写 `.md` 尾缀（与前端 `normalizeMarkdownRelPath` 一致）
pub(crate) fn normalize_markdown_rel_path(rel: &str) -> String {
    let n = rel.replace('\\', "/").trim_end_matches('/').to_string();
    if n.is_empty() {
        return String::new();
    }
    let dir = dirname_rel(&n);
    let base = if dir.is_empty() {
        n.clone()
    } else {
        n[dir.len() + 1..].to_string()
    };
    let file = format!("{}.md", strip_md_suffix(&base));
    if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    }
}

/// 与前端 `resolveWikiNoteRelPath` 一致：path_part 无 `#`（调用方已剥离）。
pub(crate) fn resolve_wiki_note_rel_path(current_rel_path: &str, path_part: &str) -> Option<String> {
    let p = path_part.trim().replace('\\', "/");
    if p.is_empty() {
        let cur = normalize_markdown_rel_path(current_rel_path);
        if cur.is_empty() {
            return None;
        }
        note_privacy::validate_workspace_rel_path(&cur).ok()?;
        return Some(cur);
    }
    let joined = if p.contains('/') {
        let dir = dirname_rel(&p);
        let base = if dir.is_empty() {
            p.clone()
        } else {
            p[dir.len() + 1..].to_string()
        };
        let file = format!("{}.md", strip_md_suffix(&base));
        if dir.is_empty() {
            file
        } else {
            format!("{dir}/{file}")
        }
    } else {
        let dir = dirname_rel(current_rel_path);
        let file = format!("{}.md", strip_md_suffix(&p));
        if dir.is_empty() {
            file
        } else {
            format!("{dir}/{file}")
        }
    };
    note_privacy::validate_workspace_rel_path(&joined).ok()?;
    Some(joined.replace('\\', "/"))
}

/// wikilink 内层：无 `|`，`#` 前为路径段；返回目标 relPath，自链返回 None。
pub(crate) fn resolve_wikilink_inner_to_rel_path(source_rel: &str, inner: &str) -> Option<String> {
    let t = inner.trim();
    if t.is_empty() || t.contains('|') {
        return None;
    }
    let path_part = t.split('#').next().unwrap_or("").trim();
    let target = resolve_wiki_note_rel_path(source_rel, path_part)?;
    let norm_source = normalize_markdown_rel_path(source_rel);
    if target == norm_source {
        return None;
    }
    Some(target)
}

fn file_modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn maturity_from_graph_rank(rank: u8) -> ThoughtMaturity {
    match rank {
        2 => ThoughtMaturity::Mature,
        1 => ThoughtMaturity::Growing,
        _ => ThoughtMaturity::Seedling,
    }
}

/// 从全文提取 MVP wikilink：`[[inner]]`，排除 `![[`、含 `|` 别名。
pub(crate) fn extract_wikilink_inners(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i + 3 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == '[' {
            if i > 0 && chars[i - 1] == '!' {
                i += 1;
                continue;
            }
            let start = i + 2;
            let mut j = start;
            let mut inner = String::new();
            let mut ok = false;
            while j + 1 < chars.len() {
                if chars[j] == ']' && chars[j + 1] == ']' {
                    ok = true;
                    break;
                }
                inner.push(chars[j]);
                j += 1;
            }
            if ok {
                if !inner.contains('|') {
                    out.push(inner);
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub fn scan_understanding_graph_blocking(root: &Path) -> Result<UnderstandingGraphForUi, String> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(root, root, &mut paths, MAX_FILES)?;
    let indexed_markdown_count = paths.len();

    let stats_map: HashMap<String, (usize, u8)> = match vault_thoughts_db::open_thoughts_db(root) {
        Ok(conn) => match vault_thoughts_db::graph_thought_stats(&conn) {
            Ok(rows) => rows.into_iter().map(|(p, c, r)| (p, (c, r))).collect(),
            Err(_) => HashMap::new(),
        },
        Err(_) => HashMap::new(),
    };

    #[derive(Clone)]
    struct Candidate {
        rel_path: String,
        abs: std::path::PathBuf,
        thought_count: usize,
        max_maturity: ThoughtMaturity,
        last_updated: u64,
    }

    let mut candidates: Vec<Candidate> = Vec::new();

    for abs in paths {
        let Some(rel_path) = vault_context_search::rel_path_from_root(root, &abs) else {
            continue;
        };
        let rel_path = rel_path.replace('\\', "/");
        let bytes = fs::read(&abs).map_err(|e| crate::sanitize_io_error(e, "reading markdown"))?;
        if bytes.len() > READ_CAP {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if note_privacy::markdown_treat_as_kf_private(&text) {
            continue;
        }
        let (thought_count, max_rank) = stats_map.get(&rel_path).copied().unwrap_or((0, 0));
        let max_maturity = if thought_count > 0 {
            maturity_from_graph_rank(max_rank)
        } else {
            // 无随手想法时仅占位；前端以 thought_count==0 显示灰色
            ThoughtMaturity::Seedling
        };
        let last_updated = file_modified_ms(&abs);
        candidates.push(Candidate {
            rel_path,
            abs,
            thought_count,
            max_maturity,
            last_updated,
        });
    }

    let candidate_thought_note_count = candidates.iter().filter(|c| c.thought_count > 0).count();

    let candidate_rel: HashSet<String> = candidates.iter().map(|c| c.rel_path.clone()).collect();
    let mut edges_set: HashSet<(String, String)> = HashSet::new();
    for c in &candidates {
        let bytes = fs::read(&c.abs).map_err(|e| crate::sanitize_io_error(e, "reading markdown"))?;
        if bytes.len() > READ_CAP {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        for inner in extract_wikilink_inners(&text) {
            if let Some(to) = resolve_wikilink_inner_to_rel_path(&c.rel_path, &inner) {
                if candidate_rel.contains(&to) {
                    edges_set.insert((c.rel_path.clone(), to));
                }
            }
        }
    }

    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| {
        let ca = &candidates[a];
        let cb = &candidates[b];
        let ha = ca.thought_count > 0;
        let hb = cb.thought_count > 0;
        // 有随手想法的笔记优先；其次按成熟度、最近修改时间
        hb.cmp(&ha).then_with(|| {
            let ra = crate::thought_parser::thought_maturity_rank(ca.max_maturity);
            let rb = crate::thought_parser::thought_maturity_rank(cb.max_maturity);
            rb.cmp(&ra)
        }).then_with(|| cb.last_updated.cmp(&ca.last_updated))
    });

    let take_n = candidates.len().min(MAX_NODES);
    let mut kept: HashSet<String> = HashSet::new();
    let mut nodes: Vec<UnderstandingGraphNode> = Vec::with_capacity(take_n);
    for &oi in order.iter().take(take_n) {
        let c = &candidates[oi];
        kept.insert(c.rel_path.clone());
        nodes.push(UnderstandingGraphNode {
            rel_path: c.rel_path.clone(),
            thought_count: c.thought_count,
            max_maturity: crate::thought_parser::thought_maturity_as_str(c.max_maturity).to_string(),
            last_updated: c.last_updated,
        });
    }

    let hidden_node_count = candidates.len().saturating_sub(take_n);

    let mut edges: Vec<UnderstandingGraphEdge> = Vec::new();
    for (from, to) in edges_set {
        if kept.contains(&from) && kept.contains(&to) {
            edges.push(UnderstandingGraphEdge { from_rel_path: from, to_rel_path: to });
        }
    }
    edges.sort_by(|a, b| {
        a.from_rel_path
            .cmp(&b.from_rel_path)
            .then(a.to_rel_path.cmp(&b.to_rel_path))
    });

    Ok(UnderstandingGraphForUi {
        nodes,
        edges,
        candidate_thought_note_count,
        indexed_markdown_count,
        hidden_node_count,
    })
}

#[tauri::command]
pub async fn scan_understanding_graph(
    state: tauri::State<'_, crate::WorkspaceState>,
) -> Result<UnderstandingGraphForUi, String> {
    let canonical_root = crate::lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || scan_understanding_graph_blocking(&canonical_root))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_same_dir_title() {
        assert_eq!(
            resolve_wiki_note_rel_path("notes/a.md", "B").as_deref(),
            Some("notes/B.md")
        );
    }

    #[test]
    fn resolve_vault_path() {
        assert_eq!(
            resolve_wiki_note_rel_path("notes/a.md", "other/x").as_deref(),
            Some("other/x.md")
        );
        assert_eq!(
            resolve_wiki_note_rel_path("notes/a.md", "other/x.md").as_deref(),
            Some("other/x.md")
        );
        assert_eq!(
            resolve_wiki_note_rel_path("notes/a.md", "other/x.MD").as_deref(),
            Some("other/x.md")
        );
    }

    #[test]
    fn resolve_empty_path_part_normalizes_current() {
        assert_eq!(
            resolve_wiki_note_rel_path("notes/n.MD", "").as_deref(),
            Some("notes/n.md")
        );
        assert_eq!(
            resolve_wiki_note_rel_path("notes/note", "").as_deref(),
            Some("notes/note.md")
        );
    }

    #[test]
    fn wikilink_heading_stripped() {
        assert_eq!(
            resolve_wikilink_inner_to_rel_path("a.md", "B#Sec").as_deref(),
            Some("B.md")
        );
    }

    #[test]
    fn wikilink_skips_alias_and_embed() {
        assert_eq!(resolve_wikilink_inner_to_rel_path("a.md", "X|Y"), None);
    }

    #[test]
    fn extract_inners_basic() {
        let s = "Hello [[Note]] and ![[Img]] and [[Dir/N]] end.";
        let v = extract_wikilink_inners(s);
        assert!(v.contains(&"Note".to_string()));
        assert!(v.contains(&"Dir/N".to_string()));
        assert!(!v.iter().any(|x| x.contains('!')));
    }

    #[test]
    fn reject_escape_rel_path() {
        assert!(resolve_wiki_note_rel_path("notes/a.md", "../secret").is_none());
    }

    #[test]
    fn self_wikilink_no_edge_target() {
        assert_eq!(resolve_wikilink_inner_to_rel_path("notes/a.md", "a"), None);
    }
}
