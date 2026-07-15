//! LLM-assisted discovery confirmation (Spec 11).
//!
//! After the vector-based candidate detection pipeline produces candidates,
//! this module sends batches to an LLM for semantic verification — filtering
//! false positives and generating human-readable recommendation reasons.

use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::llm::{create_provider, CompletionOverrides, LlmChatMessage};
use crate::vault_config;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum candidates to confirm per single LLM call.
#[allow(dead_code)]
const BATCH_SIZE: usize = 5;

/// Maximum confirmations allowed per calendar day.
#[allow(dead_code)]
const DAILY_CAP: usize = 30;

/// Cached confirmation expires after this many days.
const CACHE_VALID_DAYS: i64 = 7;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Input data for a single candidate to be confirmed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateToConfirm {
    pub id: String,
    pub rel_path: String,
    pub excerpt: String,
    pub marking_reason: String,
    pub similarity_score: Option<f64>,
    pub paired_rel_path: Option<String>,
    pub paired_excerpt: Option<String>,
    pub cluster_doc_count: Option<usize>,
}

/// LLM verdict for a single candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmResult {
    pub candidate_id: String,
    pub verdict: String, // "confirmed" | "downgraded" | "rejected"
    pub reason: String,  // 50-100 char recommendation reason
}

/// Batch response wrapper used when parsing LLM JSON output.
#[derive(Debug, Deserialize)]
struct LlmConfirmResponse {
    results: Vec<LlmSingleVerdict>,
}

#[derive(Debug, Deserialize)]
struct LlmSingleVerdict {
    /// 1-indexed candidate number from the prompt
    #[serde(alias = "candidate")]
    index: usize,
    verdict: String,
    reason: String,
}

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

const SYSTEM_PROMPT: &str = r#"You are a knowledge auditor reviewing candidate paragraphs discovered in a personal knowledge vault.
Your task is to evaluate whether each candidate is worth the user's attention for deeper thinking and knowledge consolidation.

For each candidate, you receive:
- The candidate paragraph text
- The marking reason (why the system flagged it)
- For similarity pairs: both paragraphs side by side
- For isolated paragraphs: the paragraph alone

Evaluate based on:
1. Information density: Does this contain a real insight, opinion, or knowledge claim? (vs boilerplate, table of contents, metadata, trivial notes)
2. Actionability: Would reviewing this help the user consolidate, connect, or deepen their understanding?
3. Novelty: For similarity pairs — are they truly expressing the same core idea in different contexts? Or just surface-level keyword overlap?

Respond with a JSON object containing a "results" array. Each element has:
- "index": the candidate number (1-indexed)
- "verdict": "confirmed" | "downgraded" | "rejected"
- "reason": a concise explanation (50-100 chars) in the same language as the candidate text

Guidelines for verdict:
- "confirmed": Genuinely valuable — the user should see this and think about it
- "downgraded": Marginally interesting but not urgent — can be shown at lower priority
- "rejected": False positive — template text, boilerplate, meeting notes, or trivially obvious connection

Guidelines for reason:
- Be specific and actionable
- Good: "这两段从不同角度论证了'约束即自由'，合并后可形成更完整的论述"
- Good: "This isolated insight about decision reversibility hasn't been connected to your UX notes"
- Bad: "interesting" or "worth reading" (too vague)
"#;

fn build_user_prompt(candidates: &[CandidateToConfirm]) -> String {
    let mut parts = Vec::new();

    for (i, c) in candidates.iter().enumerate() {
        let idx = i + 1;
        match c.marking_reason.as_str() {
            "high_similarity" => {
                parts.push(format!(
                    "## Candidate {idx} (similarity pair, score={score:.2})\n\
                     ### Paragraph A — {path_a}:\n{text_a}\n\
                     ### Paragraph B — {path_b}:\n{text_b}\n",
                    score = c.similarity_score.unwrap_or(0.0),
                    path_a = c.rel_path,
                    text_a = c.excerpt,
                    path_b = c.paired_rel_path.as_deref().unwrap_or("unknown"),
                    text_b = c.paired_excerpt.as_deref().unwrap_or("[text unavailable]"),
                ));
            }
            "cross_doc_recurrence" => {
                parts.push(format!(
                    "## Candidate {idx} (recurring theme across {count} docs)\n\
                     ### Representative paragraph — {path}:\n{text}\n\
                     ### Other related docs: {others}\n",
                    count = c.cluster_doc_count.unwrap_or(0),
                    path = c.rel_path,
                    text = c.excerpt,
                    others = c.paired_rel_path.as_deref().unwrap_or(""),
                ));
            }
            "semantic_isolated" => {
                parts.push(format!(
                    "## Candidate {idx} (isolated paragraph — no strong connection to other notes)\n\
                     ### Source — {path}:\n{text}\n",
                    path = c.rel_path,
                    text = c.excerpt,
                ));
            }
            other => {
                parts.push(format!(
                    "## Candidate {idx} (reason: {other})\n\
                     ### Source — {path}:\n{text}\n",
                    path = c.rel_path,
                    text = c.excerpt,
                ));
            }
        }
    }

    format!(
        "Please evaluate the following {} candidates:\n\n{}\n\n\
         Respond with JSON: {{\"results\": [{{\"index\": 1, \"verdict\": \"...\", \"reason\": \"...\"}}]}}",
        candidates.len(),
        parts.join("\n---\n")
    )
}

// ---------------------------------------------------------------------------
// Core logic
// ---------------------------------------------------------------------------

/// Load candidate details from DB for confirmation.
pub fn load_candidates_for_confirm(
    conn: &Connection,
    candidate_ids: &[String],
) -> Result<Vec<CandidateToConfirm>, String> {
    let mut result = Vec::with_capacity(candidate_ids.len());

    for id in candidate_ids {
        let row: Result<(String, String, String, Option<f64>, Option<String>, String), _> = conn
            .query_row(
                "SELECT tc.rel_path, tc.marking_reason, tc.chunk_id,
                        tc.similarity_score, tc.paired_rel_path, tc.id
                 FROM thought_candidates tc WHERE tc.id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            );

        let (rel_path, marking_reason, chunk_id, similarity_score, paired_rel_path, cand_id) =
            match row {
                Ok(r) => r,
                Err(_) => continue, // skip missing candidates
            };

        // Get the excerpt from doc_chunks
        let excerpt: String = conn
            .query_row(
                "SELECT chunk_text FROM doc_chunks WHERE chunk_id = ?1",
                params![chunk_id],
                |r| r.get(0),
            )
            .unwrap_or_default();

        // For high_similarity, try to get paired excerpt
        let paired_excerpt = if marking_reason == "high_similarity" {
            if let Some(ref paired_path) = paired_rel_path {
                // Find a chunk in the paired doc with similar score
                conn.query_row(
                    "SELECT dc.chunk_text FROM doc_chunks dc
                     WHERE dc.rel_path = ?1
                     LIMIT 1",
                    params![paired_path],
                    |r| r.get::<_, String>(0),
                )
                .ok()
            } else {
                None
            }
        } else {
            None
        };

        // For cross_doc_recurrence, count related docs
        let cluster_doc_count = if marking_reason == "cross_doc_recurrence" {
            paired_rel_path
                .as_deref()
                .map(|paths| paths.split(',').count() + 1) // +1 for self
        } else {
            None
        };

        let truncated_excerpt = if excerpt.len() > 500 {
            // Find a valid char boundary at or before byte 500
            let mut end = 500;
            while end > 0 && !excerpt.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &excerpt[..end])
        } else {
            excerpt
        };

        result.push(CandidateToConfirm {
            id: cand_id,
            rel_path,
            excerpt: truncated_excerpt,
            marking_reason,
            similarity_score,
            paired_rel_path,
            paired_excerpt,
            cluster_doc_count,
        });
    }

    Ok(result)
}

/// Call LLM to confirm a batch of candidates. Returns results for each candidate.
pub async fn confirm_batch_with_llm(
    candidates: &[CandidateToConfirm],
    workspace_root: &std::path::Path,
    http_client: &Arc<reqwest::Client>,
) -> Result<Vec<ConfirmResult>, String> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Load AI config
    let root = workspace_root.to_path_buf();
    let ai = tauri::async_runtime::spawn_blocking(move || {
        vault_config::load_ai_config_internal(&root)
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {e}"))??;

    let provider = create_provider(&ai, None, http_client)?;

    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".into(),
            content: build_user_prompt(candidates),
            ..Default::default()
        },
    ];

    let overrides = CompletionOverrides {
        temperature: Some(0.3),
        json_mode: true,
        ..Default::default()
    };

    let raw = provider.chat_completion(&msgs, Some(&overrides)).await?;

    // Parse JSON response
    let parsed = parse_llm_response(&raw, candidates)?;

    Ok(parsed)
}

/// Parse the LLM response JSON and map back to candidate IDs.
fn parse_llm_response(
    raw: &str,
    candidates: &[CandidateToConfirm],
) -> Result<Vec<ConfirmResult>, String> {
    // Extract JSON object from possible markdown fencing
    let s = raw.trim();
    let start = s.find('{').ok_or("No JSON object in LLM response")?;
    let end = s.rfind('}').ok_or("No closing brace in LLM response")?;
    if end < start {
        return Err("Invalid JSON structure".into());
    }
    let json_slice = &s[start..=end];

    let resp: LlmConfirmResponse =
        serde_json::from_str(json_slice).map_err(|e| format!("JSON parse error: {e}"))?;

    let mut results = Vec::new();
    for v in resp.results {
        // index is 1-based
        let idx = v.index.saturating_sub(1);
        if idx >= candidates.len() {
            continue;
        }
        let verdict = match v.verdict.as_str() {
            "confirmed" | "downgraded" | "rejected" => v.verdict.clone(),
            _ => "downgraded".to_string(), // default unknown verdicts to downgraded
        };
        results.push(ConfirmResult {
            candidate_id: candidates[idx].id.clone(),
            verdict,
            reason: v.reason,
        });
    }

    Ok(results)
}

/// Write confirmation results back to the database.
pub fn persist_confirm_results(
    conn: &Connection,
    results: &[ConfirmResult],
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();
    for r in results {
        conn.execute(
            "UPDATE thought_candidates
             SET llm_confirmed = ?1, llm_reason = ?2, llm_confirmed_at = ?3
             WHERE id = ?4",
            params![r.verdict, r.reason, now, r.candidate_id],
        )
        .map_err(|e| format!("persist confirm result for {}: {e}", r.candidate_id))?;
    }
    Ok(())
}

/// Check how many confirmations have been done today (for daily cap).
pub fn today_confirm_count(conn: &Connection) -> Result<usize, String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM thought_candidates
             WHERE llm_confirmed_at IS NOT NULL AND llm_confirmed_at LIKE ?1",
            params![format!("{today}%")],
            |r| r.get(0),
        )
        .map_err(|e| format!("count today confirms: {e}"))?;
    Ok(count)
}

/// Filter candidate IDs to only those that need (re-)confirmation:
/// - llm_confirmed IS NULL, or
/// - llm_confirmed_at is older than CACHE_VALID_DAYS
pub fn filter_needing_confirmation(
    conn: &Connection,
    candidate_ids: &[String],
) -> Result<Vec<String>, String> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let cutoff = (Utc::now() - chrono::Duration::days(CACHE_VALID_DAYS))
        .to_rfc3339();

    let mut result = Vec::new();
    for id in candidate_ids {
        let needs: bool = conn
            .query_row(
                "SELECT 1 FROM thought_candidates
                 WHERE id = ?1
                   AND (llm_confirmed IS NULL OR llm_confirmed_at < ?2)
                   AND dismissed_at IS NULL
                   AND promoted_thought_id IS NULL",
                params![id, cutoff],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if needs {
            result.push(id.clone());
        }
    }
    Ok(result)
}

/// High-level entry point: confirm a batch of candidates (with cap and caching).
/// Returns only the results for candidates that were actually confirmed.
#[allow(dead_code)]
pub async fn confirm_discovery_batch(
    conn: &Connection,
    candidate_ids: &[String],
    workspace_root: &std::path::Path,
    http_client: &Arc<reqwest::Client>,
) -> Result<Vec<ConfirmResult>, String> {
    // Check daily cap
    let today_count = today_confirm_count(conn)?;
    if today_count >= DAILY_CAP {
        return Ok(Vec::new()); // cap reached, silently skip
    }

    // Filter to those needing confirmation
    let needs_confirm = filter_needing_confirmation(conn, candidate_ids)?;
    if needs_confirm.is_empty() {
        return Ok(Vec::new());
    }

    // Limit to batch size and remaining daily cap
    let remaining_cap = DAILY_CAP - today_count;
    let batch_limit = BATCH_SIZE.min(remaining_cap);
    let batch_ids: Vec<String> = needs_confirm.into_iter().take(batch_limit).collect();

    // Load candidate details
    let candidates = load_candidates_for_confirm(conn, &batch_ids)?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Call LLM
    let results = confirm_batch_with_llm(&candidates, workspace_root, http_client).await?;

    // Persist results
    persist_confirm_results(conn, &results)?;

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_llm_response_valid() {
        let candidates = vec![
            CandidateToConfirm {
                id: "cand-1".into(),
                rel_path: "notes/a.md".into(),
                excerpt: "Test excerpt A".into(),
                marking_reason: "high_similarity".into(),
                similarity_score: Some(0.91),
                paired_rel_path: Some("notes/b.md".into()),
                paired_excerpt: Some("Test excerpt B".into()),
                cluster_doc_count: None,
            },
            CandidateToConfirm {
                id: "cand-2".into(),
                rel_path: "notes/c.md".into(),
                excerpt: "Isolated thought".into(),
                marking_reason: "semantic_isolated".into(),
                similarity_score: Some(0.12),
                paired_rel_path: None,
                paired_excerpt: None,
                cluster_doc_count: None,
            },
        ];

        let raw = r#"{"results": [
            {"index": 1, "verdict": "confirmed", "reason": "Both discuss constraint-based design from different angles"},
            {"index": 2, "verdict": "rejected", "reason": "Boilerplate meeting notes, no real insight"}
        ]}"#;

        let results = parse_llm_response(raw, &candidates).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].candidate_id, "cand-1");
        assert_eq!(results[0].verdict, "confirmed");
        assert_eq!(results[1].candidate_id, "cand-2");
        assert_eq!(results[1].verdict, "rejected");
    }

    #[test]
    fn test_parse_llm_response_with_markdown_fencing() {
        let candidates = vec![CandidateToConfirm {
            id: "cand-1".into(),
            rel_path: "x.md".into(),
            excerpt: "test".into(),
            marking_reason: "semantic_isolated".into(),
            similarity_score: None,
            paired_rel_path: None,
            paired_excerpt: None,
            cluster_doc_count: None,
        }];

        let raw = "```json\n{\"results\": [{\"index\": 1, \"verdict\": \"downgraded\", \"reason\": \"Low info density\"}]}\n```";
        let results = parse_llm_response(raw, &candidates).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].verdict, "downgraded");
    }

    #[test]
    fn test_parse_llm_response_invalid_index() {
        let candidates = vec![CandidateToConfirm {
            id: "cand-1".into(),
            rel_path: "x.md".into(),
            excerpt: "test".into(),
            marking_reason: "semantic_isolated".into(),
            similarity_score: None,
            paired_rel_path: None,
            paired_excerpt: None,
            cluster_doc_count: None,
        }];

        // index 99 is out of bounds — should be skipped
        let raw = r#"{"results": [{"index": 99, "verdict": "confirmed", "reason": "test"}]}"#;
        let results = parse_llm_response(raw, &candidates).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_build_user_prompt_all_types() {
        let candidates = vec![
            CandidateToConfirm {
                id: "c1".into(),
                rel_path: "a.md".into(),
                excerpt: "Design systems constrain choices".into(),
                marking_reason: "high_similarity".into(),
                similarity_score: Some(0.92),
                paired_rel_path: Some("b.md".into()),
                paired_excerpt: Some("Good component libraries constrain usage".into()),
                cluster_doc_count: None,
            },
            CandidateToConfirm {
                id: "c2".into(),
                rel_path: "c.md".into(),
                excerpt: "Distributed consensus".into(),
                marking_reason: "cross_doc_recurrence".into(),
                similarity_score: Some(0.8),
                paired_rel_path: Some("d.md,e.md,f.md".into()),
                paired_excerpt: None,
                cluster_doc_count: Some(4),
            },
            CandidateToConfirm {
                id: "c3".into(),
                rel_path: "g.md".into(),
                excerpt: "People avoid irreversible decisions".into(),
                marking_reason: "semantic_isolated".into(),
                similarity_score: Some(0.15),
                paired_rel_path: None,
                paired_excerpt: None,
                cluster_doc_count: None,
            },
        ];

        let prompt = build_user_prompt(&candidates);
        assert!(prompt.contains("Candidate 1 (similarity pair"));
        assert!(prompt.contains("Candidate 2 (recurring theme"));
        assert!(prompt.contains("Candidate 3 (isolated paragraph"));
        assert!(prompt.contains("Design systems constrain"));
        assert!(prompt.contains("Good component libraries"));
    }
}
