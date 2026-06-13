use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::context::ToolContext;

// ─── ToolCategory ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    NoteRead,
    NoteWrite,
    Web,
    Graph,
    Utility,
    Skill,
}

// ─── Effect ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Read,
    Write,
    Network,
    Llm,
    Shell,
    System,
}

// ─── Risk ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Safe,
    Caution,
    Dangerous,
}

// ─── ApprovalPolicy ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Auto,
    ConfirmOncePerSession,
    ConfirmEach,
    Forbidden,
}

// ─── DeprecationInfo ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationInfo {
    pub since: String,
    pub replacement: Option<String>,
}

// ─── ToolExample ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExample {
    pub description: String,
    pub input: Value,
    pub output: Value,
}

// ─── ToolManifest ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolManifest {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effects: Vec<Effect>,
    pub risk: Risk,
    pub privacy_aware: bool,
    pub requires_workspace: bool,
    pub default_approval: ApprovalPolicy,
    pub examples: Vec<ToolExample>,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<DeprecationInfo>,
}

// ─── ToolMetrics ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolMetrics {
    pub duration_ms: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub llm_tokens_in: u32,
    pub llm_tokens_out: u32,
    pub network_bytes: u64,
}

// ─── ToolWarning ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolWarning {
    pub code: String,
    pub message: String,
}

// ─── ToolErrorCode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolErrorCode {
    InvalidInput,
    NotFound,
    PermissionDenied,
    WorkspaceNotOpen,
    PrivacyBlocked,
    NetworkDenied,
    Timeout,
    RateLimited,
    BudgetExceeded,
    Internal,
}

// ─── ToolError ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

// ─── ToolResult ────────────────────────────────────────────────────────────────
// Internally tagged with "status"; Err variant renamed to "error" and flattened
// so the JSON shape is: { "status": "error", "code": "...", "message": "...", ... }

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolResult {
    Ok {
        data: Value,
        redacted_count: u32,
        warnings: Vec<ToolWarning>,
        metrics: ToolMetrics,
    },
    PartialOk {
        data: Value,
        redacted_count: u32,
        warnings: Vec<ToolWarning>,
        metrics: ToolMetrics,
        errors: Vec<ToolError>,
    },
    #[serde(rename = "error")]
    Err {
        error: ToolError,
    },
}

// ─── Tool trait ────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn manifest(&self) -> &ToolManifest;

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult;

    fn validate_input(&self, input: &Value) -> Result<(), ToolError> {
        crate::tools::validation::validate(&self.manifest().input_schema, input)
    }

    /// Tool 根据具体输入动态判断是否需要用户审批。
    /// 返回 true 表示此次调用需要审批确认（即使 manifest 声明为 Auto）。
    /// 默认实现返回 false（遵循 manifest 策略）。
    fn requires_approval(&self, _input: &Value) -> bool {
        false
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }
}
