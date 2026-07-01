//! Front-load summarization of tool results before they enter the LLM context.
//!
//! Long tool results (web pages, search hits) are compressed here so that
//! ContextGuard only has to deal with already-compact messages downstream.
//!
//! When a `results_dir` is configured, raw results exceeding the summarize
//! threshold are persisted to disk before summarization. The summarized marker
//! then contains a `ref:<id>` that the `tool_result.recall` tool can use to
//! retrieve the original content on demand.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::provider::{CompletionOverrides, LlmProvider};
use super::LlmChatMessage;

pub const DEFAULT_SUMMARIZE_THRESHOLD: usize = 3000;
const MAX_RAW_INPUT_FOR_SUMMARY: usize = 4000;
const MAX_GOAL_CHARS: usize = 200;

/// Marker prefix injected into summarized results so that ContextGuard can
/// recognise them and skip aggressive trimming.
pub const SUMMARIZED_MARKER: &str = "[summarized from ";

/// Marker used by ContextGuard to detect stored-ref results that can be
/// recalled via `tool_result.recall`.
pub const STORED_REF_MARKER: &str = "[tool result stored | ref:";

const SUMMARIZE_SYSTEM: &str = "\
You are a tool result summarizer. Extract only the information relevant to the user's task.\n\
Output a concise summary (under 800 chars) containing:\n\
1. Key facts and findings relevant to the task\n\
2. Important data points, names, URLs worth keeping\n\
3. Whether more reading/searching is needed\n\
Output ONLY the summary, nothing else.";

/// Maximum number of search/semantic hits to keep in rule-based extraction.
const RULE_BASED_TOP_K: usize = 5;
/// Maximum snippet length per hit in rule-based extraction.
const RULE_BASED_SNIPPET_LEN: usize = 200;

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ToolResultProcessor {
    provider: Arc<dyn LlmProvider>,
    summarize_threshold: usize,
    results_dir: Option<PathBuf>,
    session_id: String,
}

#[derive(Clone)]
pub struct ProcessedResult {
    pub content: String,
    pub was_summarized: bool,
    pub original_len: usize,
}

/// Name of the recall tool — used to skip re-summarization of recalled results.
pub const RECALL_TOOL_NAME: &str = "tool.recall";

impl ToolResultProcessor {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        summarize_threshold: usize,
        results_dir: Option<PathBuf>,
        session_id: String,
    ) -> Self {
        Self {
            provider,
            summarize_threshold,
            results_dir,
            session_id,
        }
    }

    /// Process a single tool result: summarize if long, pass through if short.
    ///
    /// When `results_dir` is configured and the result exceeds the threshold,
    /// the raw content is persisted to disk before summarization. The resulting
    /// marker includes a `ref:` that `tool_result.recall` can use to retrieve
    /// the original.
    pub async fn process(
        &self,
        tool_name: &str,
        call_id: &str,
        raw_content: &str,
        user_goal: Option<&str>,
    ) -> ProcessedResult {
        let original_len = raw_content.len();

        if tool_name == RECALL_TOOL_NAME {
            return ProcessedResult {
                content: raw_content.to_string(),
                was_summarized: false,
                original_len,
            };
        }

        if original_len <= self.summarize_threshold {
            return ProcessedResult {
                content: raw_content.to_string(),
                was_summarized: false,
                original_len,
            };
        }

        let summary = if tool_name.starts_with("web.") {
            self.llm_summarize(tool_name, raw_content, user_goal).await
        } else {
            rule_based_extract(tool_name, raw_content)
        };

        match summary {
            Some(text) => {
                let ref_tag = match self.persist_raw(call_id, tool_name, raw_content).await {
                    Some(short_id) => format!(" | ref:{}", short_id),
                    None => String::new(),
                };
                ProcessedResult {
                    content: format!(
                        "{}{} chars{}]\n{}",
                        SUMMARIZED_MARKER, original_len, ref_tag, text
                    ),
                    was_summarized: true,
                    original_len,
                }
            }
            None => ProcessedResult {
                content: raw_content.to_string(),
                was_summarized: false,
                original_len,
            },
        }
    }

    /// Persist raw tool result to `{results_dir}/{session_id}/{call_id}.json`.
    /// Returns the short ref ID (first 8 chars of call_id) on success, None on
    /// failure or if no results_dir is configured.
    async fn persist_raw(
        &self,
        call_id: &str,
        tool_name: &str,
        raw_content: &str,
    ) -> Option<String> {
        let results_dir = self.results_dir.as_ref()?;
        let session_dir = results_dir.join(&self.session_id);

        if let Err(e) = tokio::fs::create_dir_all(&session_dir).await {
            eprintln!(
                "[tool_result_processor] failed to create dir {}: {}",
                session_dir.display(),
                e
            );
            return None;
        }

        let file_path = session_dir.join(format!("{}.json", call_id));
        let ts = chrono::Utc::now().to_rfc3339();
        let record = serde_json::json!({
            "call_id": call_id,
            "tool_name": tool_name,
            "ts": ts,
            "content": raw_content,
            "len": raw_content.len(),
        });

        match serde_json::to_string(&record) {
            Ok(json_str) => {
                if let Err(e) = tokio::fs::write(&file_path, json_str).await {
                    eprintln!(
                        "[tool_result_processor] failed to write {}: {}",
                        file_path.display(),
                        e
                    );
                    return None;
                }
            }
            Err(e) => {
                eprintln!("[tool_result_processor] failed to serialize: {}", e);
                return None;
            }
        }

        let short_id = &call_id[..call_id.len().min(8)];
        eprintln!(
            "[tool_result_processor] persisted {} ({} chars) → {}",
            tool_name,
            raw_content.len(),
            file_path.display()
        );
        Some(short_id.to_string())
    }

    async fn llm_summarize(
        &self,
        tool_name: &str,
        raw_content: &str,
        user_goal: Option<&str>,
    ) -> Option<String> {
        let truncated = truncate_at_boundary(raw_content, MAX_RAW_INPUT_FOR_SUMMARY);
        let goal_line = user_goal
            .map(|g| format!("User's task context: {}\n\n", truncate_at_boundary(g, MAX_GOAL_CHARS)))
            .unwrap_or_default();

        let user_prompt = format!(
            "{}Tool: {}\nRaw result ({} chars):\n{}",
            goal_line,
            tool_name,
            raw_content.len(),
            truncated,
        );

        let messages = vec![
            LlmChatMessage {
                role: "system".to_string(),
                content: SUMMARIZE_SYSTEM.to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "user".to_string(),
                content: user_prompt,
                ..Default::default()
            },
        ];

        let overrides = CompletionOverrides {
            temperature: Some(0.0),
            ..Default::default()
        };

        self.provider
            .chat_completion(&messages, Some(&overrides))
            .await
            .ok()
            .filter(|t| !t.trim().is_empty())
    }
}

// ─── Rule-based extraction ──────────────────────────────────────────────────

fn rule_based_extract(tool_name: &str, raw_content: &str) -> Option<String> {
    match tool_name {
        "vault.search_keyword" => extract_vault_keyword_hits(raw_content),
        "vault.semantic_search" => extract_semantic_hits(raw_content),
        _ => None, // fallback: no rule-based extraction, keep raw
    }
}

/// Extract top-K snippets from vault.search_keyword JSON result.
///
/// Expected shape: `{"snippets": [{"rel_path": "...", "snippet": "...", ...}, ...]}`
fn extract_vault_keyword_hits(raw: &str) -> Option<String> {
    let val: Value = serde_json::from_str(raw).ok()?;
    let snippets = val.get("snippets")?.as_array()?;

    let mut lines = Vec::new();
    for item in snippets.iter().take(RULE_BASED_TOP_K) {
        let path = item.get("rel_path").and_then(|v| v.as_str()).unwrap_or("?");
        let snippet = item
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let short = truncate_at_boundary(snippet, RULE_BASED_SNIPPET_LEN);
        lines.push(format!("- {}: {}", path, short));
    }

    if lines.is_empty() {
        return None;
    }

    let total = snippets.len();
    let mut out = lines.join("\n");
    if total > RULE_BASED_TOP_K {
        out.push_str(&format!("\n({} more results omitted)", total - RULE_BASED_TOP_K));
    }
    Some(out)
}

/// Extract top-K hits from vault.semantic_search JSON result.
///
/// Expected shape: `{"hits": [{"rel_path": "...", "snippet": "...", "score": 0.8, ...}, ...]}`
fn extract_semantic_hits(raw: &str) -> Option<String> {
    let val: Value = serde_json::from_str(raw).ok()?;
    let hits = val.get("hits")?.as_array()?;

    let mut lines = Vec::new();
    for item in hits.iter().take(RULE_BASED_TOP_K) {
        let path = item.get("rel_path").and_then(|v| v.as_str()).unwrap_or("?");
        let snippet = item
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let score = item
            .get("score")
            .and_then(|v| v.as_f64())
            .map(|s| format!(" ({:.2})", s))
            .unwrap_or_default();
        let short = truncate_at_boundary(snippet, RULE_BASED_SNIPPET_LEN);
        lines.push(format!("- {}{}: {}", path, score, short));
    }

    if lines.is_empty() {
        return None;
    }

    let total = hits.len();
    let mut out = lines.join("\n");
    if total > RULE_BASED_TOP_K {
        out.push_str(&format!("\n({} more results omitted)", total - RULE_BASED_TOP_K));
    }
    Some(out)
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Extract the latest user message content (truncated) as a task-context hint.
pub fn extract_user_goal(messages: &[LlmChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| truncate_at_boundary(&m.content, MAX_GOAL_CHARS).to_string())
}

pub(crate) fn truncate_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_processor(threshold: usize) -> ToolResultProcessor {
        ToolResultProcessor::new(
            Arc::new(FakeProvider),
            threshold,
            None,
            "test-session".to_string(),
        )
    }

    fn make_processor_with_dir(threshold: usize, dir: PathBuf) -> ToolResultProcessor {
        ToolResultProcessor::new(
            Arc::new(FakeProvider),
            threshold,
            Some(dir),
            "test-session".to_string(),
        )
    }

    #[test]
    fn short_result_passes_through() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let proc = make_processor(3000);
            let result = proc.process("note.read", "call-1", "short content", None).await;
            assert_eq!(result.content, "short content");
            assert!(!result.was_summarized);
            assert_eq!(result.original_len, 13);
        });
    }

    #[test]
    fn long_non_web_non_vault_passes_through() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let proc = make_processor(100);
            let long = "x".repeat(200);
            let result = proc.process("note.read", "call-2", &long, None).await;
            assert_eq!(result.content, long);
            assert!(!result.was_summarized);
        });
    }

    #[test]
    fn recall_tool_result_passes_through() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let proc = make_processor(10);
            let long = "x".repeat(200);
            let result = proc
                .process(RECALL_TOOL_NAME, "call-3", &long, None)
                .await;
            assert_eq!(result.content, long);
            assert!(!result.was_summarized);
        });
    }

    #[test]
    fn persist_raw_writes_file() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = tempfile::tempdir().unwrap();
            let proc = make_processor_with_dir(10, tmp.path().to_path_buf());
            let result = proc
                .process("web.search", "abcdef12-3456", &"y".repeat(100), None)
                .await;
            assert!(result.was_summarized);
            assert!(result.content.contains("ref:abcdef12"));

            let file = tmp
                .path()
                .join("test-session")
                .join("abcdef12-3456.json");
            assert!(file.exists());
            let stored: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
            assert_eq!(stored["call_id"], "abcdef12-3456");
            assert_eq!(stored["tool_name"], "web.search");
            assert_eq!(stored["len"], 100);
        });
    }

    #[test]
    fn vault_keyword_rule_extraction() {
        let raw = serde_json::json!({
            "snippets": [
                {"rel_path": "a.md", "snippet": "x".repeat(300), "kind": "Match"},
                {"rel_path": "b.md", "snippet": "y".repeat(300), "kind": "Match"},
                {"rel_path": "c.md", "snippet": "z".repeat(50), "kind": "Match"},
            ]
        })
        .to_string();

        let result = extract_vault_keyword_hits(&raw).unwrap();
        assert!(result.contains("a.md"));
        assert!(result.contains("b.md"));
        assert!(result.contains("c.md"));
        // Long snippets should be truncated
        assert!(!result.contains(&"x".repeat(300)));
    }

    #[test]
    fn vault_keyword_more_than_top_k() {
        let snippets: Vec<Value> = (0..8)
            .map(|i| {
                serde_json::json!({
                    "rel_path": format!("{}.md", i),
                    "snippet": format!("content {}", i),
                    "kind": "Match"
                })
            })
            .collect();
        let raw = serde_json::json!({ "snippets": snippets }).to_string();
        let result = extract_vault_keyword_hits(&raw).unwrap();
        assert!(result.contains("3 more results omitted"));
    }

    #[test]
    fn semantic_hits_extraction() {
        let raw = serde_json::json!({
            "hits": [
                {"rel_path": "note1.md", "snippet": "some content", "score": 0.92},
                {"rel_path": "note2.md", "snippet": "other content", "score": 0.85},
            ]
        })
        .to_string();

        let result = extract_semantic_hits(&raw).unwrap();
        assert!(result.contains("note1.md"));
        assert!(result.contains("(0.92)"));
        assert!(result.contains("note2.md"));
    }

    #[test]
    fn extract_user_goal_finds_last_user_msg() {
        let messages = vec![
            LlmChatMessage {
                role: "user".to_string(),
                content: "first question".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "assistant".to_string(),
                content: "answer".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "user".to_string(),
                content: "second question".to_string(),
                ..Default::default()
            },
        ];
        assert_eq!(
            extract_user_goal(&messages),
            Some("second question".to_string())
        );
    }

    #[test]
    fn truncate_at_boundary_cjk() {
        let s = "你好世界测试";
        let t = truncate_at_boundary(s, 7);
        assert!(t.len() <= 7);
        assert_eq!(t, "你好");
    }

    // Minimal fake provider for unit tests (LLM summarization not tested here)
    struct FakeProvider;

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        async fn chat_stream(
            &self,
            _app: &tauri::AppHandle,
            _session_id: &str,
            _messages: Vec<LlmChatMessage>,
            _tools: Option<Vec<Value>>,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<super::super::provider::ChatStreamResult, String> {
            unimplemented!()
        }

        async fn chat_completion(
            &self,
            _messages: &[LlmChatMessage],
            _overrides: Option<&CompletionOverrides>,
        ) -> Result<String, String> {
            Ok("fake summary".to_string())
        }

        async fn list_models(&self) -> Result<Vec<String>, String> {
            Ok(vec![])
        }

        fn convert_tools(&self, _manifests: &[Value]) -> Vec<Value> {
            vec![]
        }

        fn build_tool_result_message(
            &self,
            _call_id: &str,
            _tool_name: &str,
            _content: &str,
        ) -> LlmChatMessage {
            LlmChatMessage::default()
        }

        fn provider_name(&self) -> &'static str {
            "fake"
        }
    }
}
