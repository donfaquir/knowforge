pub mod types;
pub mod registry;
pub mod context;
pub mod audit;
pub mod privacy;
pub mod validation;
pub mod commands;
pub mod path_safety;
pub mod built_in;

pub use types::*;
pub use registry::ToolRegistry;
pub use context::{ToolContext, ToolContextFactory};

// ─── time.now 内置工具 ─────────────────────────────────────────────────────────
// P0 唯一内置工具，用于端到端验证

use std::sync::Arc;

struct TimeNowTool {
    manifest: ToolManifest,
}

impl TimeNowTool {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            manifest: ToolManifest {
                name: "time.now".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "返回当前 UTC 时间戳（ISO 8601 格式）".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "iso8601": { "type": "string" },
                        "unix_ms": { "type": "integer" }
                    }
                }),
                effects: vec![],
                risk: Risk::Safe,
                privacy_aware: false,
                requires_workspace: false,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["time".to_string(), "utility".to_string()],
                deprecated: None,
            },
        })
    }
}

#[async_trait::async_trait]
impl Tool for TimeNowTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    async fn invoke(&self, _ctx: &ToolContext, _input: serde_json::Value) -> ToolResult {
        let now = chrono::Utc::now();
        ToolResult::Ok {
            data: serde_json::json!({
                "iso8601": now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "unix_ms": now.timestamp_millis(),
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms: 0,
                ..Default::default()
            },
        }
    }
}

pub fn register_builtin_tools(registry: &ToolRegistry) -> Result<(), registry::RegistryError> {
    // 注意：所有声明 Effect::Read 或 Effect::Write 的工具，必须同时设置 privacy_aware = true，
    // 否则此函数会在应用启动阶段 panic。新增工具后必须运行 `cargo test tools::mod_tests` 验证。
    registry.register(TimeNowTool::new())?;

    // P1 内置工具
    registry.register(Arc::new(built_in::vault_search::VaultSearchKeywordTool::new()))?;
    registry.register(Arc::new(built_in::vault_search::VaultSemanticSearchTool::new()))?;
    registry.register(Arc::new(built_in::note_ops::NoteListTool::new()))?;
    registry.register(Arc::new(built_in::note_ops::NoteReadTool::new()))?;
    registry.register(Arc::new(built_in::thought_ops::ThoughtListTool::new()))?;
    registry.register(Arc::new(built_in::link_ops::LinkSuggestRelatedTool::new()))?;
    registry.register(Arc::new(built_in::graph_ops::GraphQueryTopicNetworkTool::new()))?;
    registry.register(Arc::new(built_in::graph_ops::IndexStatusTool::new()))?;

    // P3 写操作工具
    registry.register(Arc::new(built_in::note_ops::NoteWriteSectionTool::new()))?;
    registry.register(Arc::new(built_in::note_ops::NoteAppendTool::new()))?;
    registry.register(Arc::new(built_in::note_ops::NoteCreateTool::new()))?;
    registry.register(Arc::new(built_in::thought_ops::ThoughtCreateTool::new()))?;

    // P4 network tools
    registry.register(Arc::new(built_in::web_ops::WebReadPageTool::new()))?;
    registry.register(Arc::new(built_in::web_search::WebSearchTool::new()))?;

    Ok(())
}

#[cfg(test)]
mod mod_tests {
    use super::*;
    use crate::tools::registry::ToolRegistry;

    /// 回归测试：确保 register_builtin_tools() 全量注册成功，
    /// 防止 privacy_aware 与 Effect::Read/Write 矛盾导致启动 panic。
    #[test]
    fn test_register_builtin_tools_succeeds() {
        let registry = ToolRegistry::new();
        let result = register_builtin_tools(&registry);
        assert!(
            result.is_ok(),
            "register_builtin_tools failed: {:?}",
            result.err()
        );
        // 确认工具总数：1(time.now) + 8(P1) + 4(P3 写操作) + 2(P4 网络) = 15
        let tools = registry.list_for_llm(crate::tools::registry::ToolScope::Global);
        assert_eq!(tools.len(), 15, "expected 15 registered tools, got {}", tools.len());
    }
}
