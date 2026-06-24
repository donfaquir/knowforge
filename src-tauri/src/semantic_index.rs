//! 语义索引：SQLite 向量侧车、Markdown 分块、增量重建与检索（迭代 6.2）。

use crate::builtin_embed::{self, encode_batch, encode_single, BuiltinEmbedModel};
use crate::rebuild_progress;
use crate::note_privacy;
use crate::vault_config::{self, SemanticConfig};
use crate::vault_context_search;
use crate::vault_thoughts_db;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

/// 重建索引阶段日志（`cargo tauri dev` 终端可见）
fn semantic_rebuild_log(msg: &str) {
    eprintln!("[semantic_rebuild] {msg}");
}

// --- 模型进程内缓存（避免每次对话重复 mmap）---

static EMBED_MODEL_CACHE: Mutex<Option<(String, Arc<BuiltinEmbedModel>)>> = Mutex::new(None);

pub fn get_cached_or_load_model(cache_dir: &Path, bundle_dir: &Path) -> Result<Arc<BuiltinEmbedModel>, String> {
    let model_dir = builtin_embed::resolve_model_dir(cache_dir, bundle_dir)?;
    let key = model_dir.to_string_lossy().to_string();
    let mut g = EMBED_MODEL_CACHE
        .lock()
        .map_err(|_| "model cache lock poisoned".to_string())?;
    if let Some((ref k, ref m)) = *g {
        if k == &key {
            return Ok(Arc::clone(m));
        }
    }
    let loaded = Arc::new(builtin_embed::load_model(&model_dir)?);
    *g = Some((key, Arc::clone(&loaded)));
    Ok(loaded)
}

// --- 路径与 SQLite ---

pub fn embedding_db_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".knowforge/semantic/embeddings.sqlite")
}

pub fn open_embedding_db(vault_root: &Path) -> Result<Connection, String> {
    let path = embedding_db_path(vault_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create semantic dir: {e}"))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("open embeddings db: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("wal: {e}"))?;
    init_embedding_schema(&conn)?;
    Ok(conn)
}

fn init_embedding_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS doc_chunks (
            chunk_id TEXT PRIMARY KEY,
            rel_path TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            chunk_text TEXT NOT NULL,
            embedding BLOB NOT NULL,
            dim INTEGER NOT NULL,
            model_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            UNIQUE(rel_path, chunk_index)
        );
        CREATE INDEX IF NOT EXISTS idx_chunks_rel ON doc_chunks(rel_path);

        CREATE TABLE IF NOT EXISTS thought_embeddings (
            thought_id TEXT PRIMARY KEY,
            embedding BLOB NOT NULL,
            dim INTEGER NOT NULL,
            model_id TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS file_hashes (
            rel_path TEXT PRIMARY KEY,
            sha256_hex TEXT NOT NULL,
            chunk_count INTEGER NOT NULL,
            indexed_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("init embedding schema: {e}"))?;
    Ok(())
}

fn f32_slice_to_blob(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

fn blob_to_f32_slice(blob: &[u8], dim: usize) -> Result<Vec<f32>, String> {
    if blob.len() != dim * 4 {
        return Err(format!(
            "embedding blob len {} expected {} for dim {}",
            blob.len(),
            dim * 4,
            dim
        ));
    }
    let mut out = Vec::with_capacity(dim);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

fn sha256_hex_bytes(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// 流式哈希，避免超大文件再分配一整块 `Vec<u8>`
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

fn sha256_hex_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read file: {e}"))?;
    Ok(sha256_hex_bytes(&bytes))
}

// --- 行类型 ---

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DocChunkRow {
    pub chunk_id: String,
    pub rel_path: String,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub embedding: Vec<f32>,
    pub dim: i32,
    pub model_id: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ThoughtEmbeddingRow {
    pub thought_id: String,
    pub embedding: Vec<f32>,
    pub dim: i32,
    pub model_id: String,
}

pub struct EmbeddingCache {
    docs: RwLock<Option<(u64, Arc<Vec<DocChunkRow>>)>>,
    thoughts: RwLock<Option<(u64, Arc<Vec<ThoughtEmbeddingRow>>)>>,
    generation: AtomicU64,
}

impl EmbeddingCache {
    pub fn new() -> Self {
        Self {
            docs: RwLock::new(None),
            thoughts: RwLock::new(None),
            generation: AtomicU64::new(0),
        }
    }

    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_docs(&self, conn: &Connection) -> Arc<Vec<DocChunkRow>> {
        let current_gen = self.generation.load(Ordering::Relaxed);
        {
            let guard = self.docs.read().unwrap();
            if let Some((cached_gen, ref data)) = *guard {
                if cached_gen == current_gen {
                    return Arc::clone(data);
                }
            }
        }
        let mut guard = self.docs.write().unwrap();
        if let Some((cached_gen, ref data)) = *guard {
            if cached_gen == current_gen {
                return Arc::clone(data);
            }
        }
        let rows = Arc::new(load_all_doc_embeddings(conn).unwrap_or_default());
        *guard = Some((current_gen, Arc::clone(&rows)));
        rows
    }

    pub fn get_thoughts(&self, conn: &Connection) -> Arc<Vec<ThoughtEmbeddingRow>> {
        let current_gen = self.generation.load(Ordering::Relaxed);
        {
            let guard = self.thoughts.read().unwrap();
            if let Some((cached_gen, ref data)) = *guard {
                if cached_gen == current_gen {
                    return Arc::clone(data);
                }
            }
        }
        let mut guard = self.thoughts.write().unwrap();
        if let Some((cached_gen, ref data)) = *guard {
            if cached_gen == current_gen {
                return Arc::clone(data);
            }
        }
        let rows = Arc::new(load_all_thought_embeddings(conn).unwrap_or_default());
        *guard = Some((current_gen, Arc::clone(&rows)));
        rows
    }
}

pub fn upsert_doc_chunk(
    conn: &Connection,
    chunk_id: &str,
    rel_path: &str,
    chunk_index: i32,
    chunk_text: &str,
    embedding: &[f32],
    model_id: &str,
) -> Result<(), String> {
    let blob = f32_slice_to_blob(embedding);
    let dim = embedding.len() as i32;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        r#"INSERT INTO doc_chunks (chunk_id, rel_path, chunk_index, chunk_text, embedding, dim, model_id, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
           ON CONFLICT(chunk_id) DO UPDATE SET
             chunk_text = excluded.chunk_text,
             embedding = excluded.embedding,
             dim = excluded.dim,
             model_id = excluded.model_id,
             created_at = excluded.created_at"#,
        params![
            chunk_id,
            rel_path,
            chunk_index,
            chunk_text,
            blob,
            dim,
            model_id,
            now
        ],
    )
    .map_err(|e| format!("upsert doc chunk: {e}"))?;
    Ok(())
}

pub fn upsert_thought_embedding(
    conn: &Connection,
    thought_id: &str,
    embedding: &[f32],
    model_id: &str,
    updated_at: &str,
) -> Result<(), String> {
    let blob = f32_slice_to_blob(embedding);
    let dim = embedding.len() as i32;
    conn.execute(
        r#"INSERT INTO thought_embeddings (thought_id, embedding, dim, model_id, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5)
           ON CONFLICT(thought_id) DO UPDATE SET
             embedding = excluded.embedding,
             dim = excluded.dim,
             model_id = excluded.model_id,
             updated_at = excluded.updated_at"#,
        params![thought_id, blob, dim, model_id, updated_at],
    )
    .map_err(|e| format!("upsert thought embedding: {e}"))?;
    Ok(())
}

pub fn get_file_hash(conn: &Connection, rel_path: &str) -> Result<Option<(String, i32)>, String> {
    let mut stmt = conn
        .prepare("SELECT sha256_hex, chunk_count FROM file_hashes WHERE rel_path = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map(params![rel_path], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?)))
        .map_err(|e| e.to_string())?;
    if let Some(r) = rows.next() {
        let (h, c) = r.map_err(|e| e.to_string())?;
        return Ok(Some((h, c)));
    }
    Ok(None)
}

pub fn set_file_hash(
    conn: &Connection,
    rel_path: &str,
    sha256_hex: &str,
    chunk_count: i32,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        r#"INSERT INTO file_hashes (rel_path, sha256_hex, chunk_count, indexed_at)
           VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT(rel_path) DO UPDATE SET
             sha256_hex = excluded.sha256_hex,
             chunk_count = excluded.chunk_count,
             indexed_at = excluded.indexed_at"#,
        params![rel_path, sha256_hex, chunk_count, now],
    )
    .map_err(|e| format!("set file hash: {e}"))?;
    Ok(())
}

pub fn delete_chunks_for_path(conn: &Connection, rel_path: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM doc_chunks WHERE rel_path = ?1",
        params![rel_path],
    )
    .map_err(|e| format!("delete chunks: {e}"))?;
    conn.execute("DELETE FROM file_hashes WHERE rel_path = ?1", params![rel_path])
        .map_err(|e| format!("delete file hash: {e}"))?;
    Ok(())
}

pub fn load_all_doc_embeddings(conn: &Connection) -> Result<Vec<DocChunkRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT chunk_id, rel_path, chunk_index, chunk_text, embedding, dim, model_id FROM doc_chunks",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let blob: Vec<u8> = row.get(4).map_err(|e| e.to_string())?;
        let dim: i32 = row.get(5).map_err(|e| e.to_string())?;
        let emb = blob_to_f32_slice(&blob, dim as usize)?;
        out.push(DocChunkRow {
            chunk_id: row.get(0).map_err(|e| e.to_string())?,
            rel_path: row.get(1).map_err(|e| e.to_string())?,
            chunk_index: row.get(2).map_err(|e| e.to_string())?,
            chunk_text: row.get(3).map_err(|e| e.to_string())?,
            embedding: emb,
            dim,
            model_id: row.get(6).map_err(|e| e.to_string())?,
        });
    }
    Ok(out)
}

pub fn load_all_thought_embeddings(conn: &Connection) -> Result<Vec<ThoughtEmbeddingRow>, String> {
    let mut stmt = conn
        .prepare("SELECT thought_id, embedding, dim, model_id FROM thought_embeddings")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let blob: Vec<u8> = row.get(1).map_err(|e| e.to_string())?;
        let dim: i32 = row.get(2).map_err(|e| e.to_string())?;
        let emb = blob_to_f32_slice(&blob, dim as usize)?;
        out.push(ThoughtEmbeddingRow {
            thought_id: row.get(0).map_err(|e| e.to_string())?,
            embedding: emb,
            dim,
            model_id: row.get(3).map_err(|e| e.to_string())?,
        });
    }
    Ok(out)
}

// --- 分块 ---

pub mod chunk {
    use super::*;

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ChunkInfo {
        pub index: usize,
        pub text: String,
        pub heading_context: Option<String>,
    }

    fn is_heading_line(line: &str) -> Option<String> {
        let t = line.trim_start();
        if !t.starts_with('#') {
            return None;
        }
        let rest = t.trim_start_matches('#');
        let level = t.len() - rest.len();
        if !(1..=3).contains(&level) {
            return None;
        }
        let title = rest.trim().trim_start_matches('#').trim();
        if title.is_empty() {
            return None;
        }
        Some(title.to_string())
    }

    fn truncate_chars(s: &str, max: usize) -> String {
        if s.chars().count() <= max {
            return s.to_string();
        }
        let mut out = String::new();
        for (i, ch) in s.chars().enumerate() {
            if i >= max {
                break;
            }
            out.push(ch);
        }
        out
    }

    /// 按 #/##/### 分节；超长节按空行分段并保留段间 1 段重叠；单块上限 `max_chunk_chars` Unicode 字符
    pub fn split_markdown_into_chunks(markdown: &str, max_chunk_chars: usize) -> Vec<ChunkInfo> {
        let max_chunk_chars = max_chunk_chars.max(256);
        let lines: Vec<&str> = markdown.lines().collect();
        let mut sections: Vec<(Option<String>, String)> = Vec::new();
        let mut cur_heading: Option<String> = None;
        let mut buf = String::new();

        for line in lines {
            if let Some(h) = is_heading_line(line) {
                if !buf.trim().is_empty() || cur_heading.is_some() {
                    let body = std::mem::take(&mut buf);
                    sections.push((cur_heading.take(), body));
                }
                cur_heading = Some(h);
                continue;
            }
            buf.push_str(line);
            buf.push('\n');
        }
        if !buf.trim().is_empty() || cur_heading.is_some() {
            sections.push((cur_heading.take(), buf));
        }
        if sections.is_empty() {
            let t = markdown.trim();
            if t.is_empty() {
                return Vec::new();
            }
            sections.push((None, markdown.to_string()));
        }

        let mut flat: Vec<(Option<String>, String)> = Vec::new();
        for (h, body) in sections {
            let body = body.trim().to_string();
            if body.is_empty() {
                continue;
            }
            if body.chars().count() <= max_chunk_chars {
                flat.push((h, body));
            } else {
                let paras: Vec<&str> = body.split("\n\n").collect();
                if paras.len() <= 1 {
                    flat.push((h, truncate_chars(&body, max_chunk_chars)));
                    continue;
                }
                let mut i = 0usize;
                while i < paras.len() {
                    let mut piece = String::new();
                    let start_i = i;
                    while i < paras.len() {
                        let next = paras[i].trim();
                        if next.is_empty() {
                            i += 1;
                            continue;
                        }
                        let add = if piece.is_empty() {
                            next.to_string()
                        } else {
                            format!("\n\n{next}")
                        };
                        if piece.chars().count() + add.chars().count() > max_chunk_chars {
                            break;
                        }
                        piece.push_str(&add);
                        i += 1;
                    }
                    if piece.is_empty() {
                        piece = truncate_chars(paras[i], max_chunk_chars);
                        i += 1;
                    }
                    flat.push((h.clone(), truncate_chars(&piece, max_chunk_chars)));
                    if i >= paras.len() {
                        break;
                    }
                    // 仅当本轮至少前进了 2 个段落时才回退 1 段做重叠；否则 `i` 会回到 `start_i` 导致死循环与内存暴涨
                    if start_i + 1 < i {
                        let overlap = paras[i.saturating_sub(1)].trim();
                        if !overlap.is_empty() && i < paras.len() {
                            // 回退一段以形成重叠（文档：保留上一 chunk 末段）
                            i = i.saturating_sub(1);
                        }
                    }
                }
            }
        }

        flat.into_iter()
            .enumerate()
            .map(|(idx, (heading_context, text))| ChunkInfo {
                index: idx,
                text,
                heading_context,
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn empty_md() {
            assert!(split_markdown_into_chunks("", 2048).is_empty());
        }

        #[test]
        fn headings_split() {
            let md = "# A\nx\n\n## B\ny";
            let c = split_markdown_into_chunks(md, 2048);
            assert!(c.len() >= 2);
        }
    }
}

// --- 向量检索 ---

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0f32;
    let mut na = 0f32;
    let mut nb = 0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let d = na.sqrt() * nb.sqrt();
    if d < 1e-12 {
        return 0.0;
    }
    dot / d
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchHit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_id: Option<String>,
    pub chunk_text: String,
    pub score: f64,
    pub source_type: String,
}

pub fn semantic_search_docs(
    query_embedding: &[f32],
    all_chunks: &[DocChunkRow],
    top_k: usize,
    exclude_paths: &HashSet<String>,
) -> Vec<SemanticSearchHit> {
    let mut scored: Vec<(f32, &DocChunkRow)> = all_chunks
        .iter()
        .filter(|r| !exclude_paths.contains(&r.rel_path.replace('\\', "/")))
        .map(|r| (cosine_similarity(query_embedding, &r.embedding), r))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(top_k)
        .map(|(s, r)| SemanticSearchHit {
            rel_path: Some(r.rel_path.clone()),
            thought_id: None,
            chunk_text: r.chunk_text.clone(),
            score: s as f64,
            source_type: "doc".to_string(),
        })
        .collect()
}

pub fn semantic_search_thoughts(
    query_embedding: &[f32],
    all: &[ThoughtEmbeddingRow],
    top_k: usize,
) -> Vec<SemanticSearchHit> {
    let mut scored: Vec<(f32, &ThoughtEmbeddingRow)> = all
        .iter()
        .map(|r| (cosine_similarity(query_embedding, &r.embedding), r))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(top_k)
        .map(|(s, r)| SemanticSearchHit {
            rel_path: None,
            thought_id: Some(r.thought_id.clone()),
            chunk_text: String::new(),
            score: s as f64,
            source_type: "thought".to_string(),
        })
        .collect()
}

/// RRF 加权融合；`id` 为稳定键（文档 `rel_path`，想法 `thought:{id}`）
/// 供后续链接推荐等复用；当前 LLM 融合使用 `reciprocal_rank_fusion_owned`
#[allow(dead_code)]
pub fn reciprocal_rank_fusion(
    keyword_results: &[(&str, f64)],
    semantic_results: &[(&str, f64)],
    keyword_weight: f64,
    semantic_weight: f64,
    top_k: usize,
) -> Vec<(String, f64)> {
    let k = 60.0;
    let mut acc: HashMap<String, f64> = HashMap::new();
    for (rank, (id, _)) in keyword_results.iter().enumerate() {
        *acc.entry((*id).to_string()).or_default() += keyword_weight / (k + rank as f64 + 1.0);
    }
    for (rank, (id, _)) in semantic_results.iter().enumerate() {
        *acc.entry((*id).to_string()).or_default() += semantic_weight / (k + rank as f64 + 1.0);
    }
    let mut v: Vec<(String, f64)> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(top_k);
    v
}

// --- 路径解析 ---

pub fn default_model_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("knowforge")
        .join("models")
        .join(builtin_embed::DEFAULT_MODEL_ID)
}

pub fn resolve_bundle_model_dir(app: &AppHandle) -> PathBuf {
    if let Ok(rd) = app.path().resource_dir() {
        let p = rd.join("models").join(builtin_embed::DEFAULT_MODEL_ID);
        if builtin_embed::model_dir_ready(&p) {
            return p;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/models").join(builtin_embed::DEFAULT_MODEL_ID)
}

// --- 状态与重建 ---

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub model_ready: bool,
    pub model_id: String,
    pub doc_chunk_count: usize,
    pub thought_embedding_count: usize,
    pub tracked_file_count: usize,
    pub stale_file_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexBuildResult {
    pub indexed_chunks: usize,
    pub indexed_thoughts: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchArgs {
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// `docs` | `thoughts` | `all`
    #[serde(default)]
    pub search_scope: Option<String>,
}

fn default_top_k() -> usize {
    8
}

fn list_thought_rows_for_embed(conn: &rusqlite::Connection) -> Result<Vec<(String, String, String, i64)>, String> {
    let mut stmt = conn
        .prepare("SELECT thought_id, body, updated_at, standalone FROM thoughts")
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in iter {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn list_thought_note_paths(conn: &rusqlite::Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT thought_id, note_rel_path FROM thoughts WHERE standalone = 0")
        .map_err(|e| e.to_string())?;
    let iter = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut m = HashMap::new();
    for r in iter {
        let (id, p) = r.map_err(|e| e.to_string())?;
        m.insert(id, p);
    }
    Ok(m)
}

pub fn get_index_status(vault_root: &Path, cache_dir: &Path, bundle_dir: &Path) -> Result<IndexStatus, String> {
    let model_ready = builtin_embed::is_model_ready(cache_dir, bundle_dir);
    let conn = open_embedding_db(vault_root)?;
    let doc_chunk_count: usize = conn
        .query_row("SELECT COUNT(*) FROM doc_chunks", [], |r| r.get(0))
        .unwrap_or(0);
    let thought_embedding_count: usize = conn
        .query_row("SELECT COUNT(*) FROM thought_embeddings", [], |r| r.get(0))
        .unwrap_or(0);
    let tracked_file_count: usize = conn
        .query_row("SELECT COUNT(*) FROM file_hashes", [], |r| r.get(0))
        .unwrap_or(0);

    let mut stale = 0usize;
    let mut stmt = conn
        .prepare("SELECT rel_path, sha256_hex FROM file_hashes")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for r in rows {
        let (rel, old_hash) = r.map_err(|e| e.to_string())?;
        let joined = crate::join_under_root(vault_root, &rel).ok();
        if let Some(j) = joined {
            if j.is_file() && crate::is_markdown_path(&j) {
                if let Ok(h) = sha256_hex_file(&j) {
                    if h != old_hash {
                        stale += 1;
                    }
                    continue;
                }
            }
        }
        stale += 1;
    }

    Ok(IndexStatus {
        model_ready,
        model_id: builtin_embed::DEFAULT_MODEL_ID.to_string(),
        doc_chunk_count,
        thought_embedding_count,
        tracked_file_count,
        stale_file_count: stale,
    })
}

const INDEX_BATCH: usize = 16;
const WALK_MAX_FILES: usize = 50_000;
/// 单文件全文读入上限（超过则只记流式 hash、不写向量，避免数 GB 单文件 OOM）
const MAX_INDEX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// 单篇最多 chunk 数（分块器在病态输入下可能产生极多块或长时间循环）
const MAX_DOC_CHUNKS: usize = 50_000;

/// 供前端读取上次/当前重建进度（落盘 JSON）
pub fn read_embedding_rebuild_progress(vault_root: &Path) -> Option<rebuild_progress::RebuildProgress> {
    rebuild_progress::read_rebuild_progress(vault_root)
}

fn validate_resume_checkpoint(cp: &rebuild_progress::RebuildProgress, md_len: usize) -> Result<(), String> {
    if cp.phase == "completed" {
        return Err("rebuild checkpoint is already completed; start a full rebuild instead".to_string());
    }
    if cp.docs_total != md_len {
        return Err(format!(
            "Markdown file count changed (checkpoint {}, current {}); run a full rebuild",
            cp.docs_total, md_len
        ));
    }
    Ok(())
}

fn flush_docs_rebuild_progress(
    vault_root: &Path,
    app: &AppHandle,
    rp: &mut rebuild_progress::RebuildProgress,
    fi: usize,
    msg: &str,
) -> Result<(), String> {
    rp.docs_completed = fi + 1;
    rp.phase = "documents".to_string();
    rp.last_message = Some(msg.to_string());
    rp.updated_at = chrono::Utc::now().to_rfc3339();
    rebuild_progress::write_rebuild_progress(vault_root, rp)?;
    rebuild_progress::emit_checkpoint(app, rp);
    Ok(())
}

pub fn rebuild_index(vault_root: &Path, app: &AppHandle, resume: bool) -> Result<IndexBuildResult, String> {
    let out = rebuild_index_impl(vault_root, app, resume);
    if let Err(e) = &out {
        if let Some(mut rp) = rebuild_progress::read_rebuild_progress(vault_root) {
            if rp.phase != "completed" {
                rp.mark_failed(e);
                let _ = rebuild_progress::write_rebuild_progress(vault_root, &rp);
                rebuild_progress::emit_checkpoint(app, &rp);
            }
        }
    }
    out.inspect_err(|e| semantic_rebuild_log(&format!("FAILED: {e}")))
}

fn rebuild_index_impl(vault_root: &Path, app: &AppHandle, resume: bool) -> Result<IndexBuildResult, String> {
    let t0 = std::time::Instant::now();
    semantic_rebuild_log(&format!("rebuild_index: start resume={resume}"));
    let cache_dir = default_model_cache_dir();
    let bundle_dir = resolve_bundle_model_dir(app);
    let model = get_cached_or_load_model(&cache_dir, &bundle_dir)?;
    let model_id = builtin_embed::DEFAULT_MODEL_ID;
    let dim = model.dim;

    let mut md_paths = Vec::new();
    vault_context_search::walk_markdown_files(vault_root, vault_root, &mut md_paths, WALK_MAX_FILES)
        .map_err(|e| e.to_string())?;
    md_paths.sort_by(|a, b| {
        let ra = vault_context_search::rel_path_from_root(vault_root, a).unwrap_or_default();
        let rb = vault_context_search::rel_path_from_root(vault_root, b).unwrap_or_default();
        ra.cmp(&rb)
    });
    let n_md = md_paths.len();

    let mut rp = if resume {
        let mut p = rebuild_progress::read_rebuild_progress(vault_root)
            .ok_or_else(|| "no rebuild_progress.json to resume".to_string())?;
        validate_resume_checkpoint(&p, n_md)?;
        p.last_error = None;
        p.updated_at = chrono::Utc::now().to_rfc3339();
        rebuild_progress::write_rebuild_progress(vault_root, &p)?;
        rebuild_progress::emit_checkpoint(app, &p);
        p
    } else {
        let p = rebuild_progress::RebuildProgress::new_session(Uuid::new_v4().to_string());
        rebuild_progress::write_rebuild_progress(vault_root, &p)?;
        rebuild_progress::emit_checkpoint(app, &p);
        p
    };

    let _ = app.emit(
        "semantic:index-progress",
        serde_json::json!({
            "phase": "scan",
            "current": 0,
            "total": 0,
            "message": "正在扫描 Markdown 文件…",
        }),
    );

    rp.docs_total = n_md;
    // 续建时不得覆盖 checkpoint 的 phase（例如 thoughts），否则 skip_doc_loop 判断失效、想法进度会被清零
    if !resume {
        rp.phase = "documents".to_string();
    }
    rp.updated_at = chrono::Utc::now().to_rfc3339();
    rebuild_progress::write_rebuild_progress(vault_root, &rp)?;
    rebuild_progress::emit_checkpoint(app, &rp);

    let _ = app.emit(
        "semantic:index-progress",
        serde_json::json!({
            "phase": "scan",
            "current": n_md,
            "total": n_md,
            "message": format!("已扫描：共 {n_md} 个 Markdown 文件（将跳过 kf-private 与空内容）"),
        }),
    );

    let emb_conn = open_embedding_db(vault_root)?;
    semantic_rebuild_log(&format!(
        "db_opened files_to_process={} db={}",
        md_paths.len(),
        embedding_db_path(vault_root).display()
    ));

    let skip_doc_loop = resume
        && (rp.phase == "thoughts"
            || (rp.phase == "failed"
                && rp.docs_completed >= rp.docs_total
                && rp.thoughts_total > 0
                && rp.thoughts_next_index < rp.thoughts_total));

    semantic_rebuild_log(&format!(
        "resume branch: phase={} skip_doc_loop={skip_doc_loop} docs_done={}/{} thoughts_next={}/{}",
        rp.phase,
        rp.docs_completed,
        rp.docs_total,
        rp.thoughts_next_index,
        rp.thoughts_total
    ));

    let mut indexed_chunks = 0usize;
    let mut indexed_thoughts = 0usize;
    let total_files = md_paths.len();

    if !skip_doc_loop {
        for (fi, path) in md_paths.iter().enumerate() {
            if fi < rp.docs_completed {
                continue;
            }

            let rel = vault_context_search::rel_path_from_root(vault_root, path)
                .ok_or_else(|| "path not under root".to_string())?;
            let meta = fs::metadata(path).map_err(|e| format!("metadata {rel}: {e}"))?;
            let sz = meta.len();
            semantic_rebuild_log(&format!(
                "file {}/{} begin rel={} size_bytes={sz}",
                fi + 1,
                total_files,
                rel
            ));
            if sz > MAX_INDEX_FILE_BYTES {
                semantic_rebuild_log(&format!(
                    "file {}/{} skip_too_large rel={} size_bytes={sz} max_bytes={MAX_INDEX_FILE_BYTES}",
                    fi + 1,
                    total_files,
                    rel
                ));
                delete_chunks_for_path(&emb_conn, &rel)?;
                let hash = sha256_hex_file_stream(path)?;
                set_file_hash(&emb_conn, &rel, &hash, 0)?;
                flush_docs_rebuild_progress(vault_root, app, &mut rp, fi, &rel)?;
                continue;
            }

            let content = fs::read_to_string(path).map_err(|e| format!("read {rel}: {e}"))?;

            if note_privacy::markdown_treat_as_kf_private(&content) {
                semantic_rebuild_log(&format!("file {}/{} skip_kf_private rel={}", fi + 1, total_files, rel));
                delete_chunks_for_path(&emb_conn, &rel)?;
                flush_docs_rebuild_progress(vault_root, app, &mut rp, fi, &rel)?;
                continue;
            }
            let hash = sha256_hex_bytes(content.as_bytes());
            if let Some((old, _)) = get_file_hash(&emb_conn, &rel)? {
                if old == hash {
                    let _ = app.emit(
                        "semantic:index-progress",
                        serde_json::json!({
                            "phase": "files",
                            "current": fi + 1,
                            "total": total_files,
                            "message": rel,
                        }),
                    );
                    flush_docs_rebuild_progress(vault_root, app, &mut rp, fi, &rel)?;
                    continue;
                }
            }
            delete_chunks_for_path(&emb_conn, &rel)?;
            let content_chars = content.chars().count();
            let mut chunks = chunk::split_markdown_into_chunks(&content, 2048);
            drop(content);
            if chunks.len() > MAX_DOC_CHUNKS {
                semantic_rebuild_log(&format!(
                    "file {}/{} truncate_chunks rel={} {} -> {}",
                    fi + 1,
                    total_files,
                    rel,
                    chunks.len(),
                    MAX_DOC_CHUNKS
                ));
                chunks.truncate(MAX_DOC_CHUNKS);
            }
            let n = chunks.len() as i32;
            semantic_rebuild_log(&format!(
                "file {}/{} reindex rel={} chunks={} content_chars={}",
                fi + 1,
                total_files,
                rel,
                chunks.len(),
                content_chars
            ));
            for batch_start in (0..chunks.len()).step_by(INDEX_BATCH) {
                let end = (batch_start + INDEX_BATCH).min(chunks.len());
                let slice = &chunks[batch_start..end];
                let texts: Vec<String> = slice
                    .iter()
                    .map(|c| {
                        if let Some(ref h) = c.heading_context {
                            format!("{h}\n{}", c.text)
                        } else {
                            c.text.clone()
                        }
                    })
                    .collect();
                let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
                let vecs = encode_batch(model.as_ref(), &refs).map_err(|e| {
                    let first_chars = texts.first().map(|t| t.chars().count()).unwrap_or(0);
                    format!(
                        "encode_batch(doc) rel={rel} batch={batch_start}..{end} first_input_chars={first_chars}: {e}"
                    )
                })?;
                for (j, vec) in vecs.into_iter().enumerate() {
                    let global_i = batch_start + j;
                    let c = &slice[j];
                    if vec.len() != dim {
                        return Err("embedding dim mismatch".to_string());
                    }
                    let chunk_id = format!("{rel}#{global_i}");
                    upsert_doc_chunk(
                        &emb_conn,
                        &chunk_id,
                        &rel,
                        global_i as i32,
                        &c.text,
                        &vec,
                        model_id,
                    )?;
                    indexed_chunks += 1;
                }
            }
            set_file_hash(&emb_conn, &rel, &hash, n)?;
            let _ = app.emit(
                "semantic:index-progress",
                serde_json::json!({
                    "phase": "files",
                    "current": fi + 1,
                    "total": total_files,
                    "message": rel,
                }),
            );
            flush_docs_rebuild_progress(vault_root, app, &mut rp, fi, &rel)?;
        }
    }

    // 磁盘已删：清理仍留在 hash 表中的路径
    let mut rels_in_db: Vec<String> = emb_conn
        .prepare("SELECT rel_path FROM file_hashes")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for rel in rels_in_db.drain(..) {
        let joined = crate::join_under_root(vault_root, &rel).ok();
        let exists = joined.as_ref().map(|p| p.is_file()).unwrap_or(false);
        if !exists {
            delete_chunks_for_path(&emb_conn, &rel)?;
        }
    }

    let _ = app.emit(
        "semantic:index-progress",
        serde_json::json!({
            "phase": "thoughts",
            "current": 0,
            "total": 0,
            "message": "indexing thoughts",
        }),
    );

    semantic_rebuild_log("phase: thoughts (collect rows)");
    let tconn = vault_thoughts_db::open_thoughts_db(vault_root)?;
    let note_paths = list_thought_note_paths(&tconn)?;
    let rows = list_thought_rows_for_embed(&tconn)?;
    let mut thought_texts: Vec<(String, String, String)> = Vec::new();
    for (tid, body, updated_at, standalone) in rows {
        if standalone == 0 {
            if let Some(np) = note_paths.get(&tid) {
                let joined = crate::join_under_root(vault_root, np).ok();
                if let Some(p) = joined {
                    if p.is_file() {
                        let head = fs::read_to_string(&p).unwrap_or_default();
                        if note_privacy::markdown_treat_as_kf_private(&head) {
                            emb_conn
                                .execute(
                                    "DELETE FROM thought_embeddings WHERE thought_id = ?1",
                                    params![tid],
                                )
                                .map_err(|e| e.to_string())?;
                            continue;
                        }
                    }
                }
            }
        }
        let need = emb_conn
            .query_row(
                "SELECT updated_at FROM thought_embeddings WHERE thought_id = ?1",
                params![tid.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if need.as_deref() != Some(updated_at.as_str()) {
            thought_texts.push((tid, body, updated_at));
        }
    }

    let tt_len = thought_texts.len();
    if skip_doc_loop && tt_len != rp.thoughts_total {
        return Err(format!(
            "thought embed queue length changed (checkpoint {}, current {}); run a full rebuild",
            rp.thoughts_total, tt_len
        ));
    }
    rp.thoughts_total = tt_len;
    rp.phase = "thoughts".to_string();
    if !skip_doc_loop {
        rp.thoughts_next_index = 0;
    }
    rp.updated_at = chrono::Utc::now().to_rfc3339();
    rebuild_progress::write_rebuild_progress(vault_root, &rp)?;
    rebuild_progress::emit_checkpoint(app, &rp);

    let thought_start = if skip_doc_loop {
        rp.thoughts_next_index.min(tt_len)
    } else {
        0
    };

    semantic_rebuild_log(&format!(
        "thoughts_to_embed={} start_index={thought_start} (encode batches of {})",
        tt_len,
        INDEX_BATCH
    ));
    for batch_start in (thought_start..thought_texts.len()).step_by(INDEX_BATCH) {
        let end = (batch_start + INDEX_BATCH).min(thought_texts.len());
        let slice = &thought_texts[batch_start..end];
        let refs: Vec<&str> = slice.iter().map(|(_, b, _)| b.as_str()).collect();
        let vecs = encode_batch(model.as_ref(), &refs).map_err(|e| {
            let tid0 = slice.first().map(|(t, _, _)| t.as_str()).unwrap_or("?");
            let b0_chars = slice.first().map(|(_, b, _)| b.chars().count()).unwrap_or(0);
            format!(
                "encode_batch(thought) batch={batch_start}..{end} first_thought_id={tid0} first_body_chars={b0_chars}: {e}"
            )
        })?;
        for (j, vec) in vecs.into_iter().enumerate() {
            let (tid, _, updated_at) = &slice[j];
            upsert_thought_embedding(&emb_conn, tid, &vec, model_id, updated_at)?;
            indexed_thoughts += 1;
        }
        rp.thoughts_next_index = end;
        rp.last_message = Some(format!("thoughts batch {batch_start}..{end}"));
        rp.updated_at = chrono::Utc::now().to_rfc3339();
        rebuild_progress::write_rebuild_progress(vault_root, &rp)?;
        rebuild_progress::emit_checkpoint(app, &rp);
    }

    // 删除侧车已不存在的想法 embedding
    let valid_ids: HashSet<String> = tconn
        .prepare("SELECT thought_id FROM thoughts")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|e| e.to_string())?;
    let te_ids: Vec<String> = emb_conn
        .prepare("SELECT thought_id FROM thought_embeddings")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for tid in te_ids {
        if !valid_ids.contains(&tid) {
            emb_conn
                .execute(
                    "DELETE FROM thought_embeddings WHERE thought_id = ?1",
                    params![tid],
                )
                .map_err(|e| e.to_string())?;
        }
    }

    let elapsed_ms = t0.elapsed().as_millis() as u64;
    rp.docs_completed = rp.docs_total;
    rp.thoughts_next_index = rp.thoughts_total;
    rp.mark_completed();
    rp.last_message = Some(format!(
        "completed chunks={indexed_chunks} thoughts={indexed_thoughts}"
    ));
    rebuild_progress::write_rebuild_progress(vault_root, &rp)?;
    rebuild_progress::emit_checkpoint(app, &rp);

    if let Some(ec) = app.try_state::<Arc<EmbeddingCache>>() {
        ec.invalidate();
    }

    semantic_rebuild_log(&format!(
        "rebuild_index: ok indexed_chunks={indexed_chunks} indexed_thoughts={indexed_thoughts} elapsed_ms={elapsed_ms}"
    ));
    let _ = app.emit(
        "semantic:index-complete",
        serde_json::json!({
            "indexedChunks": indexed_chunks,
            "indexedThoughts": indexed_thoughts,
            "elapsedMs": elapsed_ms,
        }),
    );

    Ok(IndexBuildResult {
        indexed_chunks,
        indexed_thoughts,
        elapsed_ms,
    })
}

pub fn run_semantic_search(
    vault_root: &Path,
    cache_dir: &Path,
    bundle_dir: &Path,
    args: SemanticSearchArgs,
    embed_cache: &EmbeddingCache,
) -> Result<Vec<SemanticSearchHit>, String> {
    let model = get_cached_or_load_model(cache_dir, bundle_dir)?;
    let conn = open_embedding_db(vault_root)?;
    let qv = encode_single(model.as_ref(), args.query.trim())?;
    let scope = args
        .search_scope
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("all");
    let exclude = HashSet::new();
    let mut out = Vec::new();
    if scope == "docs" || scope == "all" {
        let rows = embed_cache.get_docs(&conn);
        out.extend(semantic_search_docs(&qv, &rows, args.top_k, &exclude));
    }
    if scope == "thoughts" || scope == "all" {
        let thoughts = embed_cache.get_thoughts(&conn);
        let mut h = semantic_search_thoughts(&qv, &thoughts, args.top_k);
        let tconn = vault_thoughts_db::open_thoughts_db(vault_root)?;
        for hit in &mut h {
            if let Some(ref tid) = hit.thought_id {
                if let Ok(Some(body)) = vault_thoughts_db::get_body(&tconn, tid) {
                    let excerpt: String = body.chars().take(400).collect();
                    hit.chunk_text = excerpt;
                }
            }
        }
        out.extend(h);
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(args.top_k);
    Ok(out)
}

fn reciprocal_rank_fusion_owned(
    keyword_results: &[(String, f64)],
    semantic_results: &[(String, f64)],
    keyword_weight: f64,
    semantic_weight: f64,
    top_k: usize,
) -> Vec<(String, f64)> {
    let k = 60.0;
    let mut acc: HashMap<String, f64> = HashMap::new();
    for (rank, (id, _)) in keyword_results.iter().enumerate() {
        *acc.entry(id.clone()).or_default() += keyword_weight / (k + rank as f64 + 1.0);
    }
    for (rank, (id, _)) in semantic_results.iter().enumerate() {
        *acc.entry(id.clone()).or_default() += semantic_weight / (k + rank as f64 + 1.0);
    }
    let mut v: Vec<(String, f64)> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(top_k);
    v
}

/// 实际注入模型的一条语义补充中，文档路径与想法 id（供 UI 引用展示）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticContextForLlmUsed {
    pub document_paths: Vec<String>,
    pub thought_ids: Vec<String>,
}

/// 供 LLM 拼装的语义摘录块及引用元数据。
pub struct SemanticContextForLlmResult {
    pub block: String,
    pub used: SemanticContextForLlmUsed,
}

fn push_unique_path(out: &mut Vec<String>, p: &str) {
    let n = p.replace('\\', "/");
    if n.is_empty() || out.contains(&n) {
        return;
    }
    out.push(n);
}

fn push_unique_thought(out: &mut Vec<String>, id: &str) {
    if id.is_empty() || out.contains(&id.to_string()) {
        return;
    }
    out.push(id.to_string());
}

/// 供 LLM 拼装：返回可选语义摘录块；失败时静默省略
///
/// `omit_doc_rel_paths`：已在「当前笔记」system 中全文注入的路径，此处不再检索/摘录，避免重复与模型复述。
pub fn build_semantic_context_for_llm(
    vault_root: &Path,
    cache_dir: &Path,
    bundle_dir: &Path,
    query: &str,
    cfg: &SemanticConfig,
    keyword_snippet_paths: &[String],
    omit_doc_rel_paths: &[String],
    embed_cache: &EmbeddingCache,
) -> Option<SemanticContextForLlmResult> {
    if !cfg.enabled {
        return None;
    }
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    let omit_set: HashSet<String> = omit_doc_rel_paths
        .iter()
        .map(|p| p.replace('\\', "/"))
        .filter(|p| !p.is_empty())
        .collect();
    let model = get_cached_or_load_model(cache_dir, bundle_dir).ok()?;
    let conn = open_embedding_db(vault_root).ok()?;
    let tconn = vault_thoughts_db::open_thoughts_db(vault_root).ok()?;
    let qv = encode_single(model.as_ref(), q).ok()?;
    let doc_rows = embed_cache.get_docs(&conn);
    let thought_rows = embed_cache.get_thoughts(&conn);
    let sem_docs = semantic_search_docs(&qv, &doc_rows, 12, &omit_set);
    let mut sem_thoughts = semantic_search_thoughts(&qv, &thought_rows, 12);
    for hit in &mut sem_thoughts {
        if let Some(ref tid) = hit.thought_id {
            if let Ok(Some(body)) = vault_thoughts_db::get_body(&tconn, tid) {
                hit.chunk_text = body.chars().take(500).collect();
            }
        }
    }
    let mut sem_owned: Vec<(String, f64)> = Vec::new();
    for h in &sem_docs {
        if let Some(ref p) = h.rel_path {
            sem_owned.push((p.clone(), h.score));
        }
    }
    for h in &sem_thoughts {
        if let Some(ref tid) = h.thought_id {
            sem_owned.push((format!("thought:{tid}"), h.score));
        }
    }
    let kw_owned: Vec<(String, f64)> = keyword_snippet_paths
        .iter()
        .filter(|s| !omit_set.contains(&s.replace('\\', "/")))
        .cloned()
        .map(|s| (s, 1.0))
        .collect();
    let sem_w = cfg.search_weight.clamp(0.0, 1.0);
    let kw_w = (1.0 - sem_w).clamp(0.0, 1.0);
    let fused = reciprocal_rank_fusion_owned(&kw_owned, &sem_owned, kw_w, sem_w, 12);
    let mut lines: Vec<String> = Vec::new();
    lines.push(
        "Supplementary vault excerpts (keyword + semantic fusion). If the user's open note system message already includes a file, that message is authoritative; do not restate its full text here."
            .to_string(),
    );
    let mut used = 0usize;
    let mut document_paths: Vec<String> = Vec::new();
    let mut thought_ids: Vec<String> = Vec::new();
    for (id, _score) in fused {
        if used >= 3 {
            break;
        }
        if omit_set.contains(&id.replace('\\', "/")) {
            continue;
        }
        if let Some(rest) = id.strip_prefix("thought:") {
            if let Ok(Some(body)) = vault_thoughts_db::get_body(&tconn, rest) {
                let excerpt: String = body.chars().take(500).collect();
                lines.push(format!("- Thought `{rest}`: {excerpt}"));
                push_unique_thought(&mut thought_ids, rest);
                used += 1;
            }
            continue;
        }
        if let Some(h) = sem_docs.iter().find(|h| h.rel_path.as_deref() == Some(id.as_str())) {
            lines.push(format!(
                "- `{}`: {}",
                id,
                h.chunk_text.chars().take(600).collect::<String>()
            ));
            push_unique_path(&mut document_paths, &id);
            used += 1;
            continue;
        }
        if let Ok(joined) = crate::join_under_root(vault_root, &id) {
            if joined.is_file() {
                if let Ok(txt) = fs::read_to_string(&joined) {
                    if !note_privacy::markdown_treat_as_kf_private(&txt) {
                        let excerpt: String = txt.chars().take(500).collect();
                        lines.push(format!("- `{id}`: {excerpt}"));
                        push_unique_path(&mut document_paths, &id);
                        used += 1;
                    }
                }
            }
        }
    }
    if lines.len() <= 1 {
        return None;
    }
    Some(SemanticContextForLlmResult {
        block: lines.join("\n"),
        used: SemanticContextForLlmUsed {
            document_paths,
            thought_ids,
        },
    })
}

/// 保存单篇笔记后按配置增量更新向量（失败仅打日志）
pub fn incremental_reindex_note(vault_root: &Path, app: &AppHandle, rel_path: &str) {
    let res = (|| -> Result<(), String> {
        let cfg = vault_config::load_semantic_merged(vault_root)?;
        if !cfg.enabled || !cfg.auto_index_on_save {
            return Ok(());
        }
        let cache = default_model_cache_dir();
        let bundle = resolve_bundle_model_dir(app);
        if !builtin_embed::is_model_ready(&cache, &bundle) {
            return Ok(());
        }
        let model = get_cached_or_load_model(&cache, &bundle)?;
        let model_id = builtin_embed::DEFAULT_MODEL_ID;
        let dim = model.dim;
        let joined = crate::join_under_root(vault_root, rel_path)?;
        if !crate::is_markdown_path(&joined) || !joined.is_file() {
            return Ok(());
        }
        let mut emb_conn = open_embedding_db(vault_root)?;
        let meta = fs::metadata(&joined).map_err(|e| e.to_string())?;
        let sz = meta.len();
        if sz > MAX_INDEX_FILE_BYTES {
            delete_chunks_for_path(&emb_conn, rel_path)?;
            let hash = sha256_hex_file_stream(&joined)?;
            set_file_hash(&emb_conn, rel_path, &hash, 0)?;
            return Ok(());
        }
        let content = fs::read_to_string(&joined).map_err(|e| e.to_string())?;
        if note_privacy::markdown_treat_as_kf_private(&content) {
            delete_chunks_for_path(&emb_conn, rel_path)?;
            return Ok(());
        }
        let hash = sha256_hex_bytes(content.as_bytes());
        if let Some((old, _)) = get_file_hash(&emb_conn, rel_path)? {
            if old == hash {
                return Ok(());
            }
        }
        let mut chunks = chunk::split_markdown_into_chunks(&content, 2048);
        if chunks.len() > MAX_DOC_CHUNKS {
            chunks.truncate(MAX_DOC_CHUNKS);
        }
        let n = chunks.len() as i32;
        // 先完成向量编码，再在单笔事务内替换 doc_chunks：避免先删后写窗口内链接推荐读到「无 chunk」
        let mut staged: Vec<(usize, String, Vec<f32>)> = Vec::with_capacity(chunks.len());
        for batch_start in (0..chunks.len()).step_by(INDEX_BATCH) {
            let end = (batch_start + INDEX_BATCH).min(chunks.len());
            let slice = &chunks[batch_start..end];
            let texts: Vec<String> = slice
                .iter()
                .map(|c| {
                    if let Some(ref h) = c.heading_context {
                        format!("{h}\n{}", c.text)
                    } else {
                        c.text.clone()
                    }
                })
                .collect();
            let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            let vecs = encode_batch(model.as_ref(), &refs)?;
            for (j, vec) in vecs.into_iter().enumerate() {
                let global_i = batch_start + j;
                let c = &slice[j];
                if vec.len() != dim {
                    return Err("embedding dim mismatch".to_string());
                }
                staged.push((global_i, c.text.clone(), vec));
            }
        }
        let tx = emb_conn.transaction().map_err(|e| e.to_string())?;
        delete_chunks_for_path(&tx, rel_path)?;
        for (global_i, text, vec) in staged {
            let chunk_id = format!("{rel_path}#{global_i}");
            upsert_doc_chunk(
                &tx,
                &chunk_id,
                rel_path,
                global_i as i32,
                &text,
                &vec,
                model_id,
            )?;
        }
        set_file_hash(&tx, rel_path, &hash, n)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = &res {
        eprintln!("[semantic_index] incremental reindex skipped: {e}");
    }
    if res.is_ok() {
        if let Some(ec) = app.try_state::<Arc<EmbeddingCache>>() {
            ec.invalidate();
        }
    }
}
