use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::types::ApprovalPolicy;

// ─── ToolContext ───────────────────────────────────────────────────────────────

pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub conversation_id: String,
    /// uuid v7
    pub call_id: String,
    pub user_approval_callback: Arc<dyn ApprovalCallback>,
    pub audit_sink: Arc<dyn AuditSink>,
    pub privacy_filter: Arc<dyn PrivacyFilter>,
    /// Tauri app cache directory（用于语义搜索模型缓存路径）
    pub app_cache_dir: Option<PathBuf>,
    /// Tauri app bundle resource directory（用于语义搜索模型 bundle 路径）
    pub app_bundle_resource_dir: Option<PathBuf>,
    /// Iter 5 #4: how deeply nested this tool call is.
    /// 0 = called from main agent loop. 1 = called from inside a skill sub-turn
    /// (or another tool that recursed into agent_loop). Stage 1 caps this at 1.
    pub nesting_depth: u8,
}

// ─── AuditSink trait ───────────────────────────────────────────────────────────

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record(&self, entry: AuditEntry);
}

// ─── PrivacyFilter trait ───────────────────────────────────────────────────────

pub trait PrivacyFilter: Send + Sync {
    fn filter_note_content(&self, content: &str) -> (String, u32);
    fn is_private_path(&self, rel_path: &str, workspace_root: &Path) -> bool;
}

// ─── ApprovalCallback trait ────────────────────────────────────────────────────

#[async_trait]
pub trait ApprovalCallback: Send + Sync {
    async fn request_approval(
        &self,
        tool_name: &str,
        policy: &ApprovalPolicy,
        input_summary: &str,
    ) -> bool;
}

// ─── AutoApprovalCallback ──────────────────────────────────────────────────────
// v1.0 默认实现：Auto/ConfirmOncePerSession/ConfirmEach 均通过；Forbidden 拒绝

pub struct AutoApprovalCallback;

#[async_trait]
impl ApprovalCallback for AutoApprovalCallback {
    async fn request_approval(
        &self,
        _tool_name: &str,
        policy: &ApprovalPolicy,
        _input_summary: &str,
    ) -> bool {
        *policy != ApprovalPolicy::Forbidden
    }
}

// ─── AuditEntry ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub conversation_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub version: String,
    pub input_redacted: Value,
    pub result_summary: Value,
    pub duration_ms: u64,
    pub approval: String,
    pub error_code: Option<String>,
}

// ─── ToolContextFactory ────────────────────────────────────────────────────────

pub struct ToolContextFactory {
    pub audit_sink: Arc<dyn AuditSink>,
    pub privacy_filter: Arc<dyn PrivacyFilter>,
    pub approval_callback: Arc<dyn ApprovalCallback>,
}

impl ToolContextFactory {
    pub fn new(audit_sink: Arc<dyn AuditSink>, privacy_filter: Arc<dyn PrivacyFilter>) -> Self {
        Self {
            audit_sink,
            privacy_filter,
            approval_callback: Arc::new(AutoApprovalCallback),
        }
    }

    pub fn create_context(
        &self,
        workspace_root: PathBuf,
        conversation_id: &str,
        app_cache_dir: Option<PathBuf>,
        app_bundle_resource_dir: Option<PathBuf>,
    ) -> ToolContext {
        self.create_context_at_depth(
            workspace_root,
            conversation_id,
            app_cache_dir,
            app_bundle_resource_dir,
            0,
        )
    }

    /// Iter 5 #4: build a ToolContext that records the nesting depth.
    /// Used by SkillAsTool to mark sub-turn calls (depth >= 1) so we can
    /// short-circuit deeper nesting attempts.
    pub fn create_context_at_depth(
        &self,
        workspace_root: PathBuf,
        conversation_id: &str,
        app_cache_dir: Option<PathBuf>,
        app_bundle_resource_dir: Option<PathBuf>,
        nesting_depth: u8,
    ) -> ToolContext {
        ToolContext {
            workspace_root,
            conversation_id: conversation_id.to_string(),
            call_id: uuid::Uuid::now_v7().to_string(),
            user_approval_callback: self.approval_callback.clone(),
            audit_sink: self.audit_sink.clone(),
            privacy_filter: self.privacy_filter.clone(),
            app_cache_dir,
            app_bundle_resource_dir,
            nesting_depth,
        }
    }
}
