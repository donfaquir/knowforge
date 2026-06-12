use super::LlmChatMessage;

const DEFAULT_MAX_TOKENS: usize = 8192;
const RESERVE_TOKENS: usize = 512;
const MIN_KEEP_ROUNDS: usize = 2;

pub struct ContextGuard {
    max_tokens: usize,
    reserve: usize,
}

impl ContextGuard {
    pub fn new(max_context_tokens: Option<u64>) -> Self {
        let max_tokens = max_context_tokens
            .map(|t| t as usize)
            .unwrap_or(DEFAULT_MAX_TOKENS);
        Self {
            max_tokens,
            reserve: RESERVE_TOKENS,
        }
    }

    fn estimate_message_tokens(msg: &LlmChatMessage) -> usize {
        msg.content.len() / 3 + 4
    }

    fn estimate_total(messages: &[LlmChatMessage]) -> usize {
        messages.iter().map(Self::estimate_message_tokens).sum()
    }

    pub fn trim_if_needed(&self, messages: &mut Vec<LlmChatMessage>) {
        let budget = self.max_tokens.saturating_sub(self.reserve);
        if Self::estimate_total(messages) <= budget {
            return;
        }

        // Phase 1: remove oldest tool-result messages, preserving system messages
        // and the most recent MIN_KEEP_ROUNDS pairs (assistant + tool).
        let tail_boundary = self.find_tail_boundary(messages);
        let mut i = 0;
        while i < tail_boundary && Self::estimate_total(messages) > budget {
            if messages[i].role == "system" {
                i += 1;
                continue;
            }
            if messages[i].role == "tool" {
                messages.remove(i);
                // tail_boundary shifts but we recalculate below
                continue;
            }
            i += 1;
        }

        if Self::estimate_total(messages) <= budget {
            return;
        }

        // Phase 2: degrade — replace content of oldest non-system messages with placeholder
        let tail_boundary = self.find_tail_boundary(messages);
        for i in 0..tail_boundary {
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

    fn find_tail_boundary(&self, messages: &[LlmChatMessage]) -> usize {
        let len = messages.len();
        if len <= MIN_KEEP_ROUNDS * 2 {
            return 0;
        }

        // Count backwards to find the start of the last MIN_KEEP_ROUNDS
        // assistant+tool round-trips.
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
        // Tiny budget forces trimming
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
        // System messages should be preserved
        assert!(msgs.iter().any(|m| m.role == "system"));
        // Recent rounds should be preserved
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
        // Very tiny budget — even after removing tool messages, still over
        let guard = ContextGuard::new(Some(20));
        let mut msgs = vec![
            sys("sys"),
            user(&"long user message ".repeat(20)),
            assistant(&"long assistant reply ".repeat(20)),
            user("q2"),
            assistant("a2"),
        ];
        guard.trim_if_needed(&mut msgs);
        // The old user message should be degraded
        let degraded = msgs.iter().any(|m| m.content == "[content trimmed]");
        assert!(degraded);
    }

    #[test]
    fn estimate_tokens_cjk() {
        // CJK characters are 3 bytes each in UTF-8, so len/3 ≈ char count, which
        // is a reasonable token estimate for Chinese text.
        let msg = LlmChatMessage {
            role: "user".to_string(),
            content: "你好世界测试".to_string(),
            ..Default::default()
        };
        let tokens = ContextGuard::estimate_message_tokens(&msg);
        // 6 CJK chars × 3 bytes = 18 bytes; 18/3 + 4 = 10
        assert_eq!(tokens, 10);
    }
}
