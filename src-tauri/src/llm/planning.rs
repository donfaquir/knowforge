use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::agent_loop::{self, AgentLoopConfig, SharedMemoryManager};
use super::approval::ToolApprovalState;
use super::provider::LlmProvider;
use super::LlmChatMessage;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;
use tauri::AppHandle;

const PLANNING_SYSTEM_PROMPT: &str = "\
Before making any tool calls, briefly explain your approach in 1-2 sentences.\n\
Then proceed to execute step by step.";

#[allow(clippy::too_many_arguments)]
pub async fn run_planned_agent(
    app: AppHandle,
    session_id: String,
    mut initial_messages: Vec<LlmChatMessage>,
    tools_json: Vec<Value>,
    registry: Arc<ToolRegistry>,
    ctx_factory: Arc<ToolContextFactory>,
    workspace_root: PathBuf,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    provider: Arc<dyn LlmProvider>,
    cancel: CancellationToken,
    config: AgentLoopConfig,
    conversation_id: String,
    approval_state: Arc<ToolApprovalState>,
    memory_manager: SharedMemoryManager,
) -> String {
    initial_messages.push(LlmChatMessage {
        role: "system".to_string(),
        content: PLANNING_SYSTEM_PROMPT.to_string(),
        ..Default::default()
    });

    agent_loop::run_agent_stream(
        app,
        session_id,
        initial_messages,
        tools_json,
        registry,
        ctx_factory,
        workspace_root,
        app_cache_dir,
        app_bundle_resource_dir,
        provider,
        cancel,
        config,
        conversation_id,
        approval_state,
        memory_manager,
    )
    .await
}
