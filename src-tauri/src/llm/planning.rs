use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::agent_loop::{self, AgentLoopConfig, SharedMemoryManager};
use super::approval::ToolApprovalState;
use super::plan_approval::{PlanApprovalState, PlanDecision};
use super::provider::LlmProvider;
use super::LlmChatMessage;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;

/// How long to wait for the user to act on a plan-approval request before
/// degrading to auto-execution. Longer than the per-tool approval timeout
/// (120s) since reviewing/editing a full plan takes more time.
const PLAN_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

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
            "Execute the following plan step by step. \
             Before starting each step, call plan.update_step with {{\"step\": N, \"status\": \"in_progress\"}}. \
             After completing each step, call plan.update_step with {{\"step\": N, \"status\": \"done\"}}. \
             After all steps, provide the final answer to the user.\n\n\
             Plan:\n{plan_text}"
        ),
        ..Default::default()
    });
    out
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PlanStep {
    pub step: u32,
    pub title: String,
}

fn parse_plan_steps(plan_text: &str) -> Vec<PlanStep> {
    plan_text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let dot_pos = trimmed.find(". ")?;
            let num_part = &trimmed[..dot_pos];
            if !num_part.chars().all(|c| c.is_ascii_digit()) || num_part.is_empty() {
                return None;
            }
            let step = num_part.parse::<u32>().ok()?;
            let title = trimmed[dot_pos + 2..].trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(PlanStep { step, title })
        })
        .collect()
}

fn emit_plan_steps(app: &AppHandle, session_id: &str, steps: &[PlanStep]) {
    let _ = app.emit(
        "llm:plan-steps",
        json!({ "sessionId": session_id, "steps": steps }),
    );
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

pub(crate) fn emit_planning_start(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        "llm:planning-start",
        json!({ "sessionId": session_id }),
    );
}

pub(crate) fn emit_planning_done(app: &AppHandle, session_id: &str, plan_text: &str) {
    let _ = app.emit(
        "llm:planning-done",
        json!({ "sessionId": session_id, "planText": plan_text }),
    );
}

/// Emit `llm:agent-done` when we short-circuit before the agent loop runs
/// (e.g. the user rejected the plan), so the frontend clears its streaming state.
fn emit_agent_done(app: &AppHandle, session_id: &str) {
    let _ = app.emit("llm:agent-done", json!({ "sessionId": session_id }));
}

/// Emit when the approval gate concludes by ANY means (approve / reject / timeout /
/// cancel), so the frontend dismisses the approval card. Interactive resolutions
/// (button clicks) also clear it optimistically on the frontend; this covers the
/// non-interactive paths (timeout auto-execute, cancel) where no click happens.
fn emit_plan_approval_resolved(app: &AppHandle, session_id: &str) {
    let _ = app.emit("llm:plan-approval-resolved", json!({ "sessionId": session_id }));
}

/// Outcome of the plan-approval gate between Phase A and Phase B.
enum PlanGateOutcome {
    /// Proceed to execution with the original plan.
    Proceed,
    /// User rejected the plan; abandon the request.
    Reject,
    /// Cancelled while waiting.
    Cancelled,
}

/// Emit `llm:plan-approval-request` and wait for the user's decision.
/// Timeout or a closed channel degrades to executing the plan —
/// rejecting on timeout would silently drop the whole user request.
async fn wait_for_plan_approval(
    app: &AppHandle,
    session_id: &str,
    conversation_id: &str,
    state: &Arc<PlanApprovalState>,
    plan_text: &str,
    cancel: &CancellationToken,
) -> PlanGateOutcome {
    let (approval_id, rx, _guard) = state.register();
    let _ = app.emit(
        "llm:plan-approval-request",
        json!({
            "sessionId": session_id,
            "conversationId": conversation_id,
            "approvalId": approval_id,
            "planText": plan_text,
        }),
    );

    let decision = tokio::select! {
        res = tokio::time::timeout(PLAN_APPROVAL_TIMEOUT, rx) => res,
        _ = cancel.cancelled() => return PlanGateOutcome::Cancelled,
    };

    match decision {
        Ok(Ok(PlanDecision::Approve)) => PlanGateOutcome::Proceed,
        Ok(Ok(PlanDecision::Reject)) => PlanGateOutcome::Reject,
        // Elapsed (Err) or sender dropped (Ok(Err)): degrade to auto-execute.
        Ok(Err(_)) | Err(_) => PlanGateOutcome::Proceed,
    }
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
    plan_approval_state: Arc<PlanApprovalState>,
    planning_approval_enabled: bool,
    memory_manager: SharedMemoryManager,
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

    // Plan-approval gate (between Phase A and Phase B). Skipped when disabled or
    // when there is no real plan to review (too short → Phase A likely failed).
    if planning_approval_enabled && plan_text.trim().len() >= MIN_PLAN_LENGTH {
        let outcome = wait_for_plan_approval(
            &app,
            &session_id,
            &conversation_id,
            &plan_approval_state,
            &plan_text,
            &cancel,
        )
        .await;

        // Dismiss the approval card no matter how the gate resolved — including
        // timeout/cancel, where the user never clicked and the frontend would
        // otherwise leave the card up covering the execution output.
        emit_plan_approval_resolved(&app, &session_id);

        match outcome {
            PlanGateOutcome::Proceed => {}
            PlanGateOutcome::Reject => {
                emit_agent_done(&app, &session_id);
                return String::new();
            }
            PlanGateOutcome::Cancelled => return String::new(),
        }
    }

    // Phase B: Execution (inject plan, normal agent loop)
    if std::env::var("KNOWFORGE_DEBUG_PLANNING").is_ok() {
        eprintln!(
            "[planning] Phase B plan_text ({} chars):\n{}\n[planning] --- end plan ---",
            plan_text.trim().len(),
            plan_text.trim()
        );
    }
    let exec_messages = if plan_text.trim().len() >= MIN_PLAN_LENGTH {
        let steps = parse_plan_steps(&plan_text);
        if !steps.is_empty() {
            emit_plan_steps(&app, &session_id, &steps);
        }
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
        memory_manager,
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
        assert!(result[1].content.contains("plan.update_step"));
        assert!(result[1].content.contains("note.list"));
    }

    #[test]
    fn parse_plan_steps_numbered_list() {
        let plan = "1. Search for notes about quantum physics\n2. Read the most relevant note\n3. Summarize findings";
        let steps = parse_plan_steps(plan);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].step, 1);
        assert_eq!(steps[0].title, "Search for notes about quantum physics");
        assert_eq!(steps[2].step, 3);
        assert_eq!(steps[2].title, "Summarize findings");
    }

    #[test]
    fn parse_plan_steps_with_sub_items() {
        let plan = "1. Search vault\n   - use keyword search\n   - filter by tag\n2. Read results";
        let steps = parse_plan_steps(plan);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step, 1);
        assert_eq!(steps[1].step, 2);
    }

    #[test]
    fn parse_plan_steps_empty_and_prose() {
        assert!(parse_plan_steps("").is_empty());
        assert!(parse_plan_steps("Just do whatever seems right").is_empty());
    }

    #[test]
    fn parse_plan_steps_non_sequential() {
        let plan = "1. First\n3. Third\n5. Fifth";
        let steps = parse_plan_steps(plan);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].step, 3);
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
