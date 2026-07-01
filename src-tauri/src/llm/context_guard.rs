use std::sync::Arc;

use super::provider::{CompletionOverrides, LlmProvider};
use super::LlmChatMessage;

const DEFAULT_MAX_TOKENS: usize = 32768;
const RESERVE_TOKENS: usize = 512;
const MIN_KEEP_ROUNDS: usize = 2;

const MIN_MESSAGES_FOR_SUMMARY: usize = 4;
const MAX_SUMMARY_INPUT_CHARS: usize = 6000;
const MAX_CONTENT_PER_MESSAGE: usize = 1000;

const SUMMARY_SYSTEM: &str = "\
Summarize the following conversation into a structured summary.\n\
You MUST preserve:\n\
- What the user wants to accomplish (goal)\n\
- Any constraints the user specified\n\
- What tools were called and their key findings\n\
- Decisions made so far\n\
- Open questions or unresolved issues\n\
- What should happen next\n\n\
If a previous summary is provided, merge its information into the new summary — \
keep still-relevant items, drop outdated ones, and add new discoveries.\n\n\
Format:\n\
[Goal] ...\n\
[Findings] ...\n\
[Decisions] ...\n\
[Open] ...\n\
[Next] ...\n\n\
Be concise. Each section 1-3 sentences max. Output only the summary.";

#[derive(Clone)]
pub struct ContextGuard {
    max_tokens: usize,
    reserve: usize,
    provider: Option<Arc<dyn LlmProvider>>,
}

pub struct PrecomputedSummary {
    pub summary_text: String,
    pub summarized_up_to: usize,
}

impl ContextGuard {
    pub fn new(max_context_tokens: Option<u64>) -> Self {
        let max_tokens = max_context_tokens
            .map(|t| t as usize)
            .unwrap_or(DEFAULT_MAX_TOKENS);
        Self {
            max_tokens,
            reserve: RESERVE_TOKENS,
            provider: None,
        }
    }

    pub fn with_provider(
        max_context_tokens: Option<u64>,
        provider: Arc<dyn LlmProvider>,
    ) -> Self {
        let max_tokens = max_context_tokens
            .map(|t| t as usize)
            .unwrap_or(DEFAULT_MAX_TOKENS);
        Self {
            max_tokens,
            reserve: RESERVE_TOKENS,
            provider: Some(provider),
        }
    }

    fn estimate_message_tokens(msg: &LlmChatMessage) -> usize {
        let content = &msg.content;
        let mut non_ascii_bytes = 0usize;
        let mut ascii_bytes = 0usize;
        for b in content.bytes() {
            if b.is_ascii() {
                ascii_bytes += 1;
            } else {
                non_ascii_bytes += 1;
            }
        }
        // CJK: UTF-8 3 bytes ≈ 1 token; non_ascii_bytes / 2 is conservative
        // ASCII: ~4 bytes/token
        let token_est = non_ascii_bytes / 2 + ascii_bytes / 4;
        token_est.max(1) + 4
    }

    fn estimate_total(messages: &[LlmChatMessage]) -> usize {
        messages.iter().map(Self::estimate_message_tokens).sum()
    }

    fn budget(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserve)
    }

    pub fn budget_pressure(&self, messages: &[LlmChatMessage]) -> f64 {
        let budget = self.budget();
        if budget == 0 {
            return 0.0;
        }
        let used = Self::estimate_total(messages);
        (used as f64 / budget as f64).min(1.0)
    }

    pub async fn pre_summarize(
        &self,
        messages: &[LlmChatMessage],
    ) -> Option<PrecomputedSummary> {
        let provider = self.provider.as_ref()?;

        let tail_boundary = Self::find_tail_boundary(messages);
        let removable_indices: Vec<usize> = (0..tail_boundary.min(messages.len()))
            .filter(|&i| messages[i].role != "system")
            .collect();

        if removable_indices.len() < MIN_MESSAGES_FOR_SUMMARY {
            return None;
        }

        let removable_msgs: Vec<&LlmChatMessage> =
            removable_indices.iter().map(|&i| &messages[i]).collect();

        let previous_summary = find_previous_summary(messages);
        let summary_input = build_summary_input(&removable_msgs, previous_summary.as_deref());
        let overrides = CompletionOverrides {
            temperature: Some(0.0),
            ..Default::default()
        };

        let summary_text = provider
            .chat_completion(&summary_input, Some(&overrides))
            .await
            .ok()
            .filter(|t| !t.trim().is_empty())?;

        Some(PrecomputedSummary {
            summary_text,
            summarized_up_to: tail_boundary,
        })
    }

    pub fn apply_cached_summary(
        &self,
        messages: &mut Vec<LlmChatMessage>,
        cached: &PrecomputedSummary,
    ) -> bool {
        if messages.len() < cached.summarized_up_to {
            return false;
        }

        let removable_indices: Vec<usize> = (0..cached.summarized_up_to.min(messages.len()))
            .filter(|&i| messages[i].role != "system")
            .collect();

        for &i in removable_indices.iter().rev() {
            if i < messages.len() {
                messages.remove(i);
            }
        }

        let insert_pos = messages
            .iter()
            .position(|m| m.role != "system")
            .unwrap_or(messages.len());

        messages.insert(
            insert_pos,
            LlmChatMessage {
                role: "system".to_string(),
                content: format!(
                    "[Earlier conversation summary]\n{}",
                    cached.summary_text.trim()
                ),
                ..Default::default()
            },
        );

        true
    }

    pub async fn trim_with_summary(&self, messages: &mut Vec<LlmChatMessage>) {
        let budget = self.budget();
        if Self::estimate_total(messages) <= budget {
            return;
        }

        // Phase 1: remove oldest tool-result messages
        self.phase1_remove_tool_results(messages, budget);
        if Self::estimate_total(messages) <= budget {
            return;
        }

        // Phase 1.5: summarize old messages before degrading
        if self.provider.is_some() {
            self.phase1_5_summarize(messages, budget).await;
            if Self::estimate_total(messages) <= budget {
                return;
            }
        }

        // Phase 2: degrade — replace content with placeholder
        self.phase2_degrade(messages, budget);
    }

    #[allow(dead_code)]
    pub fn trim_if_needed(&self, messages: &mut Vec<LlmChatMessage>) {
        let budget = self.budget();
        if Self::estimate_total(messages) <= budget {
            return;
        }

        self.phase1_remove_tool_results(messages, budget);
        if Self::estimate_total(messages) <= budget {
            return;
        }

        self.phase2_degrade(messages, budget);
    }

    fn phase1_remove_tool_results(&self, messages: &mut Vec<LlmChatMessage>, budget: usize) {
        let tail_boundary = Self::find_tail_boundary(messages);

        // Pass 1: trim raw (non-summarized) tool results
        let mut i = 0;
        while i < tail_boundary.min(messages.len()) && Self::estimate_total(messages) > budget {
            if messages[i].role == "tool" && messages[i].content.len() > 40 {
                if !messages[i].content.starts_with(super::tool_result_processor::SUMMARIZED_MARKER) {
                    let orig_len = messages[i].content.len();
                    messages[i].content = format!(
                        "[tool result trimmed, was {} chars]",
                        orig_len
                    );
                }
            }
            i += 1;
        }

        // Pass 2: if still over budget, degrade summarized results that have
        // a stored ref — the model can still recall them via tool.recall.
        if Self::estimate_total(messages) > budget {
            let mut i = 0;
            while i < tail_boundary.min(messages.len()) && Self::estimate_total(messages) > budget {
                if messages[i].role == "tool"
                    && messages[i].content.starts_with(super::tool_result_processor::SUMMARIZED_MARKER)
                {
                    if let Some(stored_marker) = Self::extract_stored_ref_marker(&messages[i].content) {
                        messages[i].content = stored_marker;
                    }
                }
                i += 1;
            }
        }
    }

    fn extract_stored_ref_marker(content: &str) -> Option<String> {
        let ref_start = content.find("| ref:")?;
        let ref_value_start = ref_start + "| ref:".len();
        let ref_end = content[ref_value_start..].find(']')?;
        let ref_id = &content[ref_value_start..ref_value_start + ref_end];

        let chars_start = super::tool_result_processor::SUMMARIZED_MARKER.len();
        let chars_end = content[chars_start..].find(' ').unwrap_or(0);
        let orig_chars = &content[chars_start..chars_start + chars_end];

        Some(format!(
            "{}{}]",
            super::tool_result_processor::STORED_REF_MARKER,
            format!("{}, was {} chars", ref_id, orig_chars)
        ))
    }

    async fn phase1_5_summarize(&self, messages: &mut Vec<LlmChatMessage>, budget: usize) {
        let provider = match &self.provider {
            Some(p) => p,
            None => return,
        };

        let tail_boundary = Self::find_tail_boundary(messages);

        // Collect removable non-system messages before tail boundary
        let removable_indices: Vec<usize> = (0..tail_boundary.min(messages.len()))
            .filter(|&i| messages[i].role != "system")
            .collect();

        if removable_indices.len() < MIN_MESSAGES_FOR_SUMMARY {
            return;
        }

        let removable_msgs: Vec<&LlmChatMessage> =
            removable_indices.iter().map(|&i| &messages[i]).collect();

        let previous_summary = find_previous_summary(messages);
        let summary_input = build_summary_input(&removable_msgs, previous_summary.as_deref());

        let overrides = CompletionOverrides {
            temperature: Some(0.0),
            ..Default::default()
        };

        let summary_text = match provider.chat_completion(&summary_input, Some(&overrides)).await {
            Ok(text) if !text.trim().is_empty() => text,
            _ => return, // fallback to Phase 2
        };

        // Remove the old messages (in reverse to preserve indices)
        for &i in removable_indices.iter().rev() {
            if i < messages.len() {
                messages.remove(i);
            }
        }

        // Insert summary as system message after existing system preamble
        let insert_pos = messages
            .iter()
            .position(|m| m.role != "system")
            .unwrap_or(messages.len());

        messages.insert(
            insert_pos,
            LlmChatMessage {
                role: "system".to_string(),
                content: format!("[Earlier conversation summary]\n{}", summary_text.trim()),
                ..Default::default()
            },
        );

        // If still over budget after summary, the caller will fall through to Phase 2
        let _ = budget;
    }

    fn phase2_degrade(&self, messages: &mut Vec<LlmChatMessage>, budget: usize) {
        let tail_boundary = Self::find_tail_boundary(messages);
        for i in 0..tail_boundary.min(messages.len()) {
            if messages[i].role == "system" {
                continue;
            }
            if messages[i].content.len() > 20 {
                messages[i].content = "[content trimmed]".to_string();
            }
            if Self::estimate_total(messages) <= budget {
                break;
            }
        }
    }

    fn find_tail_boundary(messages: &[LlmChatMessage]) -> usize {
        let len = messages.len();
        if len <= MIN_KEEP_ROUNDS * 2 {
            return 0;
        }

        let mut rounds = 0;
        let mut boundary = len;
        let mut i = len;
        while i > 0 && rounds < MIN_KEEP_ROUNDS {
            i -= 1;
            if messages[i].role == "assistant" {
                rounds += 1;
                boundary = i;
            }
        }
        boundary
    }
}

const EARLIER_SUMMARY_PREFIX: &str = "[Earlier conversation summary]";

fn find_previous_summary(messages: &[LlmChatMessage]) -> Option<String> {
    messages
        .iter()
        .filter(|m| m.role == "system" && m.content.starts_with(EARLIER_SUMMARY_PREFIX))
        .last()
        .map(|m| m.content[EARLIER_SUMMARY_PREFIX.len()..].trim().to_string())
}

fn build_summary_input(
    messages: &[&LlmChatMessage],
    previous_summary: Option<&str>,
) -> Vec<LlmChatMessage> {
    let mut content = String::new();

    if let Some(prev) = previous_summary {
        content.push_str("[Previous summary]:\n");
        content.push_str(prev);
        content.push_str("\n\n[New messages]:\n");
    }

    let mut char_count = content.len();

    for m in messages {
        let truncated = truncate_for_summary(&m.content, MAX_CONTENT_PER_MESSAGE);
        let line = format!("[{}]: {}\n", m.role, truncated);
        char_count += line.len();
        if char_count > MAX_SUMMARY_INPUT_CHARS {
            break;
        }
        content.push_str(&line);
    }

    vec![
        LlmChatMessage {
            role: "system".to_string(),
            content: SUMMARY_SYSTEM.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".to_string(),
            content,
            ..Default::default()
        },
    ]
}

fn truncate_for_summary(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(content: &str) -> LlmChatMessage {
        LlmChatMessage {
            role: "system".to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn user(content: &str) -> LlmChatMessage {
        LlmChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn assistant(content: &str) -> LlmChatMessage {
        LlmChatMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn tool(content: &str) -> LlmChatMessage {
        LlmChatMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn no_trim_under_budget() {
        let guard = ContextGuard::new(Some(4096));
        let mut msgs = vec![sys("hello"), user("hi"), assistant("hey")];
        let original_len = msgs.len();
        guard.trim_if_needed(&mut msgs);
        assert_eq!(msgs.len(), original_len);
    }

    #[test]
    fn degrades_old_tool_results_first() {
        // budget = max_tokens(692) - RESERVE(512) = 180.
        // Total before trim ≈ 206 > 180 → triggers Phase 1.
        // After degrading 300-char tool results to ~36-char placeholders, total ≈ 140 < 180.
        // Phase 2 does not kick in.
        let guard = ContextGuard::new(Some(692));
        let mut msgs = vec![
            sys("system prompt"),
            user("q1"),
            assistant("a1"),
            tool(&"x".repeat(300)),
            user("q2"),
            assistant("a2"),
            tool(&"y".repeat(300)),
            user("q3"),
            assistant("a3"),
            tool(&"z".repeat(30)),
        ];
        let original_len = msgs.len();
        guard.trim_if_needed(&mut msgs);
        assert!(msgs.iter().any(|m| m.role == "system"));
        assert!(msgs.iter().any(|m| m.content == "a3"));
        // tool messages are degraded, not removed
        let tool_count = msgs.iter().filter(|m| m.role == "tool").count();
        assert!(tool_count > 0, "tool messages should be degraded, not deleted");
        // at least one tool result should be trimmed to placeholder
        let has_placeholder = msgs.iter().any(|m| m.role == "tool" && m.content.starts_with("[tool result trimmed"));
        assert!(has_placeholder);
        // message count stays the same (degrade, not delete)
        assert_eq!(msgs.len(), original_len);
    }

    #[test]
    fn degraded_tool_result_preserves_structure() {
        // budget = 620 - 512 = 108. Total with 500-char tool ≈ 164 > 108 → triggers trim.
        // After degrading to placeholder (~36 chars), total ≈ 48 < 108. Phase 2 skipped.
        let guard = ContextGuard::new(Some(620));
        let mut msgs = vec![
            sys("sys"),
            user("q1"),
            assistant("a1"),
            tool(&"r".repeat(500)),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
        ];
        guard.trim_if_needed(&mut msgs);
        // tool message still present
        assert!(msgs.iter().any(|m| m.role == "tool"));
        // its content is a placeholder
        let tool_msg = msgs.iter().find(|m| m.role == "tool").unwrap();
        assert!(tool_msg.content.starts_with("[tool result trimmed, was 500 chars]"));
    }

    #[test]
    fn small_tool_result_not_degraded() {
        // Budget tight but tool result is tiny (2 chars < 40 threshold).
        // Phase 1 skips it, Phase 2 degrades user/assistant content instead.
        let guard = ContextGuard::new(Some(30));
        let small_content = "ok";
        let mut msgs = vec![
            sys("sys"),
            user(&"long ".repeat(50)),
            assistant("a1"),
            tool(small_content),
            user("q2"),
            assistant("a2"),
        ];
        guard.trim_if_needed(&mut msgs);
        let tool_msg = msgs.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(tool_msg.content, small_content);
    }

    #[test]
    fn phase1_skips_already_summarized_tool_result() {
        // Budget very tight: 580 - 512 = 68 token budget.
        // Two tool results compete for space. The summarized one (with
        // SUMMARIZED_MARKER) should survive; the raw one should be trimmed.
        let guard = ContextGuard::new(Some(580));
        let summarized = format!(
            "{}500 chars]\nKey finding: X is important.",
            super::super::tool_result_processor::SUMMARIZED_MARKER,
        );
        let mut msgs = vec![
            sys("sys"),
            user("q1"),
            assistant("a1"),
            tool(&summarized),
            tool(&"x".repeat(300)),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
        ];
        guard.trim_if_needed(&mut msgs);
        let tool_msgs: Vec<&LlmChatMessage> = msgs.iter().filter(|m| m.role == "tool").collect();
        // The summarized tool result should be preserved as-is
        assert!(tool_msgs.iter().any(|m| m.content.contains("Key finding")));
        // The raw tool result should be trimmed
        assert!(tool_msgs.iter().any(|m| m.content.starts_with("[tool result trimmed")));
    }

    #[test]
    fn phase1_pass2_degrades_summarized_with_ref_to_stored_marker() {
        let guard = ContextGuard::new(Some(10000));
        let summarized_with_ref = format!(
            "{}12474 chars | ref:019717ab]\n{}",
            super::super::tool_result_processor::SUMMARIZED_MARKER,
            "x".repeat(600),
        );
        let summarized_no_ref = format!(
            "{}500 chars]\nAnother finding.",
            super::super::tool_result_processor::SUMMARIZED_MARKER,
        );
        let mut msgs = vec![
            sys("sys"),
            user("q1"),
            assistant("a1"),
            tool(&summarized_with_ref),
            tool(&summarized_no_ref),
            tool(&"r".repeat(500)),
            user("q2"),
            assistant("a2"),
            user("q3"),
            assistant("a3"),
        ];

        let total_before = ContextGuard::estimate_total(&msgs);
        // Budget that pass 1 alone can't satisfy (need pass 2 too).
        // Pass 1 saves ~120 tokens by trimming the raw 500-char result.
        // Pass 2 saves ~150 tokens by degrading summarized_with_ref.
        let budget = total_before - 200;
        guard.phase1_remove_tool_results(&mut msgs, budget);

        let tool_msgs: Vec<&LlmChatMessage> = msgs.iter().filter(|m| m.role == "tool").collect();
        assert!(
            tool_msgs.iter().any(|m| m.content.starts_with("[tool result trimmed")),
            "pass 1 should have trimmed the raw result"
        );
        assert!(
            tool_msgs.iter().any(|m| m.content.starts_with(
                super::super::tool_result_processor::STORED_REF_MARKER
            )),
            "pass 2 should have degraded summarized-with-ref, got: {:?}",
            tool_msgs.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
        let stored = tool_msgs.iter().find(|m| m.content.contains("019717ab")).unwrap();
        assert!(stored.content.contains("was 12474 chars"));
        assert!(
            tool_msgs.iter().any(|m| m.content.contains("Another finding")),
            "summarized-without-ref should be preserved"
        );
    }

    #[test]
    fn extract_stored_ref_marker_works() {
        let content = format!(
            "{}12474 chars | ref:019717ab]\nKey finding.",
            super::super::tool_result_processor::SUMMARIZED_MARKER,
        );
        let marker = ContextGuard::extract_stored_ref_marker(&content).unwrap();
        assert_eq!(marker, "[tool result stored | ref:019717ab, was 12474 chars]");
    }

    #[test]
    fn extract_stored_ref_marker_returns_none_without_ref() {
        let content = format!(
            "{}500 chars]\nSome summary.",
            super::super::tool_result_processor::SUMMARIZED_MARKER,
        );
        assert!(ContextGuard::extract_stored_ref_marker(&content).is_none());
    }

    #[test]
    fn preserves_system_messages() {
        let guard = ContextGuard::new(Some(30));
        let mut msgs = vec![
            sys("important system"),
            sys("another system"),
            user("q"),
            assistant("a"),
            tool(&"big".repeat(200)),
            user("q2"),
            assistant("a2"),
        ];
        guard.trim_if_needed(&mut msgs);
        let system_count = msgs.iter().filter(|m| m.role == "system").count();
        assert_eq!(system_count, 2);
    }

    #[test]
    fn degrades_when_removal_insufficient() {
        let guard = ContextGuard::new(Some(20));
        let mut msgs = vec![
            sys("sys"),
            user(&"long user message ".repeat(20)),
            assistant(&"long assistant reply ".repeat(20)),
            user("q2"),
            assistant("a2"),
        ];
        guard.trim_if_needed(&mut msgs);
        let degraded = msgs.iter().any(|m| m.content == "[content trimmed]");
        assert!(degraded);
    }

    #[test]
    fn estimate_tokens_cjk() {
        let msg = LlmChatMessage {
            role: "user".to_string(),
            content: "你好世界测试".to_string(),
            ..Default::default()
        };
        let tokens = ContextGuard::estimate_message_tokens(&msg);
        // 6 CJK chars × 3 bytes = 18 non-ascii bytes; 18/2 = 9; + 4 overhead = 13
        assert_eq!(tokens, 13);
    }

    #[test]
    fn estimate_tokens_ascii() {
        let msg = LlmChatMessage {
            role: "user".to_string(),
            content: "hello world test".to_string(),
            ..Default::default()
        };
        let tokens = ContextGuard::estimate_message_tokens(&msg);
        // 16 ascii bytes; 16/4 = 4; + 4 overhead = 8
        assert_eq!(tokens, 8);
    }

    #[test]
    fn estimate_tokens_mixed() {
        let msg = LlmChatMessage {
            role: "user".to_string(),
            content: "hello 你好".to_string(),
            ..Default::default()
        };
        let tokens = ContextGuard::estimate_message_tokens(&msg);
        // "hello " = 6 ascii bytes → 6/4 = 1
        // "你好" = 6 non-ascii bytes → 6/2 = 3
        // total = 4 + 4 overhead = 8
        assert_eq!(tokens, 8);
    }

    #[test]
    fn build_summary_input_truncates_long_messages() {
        let long_msg = user(&"x".repeat(2000));
        let msgs: Vec<&LlmChatMessage> = vec![&long_msg];
        let result = build_summary_input(&msgs, None);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert!(result[0].content.contains("Summarize"));
        assert!(result[1].content.len() < 2000);
    }

    #[test]
    fn build_summary_input_respects_char_limit() {
        let big_msgs: Vec<LlmChatMessage> = (0..20)
            .map(|i| user(&format!("message {}: {}", i, "x".repeat(400))))
            .collect();
        let refs: Vec<&LlmChatMessage> = big_msgs.iter().collect();
        let result = build_summary_input(&refs, None);
        assert!(result[1].content.len() <= MAX_SUMMARY_INPUT_CHARS + MAX_CONTENT_PER_MESSAGE + 50);
    }

    #[test]
    fn build_summary_input_includes_previous_summary() {
        let msg = user("new message");
        let msgs: Vec<&LlmChatMessage> = vec![&msg];
        let result = build_summary_input(&msgs, Some("User wanted to find bugs."));
        let user_content = &result[1].content;
        assert!(user_content.contains("[Previous summary]:"));
        assert!(user_content.contains("User wanted to find bugs."));
        assert!(user_content.contains("[New messages]:"));
        assert!(user_content.contains("new message"));
    }

    #[test]
    fn find_previous_summary_extracts_content() {
        let msgs = vec![
            sys("core prompt"),
            sys(&format!("{}\n[Goal] Fix auth bug\n[Findings] Token expired", EARLIER_SUMMARY_PREFIX)),
            user("next question"),
        ];
        let prev = find_previous_summary(&msgs);
        assert!(prev.is_some());
        let text = prev.unwrap();
        assert!(text.contains("[Goal] Fix auth bug"));
        assert!(text.contains("[Findings] Token expired"));
    }

    #[test]
    fn find_previous_summary_returns_none_without_summary() {
        let msgs = vec![sys("core prompt"), user("question")];
        assert!(find_previous_summary(&msgs).is_none());
    }

    #[test]
    fn truncate_for_summary_respects_boundaries() {
        let s = "你好世界"; // 12 bytes total
        let truncated = truncate_for_summary(s, 7);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 10); // 6 bytes (2 chars) + "..."
    }

    #[test]
    fn budget_pressure_under() {
        let guard = ContextGuard::new(Some(4096));
        let msgs = vec![sys("hello"), user("hi"), assistant("hey")];
        let pressure = guard.budget_pressure(&msgs);
        assert!(pressure < 0.1, "expected low pressure, got {}", pressure);
    }

    #[test]
    fn budget_pressure_over() {
        let guard = ContextGuard::new(Some(600));
        let msgs = vec![
            sys("system prompt"),
            user(&"long message ".repeat(50)),
            assistant(&"long reply ".repeat(50)),
        ];
        let pressure = guard.budget_pressure(&msgs);
        assert!(pressure >= 0.7, "expected high pressure, got {}", pressure);
    }

    #[tokio::test]
    async fn trim_with_summary_no_provider_falls_back() {
        let guard = ContextGuard::new(Some(20));
        let mut msgs = vec![
            sys("sys"),
            user(&"long user message ".repeat(20)),
            assistant(&"long assistant reply ".repeat(20)),
            user("q2"),
            assistant("a2"),
        ];
        guard.trim_with_summary(&mut msgs).await;
        let degraded = msgs.iter().any(|m| m.content == "[content trimmed]");
        assert!(degraded);
    }

    #[test]
    fn cached_summary_applies_when_messages_grew() {
        let guard = ContextGuard::new(Some(4096));
        // Simulate a cached summary that covered messages 0..5
        // (tail_boundary was 5 when pre_summarize ran).
        let cached = PrecomputedSummary {
            summary_text: "User asked about X, tool returned Y.".to_string(),
            summarized_up_to: 5,
        };
        let mut msgs = vec![
            sys("system prompt"),     // 0: system, skipped
            user("q1"),              // 1: removed
            assistant("a1"),         // 2: removed
            tool("result1"),         // 3: removed
            user("q2"),              // 4: removed
            // --- summarized_up_to = 5 ---
            assistant("a2"),         // 5: kept (after boundary)
            user("q3"),              // 6: kept (new since snapshot)
            assistant("a3"),         // 7: kept (new since snapshot)
        ];
        let applied = guard.apply_cached_summary(&mut msgs, &cached);
        assert!(applied);
        // system prompt preserved, 4 non-system removed, summary inserted, 3 tail kept
        assert!(msgs.iter().any(|m| m.content.contains("User asked about X")));
        assert!(msgs.iter().any(|m| m.content == "system prompt"));
        assert!(msgs.iter().any(|m| m.content == "a3"));
        assert!(!msgs.iter().any(|m| m.content == "q1"));
    }

    #[test]
    fn cached_summary_rejects_when_messages_shrunk() {
        let guard = ContextGuard::new(Some(4096));
        let cached = PrecomputedSummary {
            summary_text: "summary".to_string(),
            summarized_up_to: 10,
        };
        // Only 5 messages — fewer than summarized_up_to
        let mut msgs = vec![
            sys("sys"),
            user("q1"),
            assistant("a1"),
            user("q2"),
            assistant("a2"),
        ];
        let applied = guard.apply_cached_summary(&mut msgs, &cached);
        assert!(!applied);
        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn cached_summary_preserves_system_messages() {
        let guard = ContextGuard::new(Some(4096));
        let cached = PrecomputedSummary {
            summary_text: "conversation summary".to_string(),
            summarized_up_to: 4,
        };
        let mut msgs = vec![
            sys("core system prompt"),  // 0: system, preserved
            sys("extra system"),        // 1: system, preserved
            user("q1"),                 // 2: removed
            assistant("a1"),            // 3: removed
            // --- summarized_up_to = 4 ---
            user("q2"),                 // 4: kept
            assistant("a2"),            // 5: kept
        ];
        let applied = guard.apply_cached_summary(&mut msgs, &cached);
        assert!(applied);
        let system_count = msgs.iter().filter(|m| m.role == "system").count();
        // 2 original system msgs + 1 summary system msg = 3
        assert_eq!(system_count, 3);
        assert!(msgs.iter().any(|m| m.content == "core system prompt"));
        assert!(msgs.iter().any(|m| m.content == "extra system"));
        assert!(msgs.iter().any(|m| m.content.contains("conversation summary")));
    }
}
