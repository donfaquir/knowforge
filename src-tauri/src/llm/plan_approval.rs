//! 计划审批状态（Phase A→B 之间的门控）。
//!
//! `run_planned_agent` 在 Phase A 生成计划后调用 [`PlanApprovalState::register`] 拿到
//! receiver + [`PlanApprovalPendingGuard`]，emit `llm:plan-approval-request`，然后
//! `tokio::time::timeout` + `tokio::select!` 等待。guard 在外层 cancel 导致 future 被 drop 时
//! 自动 `discard_pending`，防止 sender 泄漏在 map 中。计划审批每次都问，不做会话级缓存。
//!
//! 仅提供"执行 / 拒绝"两态：计划是提示词的派生预览，不可编辑——用户若不认可应拒绝并
//! 修改提示词重发，保持"提示词是唯一意图来源"，避免与目标稳定性等机制产生冲突。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

/// 前端对一次计划审批请求的决策。
pub enum PlanDecision {
    /// 按计划执行。
    Approve,
    /// 拒绝执行，丢弃整个请求。
    Reject,
}

pub struct PlanApprovalState {
    pending: Mutex<HashMap<String, oneshot::Sender<PlanDecision>>>,
}

/// RAII 守卫：drop 时清理 pending 条目，避免外层 cancel 把 sender 留在 map 里。
/// `discard_pending` 对不存在的 id 是 no-op，所以 happy path（resolve 已 remove）也安全。
pub struct PlanApprovalPendingGuard {
    state: Arc<PlanApprovalState>,
    approval_id: String,
}

impl Drop for PlanApprovalPendingGuard {
    fn drop(&mut self) {
        self.state.discard_pending(&self.approval_id);
    }
}

impl Default for PlanApprovalState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanApprovalState {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// 登记一个 pending 审批，返回 (approval_id, receiver, guard)。
    /// approval_id 用 UUID v4 生成，无序无关联以防泄漏其他元信息。
    /// guard 需由调用方持有直到 await 结束。
    pub fn register(
        self: &Arc<Self>,
    ) -> (String, oneshot::Receiver<PlanDecision>, PlanApprovalPendingGuard) {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut g = match self.pending.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.insert(approval_id.clone(), tx);
        }
        let guard = PlanApprovalPendingGuard {
            state: self.clone(),
            approval_id: approval_id.clone(),
        };
        (approval_id, rx, guard)
    }

    /// 前端响应：取出 sender 并 send。若 id 不存在或 receiver 已 drop，返回 Err。
    pub fn resolve(&self, approval_id: &str, decision: PlanDecision) -> Result<(), String> {
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

    /// 超时或取消时清理 pending 项（receiver 已 drop，sender 留在 map 里会泄漏）。
    pub fn discard_pending(&self, approval_id: &str) {
        let mut g = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.remove(approval_id);
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
    async fn register_then_resolve_delivers_approve() {
        let state = Arc::new(PlanApprovalState::new());
        let (id, rx, _guard) = state.register();
        state.resolve(&id, PlanDecision::Approve).unwrap();
        assert!(matches!(rx.await.unwrap(), PlanDecision::Approve));
    }

    #[tokio::test]
    async fn resolve_delivers_reject() {
        let state = Arc::new(PlanApprovalState::new());
        let (id, rx, _guard) = state.register();
        state.resolve(&id, PlanDecision::Reject).unwrap();
        assert!(matches!(rx.await.unwrap(), PlanDecision::Reject));
    }

    #[tokio::test]
    async fn resolve_unknown_id_errors() {
        let state = PlanApprovalState::new();
        let err = state.resolve("nope", PlanDecision::Approve).unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[tokio::test]
    async fn guard_drop_cleans_pending_and_resolve_fails() {
        let state = Arc::new(PlanApprovalState::new());
        assert_eq!(state.pending_len(), 0);
        let id = {
            let (id, _rx, _guard) = state.register();
            assert_eq!(state.pending_len(), 1);
            id
        };
        assert_eq!(state.pending_len(), 0);
        let err = state.resolve(&id, PlanDecision::Approve).unwrap_err();
        assert!(err.contains("unknown"));
    }
}
