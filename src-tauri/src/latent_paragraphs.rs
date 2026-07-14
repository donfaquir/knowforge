use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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

/// Bump this when filter logic changes to invalidate cached candidates.
const FILTER_VERSION: i64 = 2;

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

// ---------------------------------------------------------------------------
// Discovery filter/response types (for list_candidates_filtered)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryFilter {
    /// "high_similarity" | "cross_doc_recurrence" | "semantic_isolated" | None (all)
    pub marking_reason: Option<String>,
    /// "freshness" | "similarity" | "age"
    pub sort_by: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryListResponse {
    pub items: Vec<CandidateForUi>,
    pub total: usize,
    pub by_reason: DiscoveryReasonCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReasonCounts {
    pub high_similarity: usize,
    pub cross_doc_recurrence: usize,
    pub semantic_isolated: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
        CREATE TABLE IF NOT EXISTS latent_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("init thought_candidates schema: {e}"))?;
    Ok(())
}

/// Check if the stored filter version matches the current FILTER_VERSION.
/// If outdated, clear all non-dismissed/non-promoted candidates so a fresh scan runs.
pub fn invalidate_if_filter_changed(conn: &Connection) -> Result<bool, String> {
    let stored: i64 = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM latent_meta WHERE key = 'filter_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if stored == FILTER_VERSION {
        return Ok(false);
    }
    conn.execute(
        "DELETE FROM thought_candidates WHERE dismissed_at IS NULL AND promoted_thought_id IS NULL",
        [],
    )
    .map_err(|e| format!("clear outdated candidates: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO latent_meta (key, value) VALUES ('filter_version', ?1)",
        params![FILTER_VERSION.to_string()],
    )
    .map_err(|e| format!("update filter_version: {e}"))?;
    eprintln!("[latent_paragraphs] filter version changed ({stored} → {FILTER_VERSION}), cleared old candidates");
    Ok(true)
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
    if is_code_heavy(trimmed) {
        return true;
    }
    if is_quote_block(trimmed) {
        return true;
    }
    if is_table_heavy(trimmed) {
        return true;
    }
    if is_frontmatter(trimmed) {
        return true;
    }
    if is_heading_only(trimmed) {
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
    if trimmed.starts_with("```") && trimmed.ends_with("```") && trimmed.matches("```").count() >= 2
    {
        return true;
    }
    // Partial fenced code (split boundary) — any ``` fence present means mostly code
    if trimmed.contains("```") {
        return true;
    }
    false
}

fn is_code_heavy(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    let code_lines = lines
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("```")
                || l.starts_with("    ")
                || l.starts_with('\t')
                || looks_like_code(t)
        })
        .count();
    code_lines * 100 / lines.len() > 60
}

fn looks_like_code(line: &str) -> bool {
    let indicators = [
        "def ", "fn ", "func ", "class ", "import ", "from ", "return ",
        "if (", "if(", "for (", "for(", "while (", "while(",
        "const ", "let ", "var ", "async ", "await ",
        "pub ", "use ", "mod ", "struct ", "enum ",
        "});", ");", "};", "} else", "} catch",
    ];
    indicators.iter().any(|p| line.starts_with(p))
        || (line.ends_with(';') && !line.ends_with("；"))
        || (line.ends_with('{') || line.ends_with('}'))
        || (line.starts_with('#') && line.contains("include"))
}

fn is_quote_block(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| line.trim_start().starts_with("> "))
}

/// Markdown table detection. Skip if any of:
/// - Contains a table separator row (e.g. `|---|---|`)
/// - > 30% of non-empty lines contain pipe `|` characters
fn is_table_heavy(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    // Fast path: if any line looks like a table separator, it's a table
    let has_separator = lines.iter().any(|l| {
        let t = l.trim();
        t.contains("|") && t.contains("---")
    });
    if has_separator {
        return true;
    }
    // Slow path: count lines with pipe chars
    let table_lines = lines.iter().filter(|l| l.trim().contains('|')).count();
    table_lines * 100 / lines.len() > 30
}

/// YAML frontmatter block: starts with `---` and ends with `---` or `...`
fn is_frontmatter(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with("---") {
        return false;
    }
    // Check if it ends with a closing fence
    let rest = trimmed.strip_prefix("---").unwrap_or("").trim();
    rest.ends_with("---") || rest.ends_with("...")
}

/// Pure heading lines: every non-empty line starts with `#`
fn is_heading_only(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|line| line.trim_start().starts_with('#'))
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
                // Collect other document paths in this cluster (excluding self)
                let self_path = chunks[idx].rel_path.as_str();
                let other_docs: Vec<&str> = doc_set.iter().copied().filter(|p| *p != self_path).collect();
                let paired = if other_docs.is_empty() {
                    None
                } else {
                    Some(other_docs.join(","))
                };
                marked.entry(idx).or_insert(RawCandidate {
                    chunk_idx: idx,
                    marking_reason: "cross_doc_recurrence",
                    similarity_score: Some(max_sim[idx] as f64),
                    paired_rel_path: paired,
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

// ---------------------------------------------------------------------------
// Filtered listing for Discovery pane
// ---------------------------------------------------------------------------

fn count_by_reason(conn: &Connection) -> Result<DiscoveryReasonCounts, String> {
    let mut stmt = conn
        .prepare(
            "SELECT marking_reason, COUNT(*) FROM thought_candidates
             WHERE dismissed_at IS NULL AND promoted_thought_id IS NULL
             GROUP BY marking_reason",
        )
        .map_err(|e| format!("prepare count_by_reason: {e}"))?;

    let mut counts = DiscoveryReasonCounts {
        high_similarity: 0,
        cross_doc_recurrence: 0,
        semantic_isolated: 0,
    };
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)))
        .map_err(|e| format!("count_by_reason query: {e}"))?;

    for row in rows {
        let (reason, cnt) = row.map_err(|e| format!("count_by_reason row: {e}"))?;
        match reason.as_str() {
            "high_similarity" => counts.high_similarity = cnt,
            "cross_doc_recurrence" => counts.cross_doc_recurrence = cnt,
            "semantic_isolated" => counts.semantic_isolated = cnt,
            _ => {}
        }
    }
    Ok(counts)
}

/// List candidates with filtering, sorting, and pagination for the Discovery pane.
/// `workspace_root` is used for freshness sort (stat file mtime).
pub fn list_candidates_filtered(
    conn: &Connection,
    filter: &DiscoveryFilter,
    workspace_root: &Path,
) -> Result<DiscoveryListResponse, String> {
    let by_reason = count_by_reason(conn)?;

    // Build WHERE clause
    let mut where_clauses = vec![
        "tc.dismissed_at IS NULL".to_string(),
        "tc.promoted_thought_id IS NULL".to_string(),
    ];
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ref reason) = filter.marking_reason {
        where_clauses.push(format!("tc.marking_reason = ?{}", params_vec.len() + 1));
        params_vec.push(Box::new(reason.clone()));
    }

    let where_sql = where_clauses.join(" AND ");

    // Count total matching
    let count_sql = format!(
        "SELECT COUNT(*) FROM thought_candidates tc WHERE {where_sql}"
    );
    let total: usize = {
        let mut stmt = conn.prepare(&count_sql).map_err(|e| format!("prepare count: {e}"))?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        stmt.query_row(params_refs.as_slice(), |r| r.get(0))
            .map_err(|e| format!("count query: {e}"))?
    };

    // Build ORDER BY
    let order_sql = match filter.sort_by.as_deref() {
        Some("similarity") => "tc.similarity_score DESC",
        Some("age") => "tc.created_at ASC",
        _ => "tc.similarity_score DESC", // default: similarity (freshness deferred to Phase 3)
    };

    // Query with LIMIT/OFFSET
    let query_sql = format!(
        "SELECT tc.id, tc.rel_path, tc.paragraph_start_line, tc.paragraph_end_line,
                tc.marking_reason, tc.similarity_score, tc.paired_rel_path, tc.chunk_id
         FROM thought_candidates tc
         WHERE {where_sql}
         ORDER BY {order_sql}
         LIMIT ?{} OFFSET ?{}",
        params_vec.len() + 1,
        params_vec.len() + 2,
    );
    params_vec.push(Box::new(filter.limit as i64));
    params_vec.push(Box::new(filter.offset as i64));

    let mut stmt = conn.prepare(&query_sql).map_err(|e| format!("prepare filtered list: {e}"))?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i32>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|e| format!("query filtered candidates: {e}"))?;

    let mut items = Vec::new();
    for row in rows {
        let (id, rel_path, start_line, end_line, reason, score, paired, chunk_id) =
            row.map_err(|e| format!("read filtered candidate row: {e}"))?;

        let chunk_text: String = conn
            .query_row(
                "SELECT chunk_text FROM doc_chunks WHERE chunk_id = ?1",
                params![chunk_id],
                |r| r.get(0),
            )
            .unwrap_or_default();

        items.push(CandidateForUi {
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

    // For freshness sort: re-sort by file mtime (Phase 1 simple approach)
    if filter.sort_by.as_deref() == Some("freshness") || filter.sort_by.is_none() {
        sort_by_freshness(&mut items, workspace_root);
    }

    Ok(DiscoveryListResponse {
        items,
        total,
        by_reason,
    })
}

/// Sort items by file modification time (most recent first).
/// Files that cannot be stat'd sort to the end.
fn sort_by_freshness(items: &mut Vec<CandidateForUi>, workspace_root: &Path) {
    let mut mtime_cache: HashMap<String, Option<std::time::SystemTime>> = HashMap::new();
    let get_mtime = |path: &str, cache: &mut HashMap<String, Option<std::time::SystemTime>>| -> Option<std::time::SystemTime> {
        if let Some(cached) = cache.get(path) {
            return *cached;
        }
        let full = workspace_root.join(path);
        let mt = std::fs::metadata(&full).ok().and_then(|m| m.modified().ok());
        cache.insert(path.to_string(), mt);
        mt
    };

    items.sort_by(|a, b| {
        let ma = get_mtime(&a.rel_path, &mut mtime_cache);
        let mb = get_mtime(&b.rel_path, &mut mtime_cache);
        mb.cmp(&ma) // descending: most recent first
    });
}

/// Batch dismiss multiple candidates at once.
pub fn batch_dismiss(conn: &Connection, ids: &[String]) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE thought_candidates SET dismissed_at = ?1 WHERE id IN ({placeholders}) AND dismissed_at IS NULL"
    );
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(ids.len() + 1);
    param_values.push(Box::new(now));
    for id in ids {
        param_values.push(Box::new(id.clone()));
    }
    let params_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let affected = conn
        .execute(&sql, params_refs.as_slice())
        .map_err(|e| format!("batch dismiss: {e}"))?;
    Ok(affected)
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

pub fn get_candidate_chunk_text(
    conn: &Connection,
    candidate_id: &str,
) -> Result<(String, CandidateForUi), String> {
    let row = conn
        .query_row(
            "SELECT tc.id, tc.rel_path, tc.paragraph_start_line, tc.paragraph_end_line,
                    tc.marking_reason, tc.similarity_score, tc.paired_rel_path, tc.chunk_id
             FROM thought_candidates tc WHERE tc.id = ?1",
            params![candidate_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|e| format!("get candidate: {e}"))?;
    let (id, rel_path, start_line, end_line, reason, score, paired, chunk_id) = row;
    let chunk_text: String = conn
        .query_row(
            "SELECT chunk_text FROM doc_chunks WHERE chunk_id = ?1",
            params![chunk_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("get chunk text: {e}"))?;
    let candidate = CandidateForUi {
        id,
        rel_path,
        excerpt: excerpt(&chunk_text),
        marking_reason: reason,
        similarity_score: score,
        paired_rel_path: paired,
        start_line,
        end_line,
    };
    Ok((chunk_text, candidate))
}

pub fn promote_candidate(
    embed_conn: &Connection,
    canonical_root: &std::path::Path,
    candidate_id: &str,
) -> Result<String, String> {
    let (chunk_text, candidate) = get_candidate_chunk_text(embed_conn, candidate_id)?;

    let abs_path = canonical_root.join(&candidate.rel_path);
    if !abs_path.exists() {
        return Err(format!("source file not found: {}", candidate.rel_path));
    }
    let existing = std::fs::read_to_string(&abs_path)
        .map_err(|e| format!("read source file: {e}"))?;

    let parsed = crate::thought_parser::parse_note_thoughts_for_workspace(
        canonical_root,
        &candidate.rel_path,
        &existing,
    );
    let count = parsed.meta.len().max(parsed.blocks.len());

    let (new_markdown, resp) = crate::thought_parser::insert_thought_into_markdown(
        canonical_root,
        &candidate.rel_path,
        &existing,
        &chunk_text,
        false,
        Some(candidate.start_line as usize),
        count,
    )?;

    std::fs::write(&abs_path, &new_markdown)
        .map_err(|e| format!("write updated note: {e}"))?;

    embed_conn
        .execute(
            "UPDATE thought_candidates SET promoted_thought_id = ?1 WHERE id = ?2",
            params![resp.thought_id, candidate_id],
        )
        .map_err(|e| format!("update promoted_thought_id: {e}"))?;

    Ok(resp.thought_id)
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

        // Partial fenced code (split boundary — only opening fence)
        let partial = "```python\nfrom langgraph.graph import StateGraph, END\ndef build_agent_graph(llm):";
        assert!(should_skip_chunk(partial));

        // Code-heavy content without fences
        let code_heavy = "def build_agent():\n    llm = get_llm()\n    return llm.run()\n\ndef main():\n    agent = build_agent()";
        assert!(should_skip_chunk(code_heavy));
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

    #[test]
    fn test_skip_table_heavy() {
        let table = "| Name | Age | City |\n|------|-----|------|\n| Alice | 30 | NYC |\n| Bob | 25 | LA |";
        assert!(should_skip_chunk(table), "pure table should be skipped");
    }

    #[test]
    fn test_skip_table_mixed_majority() {
        let mixed = "Some intro text here.\n| Col A | Col B |\n|-------|-------|\n| val1 | val2 |\n| val3 | val4 |\n| val5 | val6 |";
        assert!(should_skip_chunk(mixed), "table-heavy content (>50% table lines) should be skipped");
    }

    #[test]
    fn test_keep_prose_with_pipe() {
        // Only 1 out of 5 lines contains `|` (20%), below the 30% threshold
        let prose = "This is a paragraph about Unix pipes. We use | to chain commands.\nAnother line of normal prose about topics.\nA third line discussing ideas and concepts in detail.\nFourth line with more context about the subject.\nFifth line wrapping up the discussion on this matter.";
        assert!(!should_skip_chunk(prose), "prose mentioning | should not be skipped");
    }

    #[test]
    fn test_skip_frontmatter() {
        let fm = "---\ntitle: My Note\ndate: 2026-01-01\ntags: [rust, learning]\n---";
        assert!(should_skip_chunk(fm), "YAML frontmatter should be skipped");
    }

    #[test]
    fn test_skip_heading_only() {
        let headings = "# Chapter 1\n## Section A\n### Subsection";
        assert!(should_skip_chunk(headings), "heading-only content should be skipped");
    }

    #[test]
    fn test_keep_heading_with_prose() {
        let mixed = "# My Thoughts\nThis is a paragraph with actual prose content that contains meaningful ideas worth challenging.";
        assert!(!should_skip_chunk(mixed), "heading + prose should not be skipped");
    }
}
