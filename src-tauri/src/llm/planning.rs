use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::agent_loop::{self, AgentLoopConfig};
use super::approval::ToolApprovalState;
use super::provider::LlmProvider;
use super::LlmChatMessage;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;

const PLANNING_SYSTEM_PROMPT: &str = "\
Analyze the user's request carefully. Output a numbered plan listing the exact tool calls \
you would make and why. Do NOT call any tools — only output the plan as plain text.\n\n\
Available tools:\n";

const MIN_PLAN_LENGTH: usize = 10;

fn build_planning_messages(
    messages: &[LlmChatMessage],
    tool_descriptions: &str,
) -> Vec<LlmChatMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    for m in messages {
        out.push(m.clone());
    }
    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: format!("{PLANNING_SYSTEM_PROMPT}{tool_descriptions}"),
        ..Default::default()
    });
    out
}

fn inject_plan_into_messages(
    messages: &[LlmChatMessage],
    plan_text: &str,
) -> Vec<LlmChatMessage> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    for m in messages {
        out.push(m.clone());
    }
    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: format!(
            "Execute the following plan step by step. Call the tools listed. \
             After all steps, provide the final answer to the user.\n\n\
             Plan:\n{plan_text}"
        ),
        ..Default::default()
    });
    out
}

fn build_tool_descriptions(tools_json: &[Value]) -> String {
    let mut desc = String::new();
    for t in tools_json {
        let name = t
            .pointer("/function/name")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let description = t
            .pointer("/function/description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        desc.push_str(&format!("- {name}: {description}\n"));
    }
    desc
}

fn emit_planning_start(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        "llm:planning-start",
        json!({ "sessionId": session_id }),
    );
}

fn emit_planning_done(app: &AppHandle, session_id: &str, plan_text: &str) {
    let _ = app.emit(
        "llm:planning-done",
        json!({ "sessionId": session_id, "planText": plan_text }),
    );
}

#[allow(clippy::too_many_arguments)]
pub async fn run_planned_agent(
    app: AppHandle,
    session_id: String,
    initial_messages: Vec<LlmChatMessage>,
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
) -> String {
    // Phase A: Planning (no tools, text-only output)
    let tool_desc = build_tool_descriptions(&tools_json);
    let plan_messages = build_planning_messages(&initial_messages, &tool_desc);

    emit_planning_start(&app, &session_id);

    let plan_result = provider
        .chat_stream(
            &app,
            &session_id,
            plan_messages,
            None,
            cancel.clone(),
        )
        .await;

    let plan_text = match plan_result {
        Ok(r) => r.content,
        Err(_) => String::new(),
    };

    emit_planning_done(&app, &session_id, &plan_text);

    if cancel.is_cancelled() {
        return String::new();
    }

    // Phase B: Execution (inject plan, normal agent loop)
    let exec_messages = if plan_text.trim().len() >= MIN_PLAN_LENGTH {
        inject_plan_into_messages(&initial_messages, &plan_text)
    } else {
        initial_messages
    };

    agent_loop::run_agent_stream(
        app,
        session_id,
        exec_messages,
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
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_planning_messages_appends_system() {
        let msgs = vec![LlmChatMessage {
            role: "user".to_string(),
            content: "search my notes".to_string(),
            ..Default::default()
        }];
        let result = build_planning_messages(&msgs, "- note.list: list notes\n");
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].role, "system");
        assert!(result[1].content.contains("note.list"));
        assert!(result[1].content.contains("Do NOT call any tools"));
    }

    #[test]
    fn inject_plan_adds_execution_guidance() {
        let msgs = vec![LlmChatMessage {
            role: "user".to_string(),
            content: "find my notes".to_string(),
            ..Default::default()
        }];
        let plan = "1. Call note.list\n2. Read top result";
        let result = inject_plan_into_messages(&msgs, plan);
        assert_eq!(result.len(), 2);
        assert!(result[1].content.contains("Execute the following plan"));
        assert!(result[1].content.contains("note.list"));
    }

    #[test]
    fn build_tool_descriptions_extracts_names() {
        let tools = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "note.list",
                    "description": "List notes in the vault",
                    "parameters": {}
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "web.search",
                    "description": "Search the web",
                    "parameters": {}
                }
            }),
        ];
        let desc = build_tool_descriptions(&tools);
        assert!(desc.contains("- note.list: List notes in the vault"));
        assert!(desc.contains("- web.search: Search the web"));
    }
}
