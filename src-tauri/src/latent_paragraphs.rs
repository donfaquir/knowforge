use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use crate::note_privacy;
use crate::semantic_index::{cosine_similarity, DocChunkRow, EmbeddingCache};

const THRESHOLD_HIGH_SIM: f32 = 0.85;
const THRESHOLD_ISOLATED: f32 = 0.3;
const THRESHOLD_CLUSTER: f32 = 0.75;
const MAX_SCAN_CHUNKS: usize = 20_000;
const MAX_CANDIDATES: usize = 500;
const MIN_CHUNK_CHARS: usize = 50;
const EXCERPT_LEN: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateForUi {
    pub id: String,
    pub rel_path: String,
    pub excerpt: String,
    pub marking_reason: String,
    pub similarity_score: Option<f64>,
    pub paired_rel_path: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub total_chunks_scanned: usize,
    pub candidates_found: usize,
}

#[derive(Debug, Clone)]
struct RawCandidate {
    chunk_idx: usize,
    marking_reason: &'static str,
    similarity_score: Option<f64>,
    paired_rel_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

pub fn init_candidates_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS thought_candidates (
            id TEXT PRIMARY KEY,
            rel_path TEXT NOT NULL,
            chunk_id TEXT NOT NULL,
            paragraph_start_line INTEGER NOT NULL,
            paragraph_end_line INTEGER NOT NULL,
            paragraph_hash TEXT NOT NULL,
            marking_reason TEXT NOT NULL,
            similarity_score REAL,
            paired_rel_path TEXT,
            created_at TEXT NOT NULL,
            dismissed_at TEXT,
            promoted_thought_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_tc_rel_path ON thought_candidates(rel_path);
        CREATE INDEX IF NOT EXISTS idx_tc_reason ON thought_candidates(marking_reason);
        CREATE INDEX IF NOT EXISTS idx_tc_chunk_id ON thought_candidates(chunk_id);
        "#,
    )
    .map_err(|e| format!("init thought_candidates schema: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Heuristic filters
// ---------------------------------------------------------------------------

pub fn should_skip_chunk(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() < MIN_CHUNK_CHARS {
        return true;
    }
    if is_pure_list(trimmed) {
        return true;
    }
    if is_code_block(trimmed) {
        return true;
    }
    if is_quote_block(trimmed) {
        return true;
    }
    false
}

fn is_pure_list(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| {
        let t = line.trim_start();
        t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("+ ")
            || t.chars()
                .take_while(|c| c.is_ascii_digit())
                .count()
                .gt(&0)
                && (t.contains(". ") || t.contains(") "))
    })
}

fn is_code_block(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("```") && trimmed.ends_with("```") && trimmed.matches("```").count() >= 2
}

fn is_quote_block(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| line.trim_start().starts_with("> "))
}

// ---------------------------------------------------------------------------
// Line number computation
// ---------------------------------------------------------------------------

fn compute_line_range(file_content: &str, chunk_text: &str) -> (i32, i32) {
    let search_text = strip_heading_context(chunk_text);
    if let Some(byte_offset) = file_content.find(&search_text) {
        let newlines_before = file_content[..byte_offset].matches('\n').count();
        let start_line = (newlines_before + 1) as i32;
        let chunk_lines = search_text.lines().count().max(1) as i32;
        (start_line, start_line + chunk_lines - 1)
    } else {
        (1, 1)
    }
}

fn strip_heading_context(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut start = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with('#') {
            start = i + 1;
            while start < lines.len() && lines[start].trim().is_empty() {
                start += 1;
            }
            break;
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    lines[start..].join("\n")
}

// ---------------------------------------------------------------------------
// Union-Find for cross-doc recurrence
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Core scan
// ---------------------------------------------------------------------------

pub fn scan_vault(
    embed_conn: &Connection,
    embed_cache: &EmbeddingCache,
    vault_root: &Path,
) -> Result<ScanResult, String> {
    let all_docs = embed_cache.get_docs(embed_conn);

    let chunks = filter_chunks(&all_docs, vault_root);
    let n = chunks.len();
    if n == 0 {
        return Ok(ScanResult {
            total_chunks_scanned: 0,
            candidates_found: 0,
        });
    }

    eprintln!(
        "[latent_paragraphs] scan_vault: {} chunks after filtering (from {} total)",
        n,
        all_docs.len()
    );

    let candidates = compute_candidates(&chunks);

    let now = chrono::Utc::now().to_rfc3339();
    let inserted = persist_candidates(embed_conn, vault_root, &chunks, &candidates, &now)?;

    eprintln!("[latent_paragraphs] scan_vault: {inserted} candidates persisted");

    Ok(ScanResult {
        total_chunks_scanned: n,
        candidates_found: inserted,
    })
}

fn filter_chunks<'a>(all_docs: &'a [DocChunkRow], vault_root: &Path) -> Vec<&'a DocChunkRow> {
    let mut privacy_cache: HashMap<String, bool> = HashMap::new();
    let mut chunks: Vec<&DocChunkRow> = Vec::new();

    for chunk in all_docs.iter() {
        let is_private = *privacy_cache
            .entry(chunk.rel_path.clone())
            .or_insert_with(|| {
                let full = vault_root.join(&chunk.rel_path);
                note_privacy::peek_kf_private_from_md_file(&full)
            });
        if is_private {
            continue;
        }
        if should_skip_chunk(&chunk.chunk_text) {
            continue;
        }
        chunks.push(chunk);
    }

    if chunks.len() > MAX_SCAN_CHUNKS {
        eprintln!(
            "[latent_paragraphs] capping scan to {MAX_SCAN_CHUNKS} chunks (had {})",
            chunks.len()
        );
        chunks.truncate(MAX_SCAN_CHUNKS);
    }

    chunks
}

fn compute_candidates(chunks: &[&DocChunkRow]) -> Vec<RawCandidate> {
    let n = chunks.len();
    let mut max_sim = vec![0.0f32; n];
    let mut high_sim_pairs: Vec<(usize, usize, f32)> = Vec::new();
    let mut uf = UnionFind::new(n);

    for i in 0..n {
        for j in (i + 1)..n {
            let sim = cosine_similarity(&chunks[i].embedding, &chunks[j].embedding);

            if sim > max_sim[i] {
                max_sim[i] = sim;
            }
            if sim > max_sim[j] {
                max_sim[j] = sim;
            }

            let cross_doc = chunks[i].rel_path != chunks[j].rel_path;
            if !cross_doc {
                continue;
            }

            if sim > THRESHOLD_HIGH_SIM {
                high_sim_pairs.push((i, j, sim));
            }
            if sim > THRESHOLD_CLUSTER {
                uf.union(i, j);
            }
        }
    }

    let mut marked: HashMap<usize, RawCandidate> = HashMap::new();

    // 1. High similarity pairs (highest priority)
    for &(i, j, sim) in &high_sim_pairs {
        marked.entry(i).or_insert(RawCandidate {
            chunk_idx: i,
            marking_reason: "high_similarity",
            similarity_score: Some(sim as f64),
            paired_rel_path: Some(chunks[j].rel_path.clone()),
        });
        marked.entry(j).or_insert(RawCandidate {
            chunk_idx: j,
            marking_reason: "high_similarity",
            similarity_score: Some(sim as f64),
            paired_rel_path: Some(chunks[i].rel_path.clone()),
        });
    }

    // 2. Cross-doc recurrence (connected components spanning 3+ docs)
    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        components.entry(uf.find(i)).or_default().push(i);
    }
    for (_root, members) in &components {
        let doc_set: HashSet<&str> = members.iter().map(|&i| chunks[i].rel_path.as_str()).collect();
        if doc_set.len() >= 3 {
            for &idx in members {
                marked.entry(idx).or_insert(RawCandidate {
                    chunk_idx: idx,
                    marking_reason: "cross_doc_recurrence",
                    similarity_score: Some(max_sim[idx] as f64),
                    paired_rel_path: None,
                });
            }
        }
    }

    // 3. Semantic isolated (lowest priority)
    for i in 0..n {
        if max_sim[i] < THRESHOLD_ISOLATED {
            marked.entry(i).or_insert(RawCandidate {
                chunk_idx: i,
                marking_reason: "semantic_isolated",
                similarity_score: Some(max_sim[i] as f64),
                paired_rel_path: None,
            });
        }
    }

    let mut result: Vec<RawCandidate> = marked.into_values().collect();
    result.sort_by(|a, b| {
        b.similarity_score
            .unwrap_or(0.0)
            .partial_cmp(&a.similarity_score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result.truncate(MAX_CANDIDATES);
    result
}

fn persist_candidates(
    conn: &Connection,
    vault_root: &Path,
    chunks: &[&DocChunkRow],
    candidates: &[RawCandidate],
    now: &str,
) -> Result<usize, String> {
    // Clear old non-dismissed/non-promoted candidates
    conn.execute(
        "DELETE FROM thought_candidates WHERE dismissed_at IS NULL AND promoted_thought_id IS NULL",
        [],
    )
    .map_err(|e| format!("clear old candidates: {e}"))?;

    let mut file_cache: HashMap<String, String> = HashMap::new();
    let mut inserted = 0;

    for cand in candidates {
        let chunk = chunks[cand.chunk_idx];
        let file_content = file_cache
            .entry(chunk.rel_path.clone())
            .or_insert_with(|| {
                let path = vault_root.join(&chunk.rel_path);
                std::fs::read_to_string(&path).unwrap_or_default()
            });

        let (start_line, end_line) = compute_line_range(file_content, &chunk.chunk_text);
        let hash = paragraph_hash(&chunk.chunk_text);
        let id = uuid::Uuid::new_v4().to_string();

        conn.execute(
            "INSERT OR REPLACE INTO thought_candidates \
             (id, rel_path, chunk_id, paragraph_start_line, paragraph_end_line, \
              paragraph_hash, marking_reason, similarity_score, paired_rel_path, \
              created_at, dismissed_at, promoted_thought_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL)",
            params![
                id,
                chunk.rel_path,
                chunk.chunk_id,
                start_line,
                end_line,
                hash,
                cand.marking_reason,
                cand.similarity_score,
                cand.paired_rel_path,
                now,
            ],
        )
        .map_err(|e| format!("insert candidate: {e}"))?;
        inserted += 1;
    }

    Ok(inserted)
}

fn paragraph_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn excerpt(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= EXCERPT_LEN {
        text.to_string()
    } else {
        let mut s: String = chars[..EXCERPT_LEN].iter().collect();
        s.push_str("…");
        s
    }
}

// ---------------------------------------------------------------------------
// Incremental scan for a single note
// ---------------------------------------------------------------------------

pub fn incremental_scan_for_note(
    embed_conn: &Connection,
    embed_cache: &EmbeddingCache,
    vault_root: &Path,
    rel_path: &str,
) -> Result<(), String> {
    let full_path = vault_root.join(rel_path);
    if note_privacy::peek_kf_private_from_md_file(&full_path) {
        conn_delete_candidates_for_path(embed_conn, rel_path)?;
        return Ok(());
    }

    let all_docs = embed_cache.get_docs(embed_conn);

    let my_chunks: Vec<&DocChunkRow> = all_docs
        .iter()
        .filter(|c| c.rel_path == rel_path && !should_skip_chunk(&c.chunk_text))
        .collect();
    let other_chunks: Vec<&DocChunkRow> = all_docs
        .iter()
        .filter(|c| c.rel_path != rel_path && !should_skip_chunk(&c.chunk_text))
        .collect();

    if my_chunks.is_empty() {
        conn_delete_candidates_for_path(embed_conn, rel_path)?;
        return Ok(());
    }

    // Delete old undismissed candidates for this path
    conn_delete_candidates_for_path(embed_conn, rel_path)?;

    let file_content = std::fs::read_to_string(&full_path).unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    let mut inserted = 0;

    for my_chunk in &my_chunks {
        let mut max_sim: f32 = 0.0;
        let mut best_cross_doc_sim: f32 = 0.0;
        let mut best_cross_doc_path: Option<String> = None;
        let mut cross_doc_high_sim = false;

        for other in &other_chunks {
            let sim = cosine_similarity(&my_chunk.embedding, &other.embedding);
            if sim > max_sim {
                max_sim = sim;
            }
            if sim > best_cross_doc_sim {
                best_cross_doc_sim = sim;
                best_cross_doc_path = Some(other.rel_path.clone());
            }
            if sim > THRESHOLD_HIGH_SIM {
                cross_doc_high_sim = true;
            }
        }

        let reason = if cross_doc_high_sim {
            Some(("high_similarity", best_cross_doc_sim, best_cross_doc_path.clone()))
        } else if max_sim < THRESHOLD_ISOLATED {
            Some(("semantic_isolated", max_sim, None))
        } else {
            None
        };

        if let Some((reason_str, score, paired)) = reason {
            let (start_line, end_line) =
                compute_line_range(&file_content, &my_chunk.chunk_text);
            let hash = paragraph_hash(&my_chunk.chunk_text);
            let id = uuid::Uuid::new_v4().to_string();

            embed_conn
                .execute(
                    "INSERT OR REPLACE INTO thought_candidates \
                     (id, rel_path, chunk_id, paragraph_start_line, paragraph_end_line, \
                      paragraph_hash, marking_reason, similarity_score, paired_rel_path, \
                      created_at, dismissed_at, promoted_thought_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL)",
                    params![
                        id,
                        rel_path,
                        my_chunk.chunk_id,
                        start_line,
                        end_line,
                        hash,
                        reason_str,
                        Some(score as f64),
                        paired,
                        now,
                    ],
                )
                .map_err(|e| format!("insert incremental candidate: {e}"))?;
            inserted += 1;
        }
    }

    eprintln!(
        "[latent_paragraphs] incremental_scan for {rel_path}: {inserted} candidates"
    );
    Ok(())
}

fn conn_delete_candidates_for_path(conn: &Connection, rel_path: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thought_candidates WHERE rel_path = ?1 AND dismissed_at IS NULL AND promoted_thought_id IS NULL",
        params![rel_path],
    )
    .map_err(|e| format!("delete candidates for path: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Query & operations
// ---------------------------------------------------------------------------

pub fn list_candidates(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<CandidateForUi>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT tc.id, tc.rel_path, tc.paragraph_start_line, tc.paragraph_end_line,
                    tc.marking_reason, tc.similarity_score, tc.paired_rel_path, tc.chunk_id
             FROM thought_candidates tc
             WHERE tc.dismissed_at IS NULL AND tc.promoted_thought_id IS NULL
             ORDER BY tc.similarity_score DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("prepare list candidates: {e}"))?;

    let rows = stmt
        .query_map(params![limit as i64], |row| {
            let chunk_id: String = row.get(7)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                chunk_id,
            ))
        })
        .map_err(|e| format!("query candidates: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        let (id, rel_path, start_line, end_line, reason, score, paired, chunk_id) =
            row.map_err(|e| format!("read candidate row: {e}"))?;

        let chunk_text: String = conn
            .query_row(
                "SELECT chunk_text FROM doc_chunks WHERE chunk_id = ?1",
                params![chunk_id],
                |r| r.get(0),
            )
            .unwrap_or_default();

        result.push(CandidateForUi {
            id,
            rel_path,
            excerpt: excerpt(&chunk_text),
            marking_reason: reason,
            similarity_score: score,
            paired_rel_path: paired,
            start_line,
            end_line,
        });
    }

    Ok(result)
}

pub fn dismiss_candidate(conn: &Connection, id: &str) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE thought_candidates SET dismissed_at = ?1 WHERE id = ?2",
        params![now, id],
    )
    .map_err(|e| format!("dismiss candidate: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_skip_short_text() {
        assert!(should_skip_chunk("hi"));
        assert!(should_skip_chunk("   "));
        assert!(should_skip_chunk(""));
    }

    #[test]
    fn test_should_skip_list() {
        let list = "- item one\n- item two\n- item three\n- item four and some more text";
        assert!(should_skip_chunk(list));

        let numbered = "1. first thing\n2. second thing\n3. third thing here";
        assert!(should_skip_chunk(numbered));
    }

    #[test]
    fn test_should_not_skip_prose() {
        let prose = "This is a paragraph with enough text to be considered meaningful content for analysis and review purposes.";
        assert!(!should_skip_chunk(prose));
    }

    #[test]
    fn test_should_skip_code_block() {
        let code = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        assert!(should_skip_chunk(code));
    }

    #[test]
    fn test_should_skip_quote_block() {
        let quote = "> This is a quoted paragraph that spans\n> multiple lines and has enough content.";
        assert!(should_skip_chunk(quote));
    }

    #[test]
    fn test_mixed_content_not_skipped() {
        let mixed = "Some prose paragraph here.\n\n- a list item\n\nMore prose follows.";
        assert!(!should_skip_chunk(mixed));
    }

    #[test]
    fn test_strip_heading_context() {
        let text = "## My Heading\n\nThis is the actual content of the paragraph.";
        let stripped = strip_heading_context(text);
        assert_eq!(stripped, "This is the actual content of the paragraph.");
    }

    #[test]
    fn test_strip_heading_context_no_heading() {
        let text = "Just some regular paragraph content here.";
        let stripped = strip_heading_context(text);
        assert_eq!(stripped, "Just some regular paragraph content here.");
    }

    #[test]
    fn test_compute_line_range() {
        let file = "line 1\nline 2\nfoo bar baz\nline 4\nline 5";
        let (start, end) = compute_line_range(file, "foo bar baz");
        assert_eq!(start, 3);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_compute_line_range_multiline() {
        let file = "line 1\nline 2\nfoo bar\nbaz qux\nline 5";
        let (start, end) = compute_line_range(file, "foo bar\nbaz qux");
        assert_eq!(start, 3);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_union_find() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(2, 3);
        uf.union(1, 3);
        assert_eq!(uf.find(0), uf.find(3));
        assert_ne!(uf.find(0), uf.find(4));
    }

    #[test]
    fn test_paragraph_hash_deterministic() {
        let h1 = paragraph_hash("hello world");
        let h2 = paragraph_hash("hello world");
        assert_eq!(h1, h2);
        let h3 = paragraph_hash("hello world!");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_excerpt_short() {
        let text = "Short text.";
        assert_eq!(excerpt(text), "Short text.");
    }

    #[test]
    fn test_excerpt_long() {
        let text = "A".repeat(300);
        let ex = excerpt(&text);
        assert!(ex.len() < 300);
        assert!(ex.ends_with('…'));
    }

    #[test]
    fn test_compute_candidates_high_similarity() {
        let base_embedding = vec![1.0f32; 16];
        let similar_embedding = {
            let mut v = vec![1.0f32; 16];
            v[0] = 0.99;
            v
        };
        let distant_embedding = vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                                      0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];

        let chunks_owned = vec![
            DocChunkRow {
                chunk_id: "a.md#0".to_string(),
                rel_path: "a.md".to_string(),
                chunk_index: 0,
                chunk_text: "some text content that is long enough to not be filtered".to_string(),
                embedding: base_embedding,
                dim: 16,
                model_id: "test".to_string(),
            },
            DocChunkRow {
                chunk_id: "b.md#0".to_string(),
                rel_path: "b.md".to_string(),
                chunk_index: 0,
                chunk_text: "some text content that is long enough to not be filtered".to_string(),
                embedding: similar_embedding,
                dim: 16,
                model_id: "test".to_string(),
            },
            DocChunkRow {
                chunk_id: "c.md#0".to_string(),
                rel_path: "c.md".to_string(),
                chunk_index: 0,
                chunk_text: "completely different paragraph content here for testing".to_string(),
                embedding: distant_embedding,
                dim: 16,
                model_id: "test".to_string(),
            },
        ];

        let chunks: Vec<&DocChunkRow> = chunks_owned.iter().collect();
        let candidates = compute_candidates(&chunks);

        let high_sim: Vec<_> = candidates
            .iter()
            .filter(|c| c.marking_reason == "high_similarity")
            .collect();
        assert!(
            !high_sim.is_empty(),
            "should detect high similarity between a.md and b.md"
        );
    }

    #[test]
    fn test_compute_candidates_isolated() {
        let chunks_owned = vec![
            DocChunkRow {
                chunk_id: "a.md#0".to_string(),
                rel_path: "a.md".to_string(),
                chunk_index: 0,
                chunk_text: "this is paragraph content in document a for testing purposes".to_string(),
                embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                dim: 8,
                model_id: "test".to_string(),
            },
            DocChunkRow {
                chunk_id: "b.md#0".to_string(),
                rel_path: "b.md".to_string(),
                chunk_index: 0,
                chunk_text: "this is paragraph content in document b for testing purposes".to_string(),
                embedding: vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                dim: 8,
                model_id: "test".to_string(),
            },
            DocChunkRow {
                chunk_id: "c.md#0".to_string(),
                rel_path: "c.md".to_string(),
                chunk_index: 0,
                chunk_text: "this is paragraph content in document c for testing purposes".to_string(),
                embedding: vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                dim: 8,
                model_id: "test".to_string(),
            },
        ];

        let chunks: Vec<&DocChunkRow> = chunks_owned.iter().collect();
        let candidates = compute_candidates(&chunks);

        let isolated: Vec<_> = candidates
            .iter()
            .filter(|c| c.marking_reason == "semantic_isolated")
            .collect();
        assert_eq!(
            isolated.len(),
            3,
            "all chunks are orthogonal, all should be isolated"
        );
    }
}
