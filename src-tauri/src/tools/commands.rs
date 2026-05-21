use serde_json::Value;
use std::sync::Arc;
use tauri::State;

use crate::tools::context::ToolContextFactory;
use crate::tools::registry::{ToolRegistry, ToolScope};
use crate::WorkspaceState;

#[tauri::command]
pub async fn list_tools(
    scope: Option<String>,
    registry: State<'_, Arc<ToolRegistry>>,
) -> Result<Vec<Value>, String> {
    let scope = match scope.as_deref() {
        Some(s) if s.starts_with("conv:") => ToolScope::Conversation(s[5..].to_string()),
        _ => ToolScope::Global,
    };
    Ok(registry.list_for_llm(scope))
}

#[tauri::command]
pub async fn invoke_tool(
    name: String,
    input: Value,
    conversation_id: Option<String>,
    registry: State<'_, Arc<ToolRegistry>>,
    ctx_factory: State<'_, Arc<ToolContextFactory>>,
    ws_state: State<'_, WorkspaceState>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let workspace_root = crate::lock_workspace_root(&ws_state)?;

    let tool = registry
        .get(&name)
        .ok_or_else(|| format!("tool not found: {name}"))?;

    tool.validate_input(&input).map_err(|e| e.message)?;

    let cache_dir = crate::semantic_index::default_model_cache_dir();
    let bundle_dir = crate::semantic_index::resolve_bundle_model_dir(&app);

    let ctx = ctx_factory.create_context(
        workspace_root,
        conversation_id.as_deref().unwrap_or(""),
        Some(cache_dir),
        Some(bundle_dir),
    );

    let manifest = tool.manifest().clone();
    let start = std::time::Instant::now();

    let result = tool.invoke(&ctx, input.clone()).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    // 构造 AuditEntry 并记录
    let (result_summary, error_code) = match &result {
        crate::tools::types::ToolResult::Ok {
            redacted_count, ..
        } => (
            serde_json::json!({ "status": "ok", "redacted_count": redacted_count }),
            None,
        ),
        crate::tools::types::ToolResult::PartialOk { .. } => (
            serde_json::json!({ "status": "partial_ok" }),
            None,
        ),
        crate::tools::types::ToolResult::Err { error } => (
            serde_json::json!({ "status": "error" }),
            Some(format!("{:?}", error.code)),
        ),
    };

    let entry = crate::tools::context::AuditEntry {
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        conversation_id: ctx.conversation_id.clone(),
        call_id: ctx.call_id.clone(),
        tool_name: manifest.name.clone(),
        version: manifest.version.clone(),
        input_redacted: crate::tools::audit::redact_value(&input),
        result_summary,
        duration_ms,
        approval: format!("{:?}", manifest.default_approval),
        error_code,
    };
    ctx.audit_sink.record(entry).await;

    serde_json::to_value(&result).map_err(|e| e.to_string())
}
