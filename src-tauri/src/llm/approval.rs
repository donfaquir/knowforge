//! Tool Call 审批状态。
//!
//! - **pending**:`approval_id -> oneshot::Sender<bool>`,等待前端 `respond_tool_approval`。
//! - **approved**:`conversation_id -> {tool_name}`,`ConfirmOncePerSession` 缓存。
//!
//! agent_loop 在执行非 Auto 工具前调用 [`ToolApprovalState::register`] 拿到 receiver +
//! [`ApprovalPendingGuard`],emit `llm:tool-approval-request`,然后 `tokio::time::timeout` +
//! `tokio::select!` 等待。guard 在外层 cancel 导致整个 future 被 drop 时,
//! 自动调用 [`ToolApprovalState::discard_pending`] 防止 sender 在 map 中泄漏。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

pub struct ToolApprovalState {
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    approved: Mutex<HashMap<String, HashSet<String>>>,
}

/// RAII 守卫:在 drop 时自动清理 pending 条目,避免外层 cancel 把 sender 留在 map 里。
/// `discard_pending` 对不存在的 id 是 no-op,所以 happy path(resolve 已 remove)也安全。
pub struct ApprovalPendingGuard {
    state: Arc<ToolApprovalState>,
    approval_id: String,
}

impl ApprovalPendingGuard {
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }
}

impl Drop for ApprovalPendingGuard {
    fn drop(&mut self) {
        self.state.discard_pending(&self.approval_id);
    }
}

impl Default for ToolApprovalState {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolApprovalState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            approved: Mutex::new(HashMap::new()),
        }
    }

    /// 检查会话级 ConfirmOncePerSession 缓存。
    pub fn is_pre_approved(&self, conv_id: &str, tool: &str) -> bool {
        let g = match self.approved.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.get(conv_id).map(|s| s.contains(tool)).unwrap_or(false)
    }

    /// 记入会话级 ConfirmOncePerSession 缓存。
    pub fn remember_approval(&self, conv_id: &str, tool: &str) {
        let mut g = match self.approved.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.entry(conv_id.to_string())
            .or_default()
            .insert(tool.to_string());
    }

    /// 登记一个 pending 审批,返回 (approval_id, receiver, guard)。
    /// approval_id 用 UUID v4 生成,无序无关联以防泄漏其他元信息。
    /// guard 在 drop 时清理 pending,需要由调用方持有直到 await 结束。
    pub fn register(self: &Arc<Self>) -> (String, oneshot::Receiver<bool>, ApprovalPendingGuard) {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut g = match self.pending.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.insert(approval_id.clone(), tx);
        }
        let guard = ApprovalPendingGuard {
            state: self.clone(),
            approval_id: approval_id.clone(),
        };
        (approval_id, rx, guard)
    }

    /// 前端响应:取出 sender 并 send。若 id 不存在或 receiver 已 drop,返回 Err。
    pub fn resolve(&self, approval_id: &str, decision: bool) -> Result<(), String> {
        let tx = {
            let mut g = match self.pending.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.remove(approval_id)
                .ok_or_else(|| format!("unknown approval_id: {approval_id}"))?
        };
        tx.send(decision)
            .map_err(|_| "approval receiver dropped (timed out or cancelled)".to_string())
    }

    /// 超时或取消时清理 pending 项(receiver 已 drop,sender 留在 map 里会泄漏)。
    pub fn discard_pending(&self, approval_id: &str) {
        let mut g = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.remove(approval_id);
    }

    /// 切换/删除会话时清理该会话的 ConfirmOncePerSession 缓存。
    pub fn clear_conversation(&self, conv_id: &str) {
        let mut g = match self.approved.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.remove(conv_id);
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        let g = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_then_resolve_delivers_decision() {
        let state = Arc::new(ToolApprovalState::new());
        let (id, rx, _guard) = state.register();
        state.resolve(&id, true).unwrap();
        assert_eq!(rx.await.unwrap(), true);
    }

    #[tokio::test]
    async fn resolve_unknown_id_errors() {
        let state = ToolApprovalState::new();
        let err = state.resolve("nope", true).unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[tokio::test]
    async fn discard_pending_releases_id() {
        let state = Arc::new(ToolApprovalState::new());
        let (id, _rx, guard) = state.register();
        drop(guard);
        let err = state.resolve(&id, true).unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[tokio::test]
    async fn guard_drop_cleans_pending_entry() {
        let state = Arc::new(ToolApprovalState::new());
        assert_eq!(state.pending_len(), 0);
        {
            let (_id, _rx, _guard) = state.register();
            assert_eq!(state.pending_len(), 1);
        }
        assert_eq!(state.pending_len(), 0);
    }

    #[test]
    fn approval_cache_per_conversation_and_tool() {
        let s = ToolApprovalState::new();
        assert!(!s.is_pre_approved("c1", "note.create"));
        s.remember_approval("c1", "note.create");
        assert!(s.is_pre_approved("c1", "note.create"));
        // 不同 conv
        assert!(!s.is_pre_approved("c2", "note.create"));
        // 不同 tool
        assert!(!s.is_pre_approved("c1", "note.write_section"));
    }

    #[test]
    fn clear_conversation_drops_cache() {
        let s = ToolApprovalState::new();
        s.remember_approval("c1", "note.create");
        s.remember_approval("c1", "thought.create");
        s.remember_approval("c2", "note.create");
        s.clear_conversation("c1");
        assert!(!s.is_pre_approved("c1", "note.create"));
        assert!(!s.is_pre_approved("c1", "thought.create"));
        assert!(s.is_pre_approved("c2", "note.create"));
    }
}
