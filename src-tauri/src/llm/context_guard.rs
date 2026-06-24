use std::sync::Arc;

use super::provider::{CompletionOverrides, LlmProvider};
use super::LlmChatMessage;

const DEFAULT_MAX_TOKENS: usize = 32768;
const RESERVE_TOKENS: usize = 512;
const MIN_KEEP_ROUNDS: usize = 2;

const MIN_MESSAGES_FOR_SUMMARY: usize = 4;
const MAX_SUMMARY_INPUT_CHARS: usize = 6000;
const MAX_CONTENT_PER_MESSAGE: usize = 500;

const SUMMARY_SYSTEM: &str = "\
Summarize the following conversation excerpt in 2-3 concise sentences. \
Focus on: what the user asked, what tools were called, and key findings. \
Output only the summary, nothing else.";

#[derive(Clone)]
pub struct ContextGuard {
    max_tokens: usize,
    reserve: usize,
    provider: Option<Arc<dyn LlmProvider>>,
}

pub struct PrecomputedSummary {
    pub summary_text: String,
    pub original_msg_count: usize,
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

        let summary_input = build_summary_input(&removable_msgs);
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
            original_msg_count: messages.len(),
        })
    }

    pub fn apply_cached_summary(
        &self,
        messages: &mut Vec<LlmChatMessage>,
        cached: &PrecomputedSummary,
    ) -> bool {
        if messages.len() != cached.original_msg_count {
            return false;
        }

        let tail_boundary = Self::find_tail_boundary(messages);
        let removable_indices: Vec<usize> = (0..tail_boundary.min(messages.len()))
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
        let mut i = 0;
        while i < tail_boundary.min(messages.len()) && Self::estimate_total(messages) > budget {
            if messages[i].role == "system" {
                i += 1;
                continue;
            }
            if messages[i].role == "tool" {
                messages.remove(i);
                continue;
            }
            i += 1;
        }
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

        let summary_input = build_summary_input(&removable_msgs);

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

fn build_summary_input(messages: &[&LlmChatMessage]) -> Vec<LlmChatMessage> {
    let mut content = String::new();
    let mut char_count = 0;

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
    fn removes_old_tool_results_first() {
        let guard = ContextGuard::new(Some(50));
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
        guard.trim_if_needed(&mut msgs);
        assert!(msgs.iter().any(|m| m.role == "system"));
        assert!(msgs.iter().any(|m| m.content == "a3"));
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
        let long_msg = user(&"x".repeat(1000));
        let msgs: Vec<&LlmChatMessage> = vec![&long_msg];
        let result = build_summary_input(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert!(result[0].content.contains("Summarize"));
        assert!(result[1].content.len() < 1000);
    }

    #[test]
    fn build_summary_input_respects_char_limit() {
        let big_msgs: Vec<LlmChatMessage> = (0..20)
            .map(|i| user(&format!("message {}: {}", i, "x".repeat(400))))
            .collect();
        let refs: Vec<&LlmChatMessage> = big_msgs.iter().collect();
        let result = build_summary_input(&refs);
        assert!(result[1].content.len() <= MAX_SUMMARY_INPUT_CHARS + MAX_CONTENT_PER_MESSAGE + 50);
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
}
