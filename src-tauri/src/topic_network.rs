//! 主题网络（迭代 6.4）：LLM 提取主题、SQLite 缓存、二部图构建与 Markdown 导出快照。

use crate::llm::ollama;
use crate::llm::LlmChatMessage;
use crate::note_privacy;
use crate::semantic_index::{self, cosine_similarity};
use crate::thought_parser::{split_frontmatter, FrontmatterSplit};
use crate::understanding_graph::normalize_markdown_rel_path;
use crate::vault_config::{ActiveProvider, AiConfig};
use crate::vault_context_search;
use crate::vault_thoughts_db;
use chrono::Utc;
use hex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri::Emitter;

const MAX_FILES: usize = 600;
const READ_CAP: usize = 512 * 1024;
const EXCERPT_CHARS: usize = 4096;
const MAX_TOPICS_PER_DOC: usize = 5;
const MIN_TOPICS_PER_DOC: usize = 2;
const TOPIC_IN_MIN_DOCS: usize = 2;
const TOPIC_COOC_MIN_DOCS: usize = 3;
const MAX_TOPIC_NODES: usize = 50;
const MAX_DOC_NODES: usize = 150;
const MODEL_ID_FALLBACK: &str = "unknown";
/// 用户「新增主题」后经语义检索写入 doc_topics 的 model_id 标记
const MODEL_ID_SEMANTIC_MANUAL: &str = "semantic-manual";
/// 文档级最大相似度超过此阈值才写入该主题（与 chunk 余弦一致，约 0.4–0.5）
const MANUAL_TOPIC_MIN_SEMANTIC: f32 = 0.42;
/// 单主题语义关联写入的笔记数量上限
const MANUAL_TOPIC_MAX_DOCS: usize = 100;

// --- 路径 ---

pub fn topic_db_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".knowforge/topics/topic_cache.sqlite")
}

pub fn topic_export_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(".knowforge/topics/export")
}

pub fn open_topic_db(vault_root: &Path) -> Result<Connection, String> {
    let path = topic_db_path(vault_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 .knowforge/topics 失败: {e}"))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开 topic_cache.sqlite 失败: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 失败: {e}"))?;
    init_topic_schema(&conn)?;
    Ok(conn)
}

pub fn init_topic_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS doc_topics (
            rel_path    TEXT NOT NULL,
            topic       TEXT NOT NULL,
            confidence  REAL NOT NULL DEFAULT 1.0,
            model_id    TEXT NOT NULL,
            file_hash   TEXT NOT NULL,
            created_at  TEXT NOT NULL,
            PRIMARY KEY(rel_path, topic)
        );
        CREATE INDEX IF NOT EXISTS idx_topics_topic ON doc_topics(topic);

        CREATE TABLE IF NOT EXISTS topic_dictionary (
            canonical   TEXT PRIMARY KEY,
            display     TEXT NOT NULL,
            aliases     TEXT,
            created_at  TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化 topic 表失败: {e}"))?;
    Ok(())
}

/// 流式 SHA-256（与语义索引侧一致思路，避免整文件读入）
fn sha256_hex_file_stream(path: &Path) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("open for hash: {e}"))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| format!("read for hash: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// 删除某文档在 doc_topics 中的全部行后批量插入
pub fn upsert_doc_topics(
    conn: &Connection,
    rel_path: &str,
    topics: &[(String, f64)],
    model_id: &str,
    file_hash: &str,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("topic tx: {e}"))?;
    tx.execute("DELETE FROM doc_topics WHERE rel_path = ?1", params![rel_path])
        .map_err(|e| format!("delete doc_topics: {e}"))?;
    for (topic, conf) in topics {
        tx.execute(
            r#"INSERT INTO doc_topics (rel_path, topic, confidence, model_id, file_hash, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![rel_path, topic, conf, model_id, file_hash, now],
        )
        .map_err(|e| format!("insert doc_topics: {e}"))?;
    }
    tx.commit().map_err(|e| format!("commit doc_topics: {e}"))?;
    Ok(())
}

/// 若该行存在且 `file_hash` 与磁盘一致，返回 (主题列表, 缓存中的 hash)
pub fn get_cached_topics(conn: &Connection, rel_path: &str, current_hash: &str) -> Result<Option<Vec<String>>, String> {
    let mut stmt = conn
        .prepare("SELECT topic, file_hash FROM doc_topics WHERE rel_path = ?1")
        .map_err(|e| format!("prepare get_cached_topics: {e}"))?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![rel_path], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("query doc_topics: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("row doc_topics: {e}"))?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.iter().any(|(_, h)| h != current_hash) {
        return Ok(None);
    }
    Ok(Some(rows.into_iter().map(|(t, _)| t).collect()))
}

pub fn upsert_topic_dictionary(conn: &Connection, canonical: &str, display: &str) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        r#"INSERT INTO topic_dictionary (canonical, display, aliases, created_at)
           VALUES (?1, ?2, NULL, ?3)
           ON CONFLICT(canonical) DO NOTHING"#,
        params![canonical, display, now],
    )
    .map_err(|e| format!("upsert topic_dictionary: {e}"))?;
    Ok(())
}

/// 将别名并入 JSON 数组（幂等）
pub fn add_topic_alias(conn: &Connection, canonical: &str, alias: &str) -> Result<(), String> {
    if alias.is_empty() || alias == canonical {
        return Ok(());
    }
    // aliases 列为 SQL NULL 时必须按 Option<String> 读取，否则会报 Invalid column type Null
    let aliases_cell: Option<Option<String>> = conn
        .query_row(
            "SELECT aliases FROM topic_dictionary WHERE canonical = ?1",
            params![canonical],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| format!("read aliases: {e}"))?;
    let Some(aliases_opt) = aliases_cell else {
        return Ok(());
    };
    let mut arr: Vec<String> = aliases_opt
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    if !arr.iter().any(|a| a == alias) {
        arr.push(alias.to_string());
    }
    let json = serde_json::to_string(&arr).map_err(|e| format!("aliases json: {e}"))?;
    conn.execute(
        "UPDATE topic_dictionary SET aliases = ?1 WHERE canonical = ?2",
        params![json, canonical],
    )
    .map_err(|e| format!("update aliases: {e}"))?;
    Ok(())
}

/// 读取某笔记当前所有主题（合并重复 topic 取最大 confidence）
fn load_doc_topics_confidence_map(conn: &Connection, rel_path: &str) -> Result<HashMap<String, f64>, String> {
    let mut stmt = conn
        .prepare("SELECT topic, confidence FROM doc_topics WHERE rel_path = ?1")
        .map_err(|e| format!("load doc_topics: {e}"))?;
    let mut m: HashMap<String, f64> = HashMap::new();
    for row in stmt
        .query_map(params![rel_path], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
        .map_err(|e| format!("query doc_topics merge: {e}"))?
    {
        let (t, c) = row.map_err(|e| format!("row doc_topics merge: {e}"))?;
        m.entry(t).and_modify(|e| *e = e.max(c)).or_insert(c);
    }
    Ok(m)
}

pub fn list_dictionary_canonicals(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT canonical FROM topic_dictionary ORDER BY canonical")
        .map_err(|e| format!("list dictionary: {e}"))?;
    let out = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query dictionary: {e}"))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| format!("row dictionary: {e}"))?;
    Ok(out)
}

// --- Levenshtein（仅 ASCII 侧优化；UTF-32 字符级 DP，主题串通常很短） ---

fn levenshtein_utf32(a: &str, b: &str) -> usize {
    let a32: Vec<char> = a.chars().collect();
    let b32: Vec<char> = b.chars().collect();
    let n = a32.len();
    let m = b32.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = usize::from(a32[i - 1] != b32[j - 1]);
            curr[j] = (prev[j - 1] + cost)
                .min(prev[j] + 1)
                .min(curr[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// 小写 trim 后与词典合并：编辑距离 ≤2 则采用已有 canonical
pub fn normalize_topic(raw: &str, dictionary: &[String]) -> String {
    let t = raw.trim().to_lowercase();
    if t.is_empty() {
        return String::new();
    }
    for d in dictionary {
        let dc = d.trim().to_lowercase();
        if dc.is_empty() {
            continue;
        }
        if levenshtein_utf32(&t, &dc) <= 2 {
            return dc;
        }
    }
    t
}

/// 尝试用内置 embedding 将 `text` 与词典中短语对齐（余弦 > 0.9）
fn normalize_topic_with_embedding(
    raw: &str,
    dictionary: &[String],
    emb_model: &std::sync::Arc<crate::builtin_embed::BuiltinEmbedModel>,
) -> Result<String, String> {
    let base = normalize_topic(raw, dictionary);
    if dictionary.len() < 2 {
        return Ok(base);
    }
    let v_raw = crate::builtin_embed::encode_single(emb_model, raw.trim())?;
    let mut best: Option<(String, f32)> = None;
    for d in dictionary {
        let dtrim = d.trim();
        if dtrim.is_empty() {
            continue;
        }
        let v = crate::builtin_embed::encode_single(emb_model, dtrim)?;
        let sim = cosine_similarity(&v_raw, &v);
        if sim > 0.9 {
            if best.as_ref().map(|(_, s)| sim > *s).unwrap_or(true) {
                best = Some((dtrim.to_lowercase(), sim));
            }
        }
    }
    if let Some((canon, _)) = best {
        return Ok(canon);
    }
    Ok(base)
}

fn extract_json_object(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    let start = t.find('{').ok_or_else(|| "no JSON object in model output".to_string())?;
    let end = t.rfind('}').ok_or_else(|| "no JSON end in model output".to_string())?;
    if end <= start {
        return Err("invalid JSON span".to_string());
    }
    Ok(t[start..=end].to_string())
}

#[derive(Deserialize)]
struct TopicsJson {
    topics: Vec<String>,
}

const SYSTEM_TOPIC_EXTRACT: &str = r#"You extract 2-5 core topics from a personal knowledge note.
Output MUST be one JSON object only (no markdown fences, no prose outside JSON). Use exactly:
{ "topics": [ string, ... ] }
Rules:
- 2 to 5 short topics per note (noun phrases; prefer 2-4 Chinese characters each when the note is Chinese).
- Prefer reusing topics from the provided existing list when they fit; you may add at most ONE new topic not in the list if needed.
- Topics should be stable labels useful for grouping many notes (no full sentences).
"#;

fn resolve_ollama_model_name(ai: &AiConfig) -> Option<String> {
    ai.ollama
        .last_used_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let d = ai.ollama.default_model.trim();
            if d.is_empty() {
                None
            } else {
                Some(ai.ollama.default_model.clone())
            }
        })
}

fn markdown_body_and_headings(md: &str) -> (String, String) {
    let body = match split_frontmatter(md) {
        FrontmatterSplit::NoFence(s) | FrontmatterSplit::Unclosed(s) => s.to_string(),
        FrontmatterSplit::Closed { body, .. } => body.to_string(),
    };
    let mut headings = String::new();
    for line in body.lines() {
        if line.trim_start().starts_with('#') {
            if !headings.is_empty() {
                headings.push('\n');
            }
            let cap = line.chars().take(200).collect::<String>();
            headings.push_str(&cap);
        }
    }
    let excerpt: String = body.chars().take(EXCERPT_CHARS).collect();
    (excerpt, headings)
}

/// 对单篇文档提取核心主题（Ollama 非流式）；非 Ollama 提供商返回 Err
pub async fn extract_topics_for_document(
    doc_content_for_prompt: &str,
    existing_topics_list: &[String],
    ai: &AiConfig,
) -> Result<Vec<String>, String> {
    if ai.active_provider != ActiveProvider::Ollama {
        return Err("topic_extract_requires_ollama: switch AI provider to Ollama for topic extraction".to_string());
    }
    let model = resolve_ollama_model_name(ai).ok_or_else(|| "no Ollama model configured".to_string())?;
    let list_block = if existing_topics_list.is_empty() {
        "(none)".to_string()
    } else {
        existing_topics_list.join(", ")
    };
    let user_body = format!(
        "Existing topics (prefer these when applicable):\n{list_block}\n\nNote content (excerpt and headings):\n---\n{doc_content_for_prompt}\n---"
    );
    let timeout_ms = ai.request.timeout_ms.max(5000).min(120_000);
    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_TOPIC_EXTRACT.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".into(),
            content: user_body,
            ..Default::default()
        },
    ];
    let raw = ollama::run_chat_completion(
        &ai.ollama.base_url,
        &model,
        &msgs,
        ai.parameters.temperature.min(0.6),
        ai.parameters.top_p,
        timeout_ms,
    )
    .await?;
    let slice = extract_json_object(&raw)?;
    let parsed: TopicsJson = serde_json::from_str(&slice).map_err(|e| format!("topics JSON: {e}"))?;
    let mut out: Vec<String> = parsed
        .topics
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    if out.len() > MAX_TOPICS_PER_DOC {
        out.truncate(MAX_TOPICS_PER_DOC);
    }
    Ok(out)
}

// --- UI 载荷 ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicNode {
    pub id: String,
    pub name: String,
    pub doc_count: usize,
    pub related_topic_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocNode {
    pub rel_path: String,
    pub topic_count: usize,
    pub thought_count: usize,
    pub max_maturity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicDocEdge {
    pub topic_id: String,
    pub doc_rel_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicTopicEdge {
    pub source_topic_id: String,
    pub target_topic_id: String,
    pub weight: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicNetworkMeta {
    pub topic_node_cap: usize,
    pub doc_node_cap: usize,
    pub truncated_topic_count: usize,
    pub truncated_doc_count: usize,
    pub extract_skipped_no_llm: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicNetworkForUi {
    pub topic_nodes: Vec<TopicNode>,
    pub doc_nodes: Vec<DocNode>,
    pub topic_doc_edges: Vec<TopicDocEdge>,
    pub topic_topic_edges: Vec<TopicTopicEdge>,
    pub meta: TopicNetworkMeta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicCacheStatus {
    pub doc_topics_row_count: usize,
    pub dictionary_topic_count: usize,
    pub distinct_doc_paths: usize,
}

pub fn get_topic_cache_status(conn: &Connection) -> Result<TopicCacheStatus, String> {
    let doc_topics_row_count: usize = conn
        .query_row("SELECT COUNT(*) FROM doc_topics", [], |r| r.get(0))
        .unwrap_or(0);
    let dictionary_topic_count: usize = conn
        .query_row("SELECT COUNT(*) FROM topic_dictionary", [], |r| r.get(0))
        .unwrap_or(0);
    let distinct_doc_paths: usize = conn
        .query_row("SELECT COUNT(DISTINCT rel_path) FROM doc_topics", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(TopicCacheStatus {
        doc_topics_row_count,
        dictionary_topic_count,
        distinct_doc_paths,
    })
}

fn maturity_label_from_rank(rank: u8) -> &'static str {
    match rank {
        2 => "mature",
        1 => "growing",
        _ => "seedling",
    }
}

/// 从 DB 构建二部图；`topic_rows` 为 (rel_path, topic) 全量
fn build_ui_graph_from_rows(
    topic_rows: &[(String, String)],
    stats_map: &HashMap<String, (usize, u8)>,
) -> TopicNetworkForUi {
    let mut topic_to_docs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut doc_to_topics: HashMap<String, HashSet<String>> = HashMap::new();
    for (rel, topic) in topic_rows {
        topic_to_docs
            .entry(topic.clone())
            .or_default()
            .insert(rel.clone());
        doc_to_topics
            .entry(rel.clone())
            .or_default()
            .insert(topic.clone());
    }
    let valid_topics: HashSet<String> = topic_to_docs
        .iter()
        .filter(|(_, docs)| docs.len() >= TOPIC_IN_MIN_DOCS)
        .map(|(t, _)| t.clone())
        .collect();

    let mut doc_kept: HashSet<String> = HashSet::new();
    for (doc, tops) in &doc_to_topics {
        if tops.iter().any(|t| valid_topics.contains(t)) {
            doc_kept.insert(doc.clone());
        }
    }

    let mut topic_scores: Vec<(String, usize)> = valid_topics
        .iter()
        .map(|t| {
            let c = topic_to_docs.get(t).map(|s| s.len()).unwrap_or(0);
            (t.clone(), c)
        })
        .collect();
    topic_scores.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let truncated_topic_count = topic_scores.len().saturating_sub(MAX_TOPIC_NODES);
    topic_scores.truncate(MAX_TOPIC_NODES);
    let kept_topic_set: HashSet<String> = topic_scores.iter().map(|(t, _)| t.clone()).collect();

    let mut doc_degree: Vec<(String, usize)> = doc_kept
        .iter()
        .map(|d| {
            let deg = doc_to_topics
                .get(d)
                .map(|ts| ts.iter().filter(|t| kept_topic_set.contains(*t)).count())
                .unwrap_or(0);
            (d.clone(), deg)
        })
        .filter(|(_, deg)| *deg > 0)
        .collect();
    doc_degree.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let truncated_doc_count = doc_degree.len().saturating_sub(MAX_DOC_NODES);
    doc_degree.truncate(MAX_DOC_NODES);
    let kept_docs: HashSet<String> = doc_degree.iter().map(|(d, _)| d.clone()).collect();

    let mut cooc: HashMap<(String, String), usize> = HashMap::new();
    for doc in &kept_docs {
        let Some(ts) = doc_to_topics.get(doc) else {
            continue;
        };
        let mut list: Vec<String> = ts
            .iter()
            .filter(|t| kept_topic_set.contains(*t))
            .cloned()
            .collect();
        list.sort();
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let a = &list[i];
                let b = &list[j];
                let key = if a < b {
                    (a.clone(), b.clone())
                } else {
                    (b.clone(), a.clone())
                };
                *cooc.entry(key).or_insert(0) += 1;
            }
        }
    }

    let mut topic_topic_edges: Vec<TopicTopicEdge> = Vec::new();
    let mut related_count: HashMap<String, usize> = HashMap::new();
    for ((ta, tb), w) in &cooc {
        if *w >= TOPIC_COOC_MIN_DOCS {
            topic_topic_edges.push(TopicTopicEdge {
                source_topic_id: ta.clone(),
                target_topic_id: tb.clone(),
                weight: *w,
            });
            *related_count.entry(ta.clone()).or_insert(0) += 1;
            *related_count.entry(tb.clone()).or_insert(0) += 1;
        }
    }

    let topic_nodes: Vec<TopicNode> = topic_scores
        .iter()
        .map(|(name, dc)| TopicNode {
            id: name.clone(),
            name: name.clone(),
            doc_count: *dc,
            related_topic_count: *related_count.get(name).unwrap_or(&0),
        })
        .collect();

    let mut topic_doc_edges: Vec<TopicDocEdge> = Vec::new();
    let mut doc_nodes: Vec<DocNode> = Vec::new();
    for doc in kept_docs.iter() {
        let tops: Vec<String> = doc_to_topics
            .get(doc)
            .into_iter()
            .flat_map(|s| s.iter())
            .filter(|t| kept_topic_set.contains(*t))
            .cloned()
            .collect();
        if tops.is_empty() {
            continue;
        }
        for t in &tops {
            topic_doc_edges.push(TopicDocEdge {
                topic_id: t.clone(),
                doc_rel_path: doc.clone(),
            });
        }
        let (thought_count, max_rank) = stats_map.get(doc).copied().unwrap_or((0, 0));
        doc_nodes.push(DocNode {
            rel_path: doc.clone(),
            topic_count: tops.len(),
            thought_count,
            max_maturity: maturity_label_from_rank(max_rank).to_string(),
        });
    }
    doc_nodes.sort_by(|a, b| b.topic_count.cmp(&a.topic_count).then_with(|| a.rel_path.cmp(&b.rel_path)));

    TopicNetworkForUi {
        topic_nodes,
        doc_nodes,
        topic_doc_edges,
        topic_topic_edges,
        meta: TopicNetworkMeta {
            topic_node_cap: MAX_TOPIC_NODES,
            doc_node_cap: MAX_DOC_NODES,
            truncated_topic_count,
            truncated_doc_count,
            extract_skipped_no_llm: false,
        },
    }
}

/// 从 topic 库与想法统计构建当前 UI 图（供全量构建与「新增主题」后复用）
pub fn load_topic_network_graph(vault_root: &Path, topic_conn: &Connection) -> Result<TopicNetworkForUi, String> {
    let mut stmt = topic_conn
        .prepare("SELECT rel_path, topic FROM doc_topics")
        .map_err(|e| format!("scan doc_topics: {e}"))?;
    let topic_rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("rows doc_topics: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect doc_topics: {e}"))?;

    let stats_map: HashMap<String, (usize, u8)> = match vault_thoughts_db::open_thoughts_db(vault_root) {
        Ok(c) => vault_thoughts_db::graph_thought_stats(&c)
            .unwrap_or_default()
            .into_iter()
            .map(|(p, count, r)| (p, (count, r)))
            .collect(),
        Err(_) => HashMap::new(),
    };

    Ok(build_ui_graph_from_rows(&topic_rows, &stats_map))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddManualTopicResult {
    pub graph: TopicNetworkForUi,
    pub associated_doc_count: usize,
    pub canonical: String,
}

/// 用户新增主题：写入 `topic_dictionary`，对嵌入索引做查询向量检索，按文档聚合相似度后写回 `doc_topics`（model_id=`semantic-manual`），再构图返回前端。
pub fn add_manual_topic_semantic_blocking(
    vault_root: &Path,
    app: &AppHandle,
    display_raw: &str,
) -> Result<AddManualTopicResult, String> {
    let display = display_raw.trim();
    if display.is_empty() {
        return Err("主题名称不能为空".to_string());
    }
    let canonical = display.to_lowercase();
    if canonical.is_empty() {
        return Err("主题名称无效".to_string());
    }

    let topic_conn = open_topic_db(vault_root)?;
    upsert_topic_dictionary(&topic_conn, &canonical, display)?;

    let cache_dir = semantic_index::default_model_cache_dir();
    let bundle_dir = semantic_index::resolve_bundle_model_dir(app);
    let emb_model = semantic_index::get_cached_or_load_model(&cache_dir, &bundle_dir)?;

    let query_vec = crate::builtin_embed::encode_single(&emb_model, display)?;
    let emb_conn = semantic_index::open_embedding_db(vault_root)?;
    let chunks = semantic_index::load_all_doc_embeddings(&emb_conn)?;
    if chunks.is_empty() {
        return Err("语义索引中尚无文档向量，请先在设置中重建嵌入索引后再新增主题。".to_string());
    }

    let exclude: HashSet<String> = HashSet::new();
    let take = chunks.len().min(800);
    let hits = semantic_index::semantic_search_docs(&query_vec, &chunks, take, &exclude);
    let mut doc_best: HashMap<String, f32> = HashMap::new();
    for h in hits {
        let Some(rel) = h.rel_path else {
            continue;
        };
        let key = normalize_markdown_rel_path(&rel.replace('\\', "/"));
        if key.is_empty() {
            continue;
        }
        let score = h.score as f32;
        doc_best
            .entry(key)
            .and_modify(|e| *e = e.max(score))
            .or_insert(score);
    }
    doc_best.retain(|_, s| *s >= MANUAL_TOPIC_MIN_SEMANTIC);
    let mut scored: Vec<(String, f32)> = doc_best.into_iter().collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(MANUAL_TOPIC_MAX_DOCS);

    let mut associated = 0usize;
    for (rel_path, sem_score) in scored {
        let abs = vault_root.join(&rel_path);
        let bytes = match fs::read(&abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > READ_CAP {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if note_privacy::markdown_treat_as_kf_private(&text) {
            continue;
        }
        let hash = sha256_hex_file_stream(&abs)?;
        let conf = (sem_score as f64).min(1.0);
        let mut merged = load_doc_topics_confidence_map(&topic_conn, &rel_path)?;
        merged
            .entry(canonical.clone())
            .and_modify(|e| *e = e.max(conf))
            .or_insert(conf);

        let mut pairs: Vec<(String, f64)> = merged.into_iter().collect();
        pairs.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        pairs.truncate(MAX_TOPICS_PER_DOC);
        if !pairs.iter().any(|(t, _)| t == &canonical) {
            if pairs.len() >= MAX_TOPICS_PER_DOC {
                pairs.pop();
            }
            pairs.push((canonical.clone(), conf));
            pairs.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0))
            });
        }

        upsert_doc_topics(&topic_conn, &rel_path, &pairs, MODEL_ID_SEMANTIC_MANUAL, &hash)?;
        associated += 1;
    }

    let mut graph = load_topic_network_graph(vault_root, &topic_conn)?;
    graph.meta.extract_skipped_no_llm = false;
    Ok(AddManualTopicResult {
        graph,
        associated_doc_count: associated,
        canonical,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicMarkdownExportSummary {
    pub topics_written: usize,
    pub export_dir_rel: String,
}

fn topic_slug(canonical: &str) -> String {
    let safe: String = canonical
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let compact: String = safe.trim_matches('_').chars().take(48).collect();
    if compact.len() >= 2 && compact.chars().filter(|c| *c != '_').count() >= 2 {
        return compact;
    }
    let mut h = Sha256::new();
    h.update(canonical.as_bytes());
    format!("t_{}", &hex::encode(h.finalize())[..16])
}

/// 从 SQLite 导出 Markdown 快照（全量覆盖 export 子目录）
pub fn export_topic_index_markdown(vault_root: &Path, conn: &Connection) -> Result<TopicMarkdownExportSummary, String> {
    let export_root = topic_export_dir(vault_root);
    if export_root.exists() {
        fs::remove_dir_all(&export_root).map_err(|e| format!("remove export dir: {e}"))?;
    }
    fs::create_dir_all(export_root.join("by-topic")).map_err(|e| format!("create export dirs: {e}"))?;

    let readme = r"# Topic index export

This directory is **generated by KnowForge** from `.knowforge/topics/topic_cache.sqlite`.
The database is the source of truth; edit these files for reading only — changes are **not** synced back.

Re-export from the app anytime to refresh.
";
    fs::write(export_root.join("README.md"), readme).map_err(|e| format!("write README: {e}"))?;

    let mut topic_to_docs: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT rel_path, topic FROM doc_topics ORDER BY topic, rel_path")
        .map_err(|e| format!("export select: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("export query: {e}"))?;
    for r in rows {
        let (rel, topic): (String, String) = r.map_err(|e| format!("export row: {e}"))?;
        topic_to_docs.entry(topic).or_default().push(rel);
    }

    let displays: HashMap<String, String> = conn
        .prepare("SELECT canonical, display FROM topic_dictionary")
        .map_err(|e| format!("dict select: {e}"))?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("dict query: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("dict row: {e}"))?;

    let mut topics_sorted: Vec<String> = topic_to_docs.keys().cloned().collect();
    topics_sorted.sort();

    if topics_sorted.is_empty() {
        let empty_msg = "---\ntitle: Topic index\nsource: knowforge-topic-cache\n---\n\nNo topics in cache yet. Run topic extraction from the Topic graph panel.\n";
        fs::write(export_root.join("_index.md"), empty_msg).map_err(|e| format!("write empty index: {e}"))?;
        return Ok(TopicMarkdownExportSummary {
            topics_written: 0,
            export_dir_rel: ".knowforge/topics/export".to_string(),
        });
    }

    let mut index_lines = vec![
        "---".to_string(),
        "title: Topic index".to_string(),
        format!("exported_at: {}", Utc::now().to_rfc3339()),
        "source: knowforge-topic-cache".to_string(),
        "---".to_string(),
        String::new(),
        "## All topics".to_string(),
        String::new(),
    ];
    for t in &topics_sorted {
        let slug = topic_slug(t);
        let disp = displays.get(t).cloned().unwrap_or_else(|| t.clone());
        index_lines.push(format!("- [[by-topic/{}.md|{}]]", slug, disp));
    }
    fs::write(export_root.join("_index.md"), index_lines.join("\n"))
        .map_err(|e| format!("write _index: {e}"))?;

    let mut written = 0usize;
    for t in &topics_sorted {
        let docs = topic_to_docs.get(t).cloned().unwrap_or_default();
        let slug = topic_slug(t);
        let disp = displays.get(t).cloned().unwrap_or_else(|| t.clone());
        let mut body = vec![
            "---".to_string(),
            format!("canonical: {}", serde_json::to_string(t).unwrap_or_else(|_| "\"\"".into())),
            format!("display: {}", serde_json::to_string(&disp).unwrap_or_else(|_| "\"\"".into())),
            format!("exported_at: {}", Utc::now().to_rfc3339()),
            format!("doc_count: {}", docs.len()),
            "source: knowforge-topic-cache".to_string(),
            "---".to_string(),
            String::new(),
        ];
        if docs.is_empty() {
            body.push("(no documents)".to_string());
        } else {
            for d in &docs {
                let link = format!("[[{}]]", d.replace('\\', "/"));
                body.push(format!("- {link}"));
            }
        }
        fs::write(export_root.join("by-topic").join(format!("{slug}.md")), body.join("\n"))
            .map_err(|e| format!("write topic md: {e}"))?;
        written += 1;
    }

    Ok(TopicMarkdownExportSummary {
        topics_written: written,
        export_dir_rel: ".knowforge/topics/export".to_string(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TopicExtractProgressPayload {
    current: u32,
    total: u32,
}

/// 全量：扫描 vault、增量提取、构图
pub async fn build_topic_network(vault_root: &Path, app: &AppHandle) -> Result<TopicNetworkForUi, String> {
    let ai = crate::vault_config::load_ai_config_internal(vault_root)?;
    let ollama_ok = ai.active_provider == ActiveProvider::Ollama && resolve_ollama_model_name(&ai).is_some();

    let topic_conn = open_topic_db(vault_root)?;
    let mut paths: Vec<PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(vault_root, vault_root, &mut paths, MAX_FILES)?;

    let cache_dir = semantic_index::default_model_cache_dir();
    let bundle_dir = semantic_index::resolve_bundle_model_dir(app);
    let emb_model = if ollama_ok {
        semantic_index::get_cached_or_load_model(&cache_dir, &bundle_dir).ok()
    } else {
        None
    };

    let total = paths.len().max(1) as u32;
    let mut current_idx = 0u32;

    for abs in &paths {
        current_idx += 1;
        let _ = app.emit(
            "topic:extract-progress",
            TopicExtractProgressPayload {
                current: current_idx,
                total: total.max(current_idx),
            },
        );

        let Some(rel_path) = vault_context_search::rel_path_from_root(vault_root, abs) else {
            continue;
        };
        let rel_path = normalize_markdown_rel_path(&rel_path.replace('\\', "/"));
        if rel_path.is_empty() {
            continue;
        }

        let bytes = match fs::read(abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > READ_CAP {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if note_privacy::markdown_treat_as_kf_private(&text) {
            continue;
        }

        let hash = sha256_hex_file_stream(abs)?;
        if let Some(cached) = get_cached_topics(&topic_conn, &rel_path, &hash)? {
            if cached.len() >= MIN_TOPICS_PER_DOC {
                continue;
            }
        }

        if !ollama_ok {
            continue;
        }

        let mut dict_vec = list_dictionary_canonicals(&topic_conn).unwrap_or_default();
        let (excerpt, headings) = markdown_body_and_headings(&text);
        let prompt_body = format!("## Outline / headings\n{headings}\n\n## Body excerpt\n{excerpt}");
        let raw_topics = match extract_topics_for_document(&prompt_body, &dict_vec, &ai).await {
            Ok(t) => t,
            Err(_) => continue,
        };
        if raw_topics.len() < MIN_TOPICS_PER_DOC {
            continue;
        }
        let model_id = resolve_ollama_model_name(&ai)
            .unwrap_or_else(|| MODEL_ID_FALLBACK.to_string());

        let mut normalized: Vec<(String, f64)> = Vec::new();
        let mut new_aliases: Vec<(String, String)> = Vec::new();
        let mut new_canonicals: Vec<(String, String)> = Vec::new();

        for raw in &raw_topics {
            let canon = if let Some(ref m) = emb_model {
                normalize_topic_with_embedding(raw, &dict_vec, m).unwrap_or_else(|_| normalize_topic(raw, &dict_vec))
            } else {
                normalize_topic(raw, &dict_vec)
            };
            if canon.is_empty() {
                continue;
            }
            let raw_l = raw.trim().to_lowercase();
            if raw_l != canon {
                new_aliases.push((canon.clone(), raw.trim().to_string()));
            }
            if !dict_vec.iter().any(|x| x == &canon) && !new_canonicals.iter().any(|(c, _)| c == &canon) {
                let display = raw.trim().to_string();
                new_canonicals.push((canon.clone(), display));
                dict_vec.push(canon.clone());
            }
            normalized.push((canon, 1.0));
        }
        normalized.sort_by(|a, b| a.0.cmp(&b.0));
        normalized.dedup_by(|a, b| a.0 == b.0);

        if normalized.len() < MIN_TOPICS_PER_DOC {
            continue;
        }

        for (c, d) in &new_canonicals {
            upsert_topic_dictionary(&topic_conn, c, d)?;
        }
        for (c, alias) in &new_aliases {
            add_topic_alias(&topic_conn, c, alias)?;
        }

        upsert_doc_topics(&topic_conn, &rel_path, &normalized, &model_id, &hash)?;
    }

    let mut graph = load_topic_network_graph(vault_root, &topic_conn)?;
    graph.meta.extract_skipped_no_llm = !ollama_ok;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init_topic_schema(&c).unwrap();
        c
    }

    #[test]
    fn cache_hit_when_hash_matches() {
        let c = mem_conn();
        upsert_doc_topics(
            &c,
            "a.md",
            &[("rust".into(), 1.0), ("async".into(), 1.0)],
            "m1",
            "abc123",
        )
        .unwrap();
        let got = get_cached_topics(&c, "a.md", "abc123").unwrap();
        assert!(got.is_some());
        let mut v = got.unwrap();
        v.sort();
        assert_eq!(v, vec!["async".to_string(), "rust".to_string()]);
    }

    #[test]
    fn cache_miss_when_hash_differs() {
        let c = mem_conn();
        upsert_doc_topics(&c, "a.md", &[("t".into(), 1.0)], "m1", "old").unwrap();
        assert!(get_cached_topics(&c, "a.md", "new").unwrap().is_none());
    }

    #[test]
    fn normalize_topic_merge_close() {
        let dict = vec!["machine learning".to_string()];
        assert_eq!(normalize_topic("Machine Learning", &dict), "machine learning");
        assert_eq!(normalize_topic("machine learnin", &dict), "machine learning");
    }

    #[test]
    fn graph_filters_single_doc_topics() {
        let rows = vec![
            ("a.md".into(), "x".into()),
            ("b.md".into(), "y".into()),
            ("c.md".into(), "z".into()),
        ];
        let g = build_ui_graph_from_rows(&rows, &HashMap::new());
        assert!(g.topic_nodes.is_empty());
    }

    #[test]
    fn graph_keeps_cooccurring_topics() {
        let rows = vec![
            ("a.md".into(), "t1".into()),
            ("b.md".into(), "t1".into()),
            ("a.md".into(), "t2".into()),
            ("b.md".into(), "t2".into()),
        ];
        let g = build_ui_graph_from_rows(&rows, &HashMap::new());
        assert_eq!(g.topic_nodes.len(), 2);
        assert!(g.doc_nodes.len() >= 1);
    }

    #[test]
    fn add_topic_alias_when_aliases_column_null() {
        let c = mem_conn();
        upsert_topic_dictionary(&c, "rust", "Rust").unwrap();
        c.execute("UPDATE topic_dictionary SET aliases = NULL WHERE canonical = 'rust'", [])
            .unwrap();
        add_topic_alias(&c, "rust", "rs").unwrap();
        let json: String = c
            .query_row("SELECT aliases FROM topic_dictionary WHERE canonical = 'rust'", [], |row| {
                row.get::<_, Option<String>>(0)
            })
            .unwrap()
            .unwrap_or_default();
        let arr: Vec<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(arr, vec!["rs".to_string()]);
    }
}
