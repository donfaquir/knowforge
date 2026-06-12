use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::agent_loop::{self, AgentLoopConfig};
use super::approval::ToolApprovalState;
use super::context_guard::ContextGuard;
use super::planning;
use super::provider::{LlmProvider, NormalizedToolCall};
use super::{LlmChatMessage, LlmToolCall, LlmToolCallFunction};
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;

const TIERED_CLOUD_SYSTEM: &str = "\
You are a planning assistant. Analyze the user's request and call the appropriate tools. \
You have access to the user's vault metadata (file names and titles only, no content). \
Call tools to gather information. Do NOT generate a final answer — only make tool calls.";

/// Strip note body content from messages, keeping only metadata.
/// This is the core privacy enforcement for tiered mode.
pub(crate) fn build_tiered_planning_messages(
    messages: &[LlmChatMessage],
) -> Vec<LlmChatMessage> {
    let mut out = Vec::new();
    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: TIERED_CLOUD_SYSTEM.to_string(),
        ..Default::default()
    });

    for m in messages {
        match m.role.as_str() {
            "system" => {
                let stripped = strip_note_body(&m.content);
                if !stripped.trim().is_empty() {
                    out.push(LlmChatMessage {
                        role: "system".to_string(),
                        content: stripped,
                        ..Default::default()
                    });
                }
            }
            "user" | "assistant" => {
                out.push(LlmChatMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    tool_calls: m.tool_calls.clone(),
                    tool_name: m.tool_name.clone(),
                    tool_call_id: m.tool_call_id.clone(),
                });
            }
            _ => {}
        }
    }
    out
}

/// Remove note body content from system messages.
/// Keeps file path references but strips markdown code blocks that contain note content.
fn strip_note_body(content: &str) -> String {
    if !content.contains("```markdown") && !content.contains("```\n") {
        return content.to_string();
    }

    let mut result = String::new();
    let mut in_code_block = false;
    let mut skip_block = false;

    for line in content.lines() {
        if line.trim_start().starts_with("```") && !in_code_block {
            in_code_block = true;
            skip_block = true;
            result.push_str("[note content omitted for privacy]\n");
            continue;
        }
        if line.trim_start().starts_with("```") && in_code_block {
            in_code_block = false;
            skip_block = false;
            continue;
        }
        if !skip_block {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

fn build_tool_result_metadata(
    tc_name: &str,
    success: bool,
    content_len: usize,
) -> String {
    format!(
        "Tool '{}': status={}, result_length={} chars",
        tc_name,
        if success { "ok" } else { "error" },
        content_len,
    )
}

fn build_generation_messages(
    original_messages: &[LlmChatMessage],
    tool_results: &[(String, String, bool)], // (tool_name, result_content, success)
    cloud_guidance: &str,
) -> Vec<LlmChatMessage> {
    let mut out = Vec::new();

    // Keep all original messages (including system messages with full note content)
    for m in original_messages {
        out.push(m.clone());
    }

    // Add tool results as context
    if !tool_results.is_empty() {
        let mut ctx = String::from("The following tool results were gathered:\n\n");
        for (name, content, _success) in tool_results {
            ctx.push_str(&format!("--- {name} ---\n{content}\n\n"));
        }
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: ctx,
            ..Default::default()
        });
    }

    // Add cloud planning guidance
    if !cloud_guidance.trim().is_empty() {
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: format!(
                "Planning guidance from analysis:\n{cloud_guidance}\n\n\
                 Use the tool results above to compose your final answer to the user."
            ),
            ..Default::default()
        });
    }

    out
}

#[allow(clippy::too_many_arguments)]
pub async fn run_tiered_agent(
    cloud_provider: Arc<dyn LlmProvider>,
    local_provider: Arc<dyn LlmProvider>,
    app: AppHandle,
    session_id: String,
    initial_messages: Vec<LlmChatMessage>,
    tools_json: Vec<Value>,
    registry: Arc<ToolRegistry>,
    ctx_factory: Arc<ToolContextFactory>,
    workspace_root: PathBuf,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    cancel: CancellationToken,
    config: AgentLoopConfig,
    conversation_id: String,
    approval_state: Arc<ToolApprovalState>,
) -> String {
    // Step 1: Cloud planning (silent — separate session_id)
    let planning_sid = format!("plan-{}", uuid::Uuid::new_v4());
    let plan_messages = build_tiered_planning_messages(&initial_messages);

    let plan_result = cloud_provider
        .chat_stream(
            &app,
            &planning_sid,
            plan_messages.clone(),
            Some(tools_json.clone()),
            cancel.clone(),
        )
        .await;

    let (cloud_tool_calls, cloud_content) = match plan_result {
        Ok(r) => (r.tool_calls.unwrap_or_default(), r.content),
        Err(_) => {
            // Degradation: cloud failed → fall back to local planning
            return planning::run_planned_agent(
                app,
                session_id,
                initial_messages,
                tools_json,
                registry,
                ctx_factory,
                workspace_root,
                app_cache_dir,
                app_bundle_resource_dir,
                local_provider,
                cancel,
                config,
                conversation_id,
                approval_state,
            )
            .await;
        }
    };

    if cancel.is_cancelled() {
        return String::new();
    }

    // Step 2: Local tool execution (visible to user)
    let mut all_tool_results: Vec<(String, String, bool)> = Vec::new();

    if !cloud_tool_calls.is_empty() {
        let results = execute_tool_calls(
            &app,
            &session_id,
            &cloud_tool_calls,
            &registry,
            &ctx_factory,
            &workspace_root,
            &app_cache_dir,
            &app_bundle_resource_dir,
            &conversation_id,
            &approval_state,
            &cancel,
            &config,
        )
        .await;
        all_tool_results.extend(results);
    }

    if cancel.is_cancelled() {
        return String::new();
    }

    // Step 3: Feedback round (send metadata to cloud, get additional tool calls)
    if !all_tool_results.is_empty() {
        let mut feedback_msgs = plan_messages;

        // Add assistant message with original tool calls
        let llm_tcs: Vec<LlmToolCall> = cloud_tool_calls
            .iter()
            .map(|tc| LlmToolCall {
                id: tc.id.clone(),
                function: LlmToolCallFunction {
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                },
            })
            .collect();
        feedback_msgs.push(LlmChatMessage {
            role: "assistant".to_string(),
            content: cloud_content.clone(),
            tool_calls: Some(llm_tcs),
            ..Default::default()
        });

        // Add tool result metadata (NOT full content)
        for (i, tc) in cloud_tool_calls.iter().enumerate() {
            if let Some((_, content, success)) = all_tool_results.get(i) {
                let metadata = build_tool_result_metadata(&tc.name, *success, content.len());
                let mut msg = cloud_provider.build_tool_result_message(&tc.id, &tc.name, &metadata);
                msg.content = metadata;
                feedback_msgs.push(msg);
            }
        }

        let feedback_result = cloud_provider
            .chat_stream(
                &app,
                &planning_sid,
                feedback_msgs,
                Some(tools_json.clone()),
                cancel.clone(),
            )
            .await;

        if let Ok(r) = feedback_result {
            if let Some(extra_calls) = r.tool_calls {
                if !extra_calls.is_empty() && !cancel.is_cancelled() {
                    let extra_results = execute_tool_calls(
                        &app,
                        &session_id,
                        &extra_calls,
                        &registry,
                        &ctx_factory,
                        &workspace_root,
                        &app_cache_dir,
                        &app_bundle_resource_dir,
                        &conversation_id,
                        &approval_state,
                        &cancel,
                        &config,
                    )
                    .await;
                    all_tool_results.extend(extra_results);
                }
            }
        }
    }

    if cancel.is_cancelled() {
        return String::new();
    }

    // Step 4: Local generation (visible — stream to user)
    let mut gen_messages = build_generation_messages(
        &initial_messages,
        &all_tool_results,
        &cloud_content,
    );

    let context_guard = ContextGuard::with_provider(
        config.max_context_tokens,
        local_provider.clone(),
    );
    context_guard.trim_with_summary(&mut gen_messages).await;

    match local_provider
        .chat_stream(&app, &session_id, gen_messages, None, cancel)
        .await
    {
        Ok(r) => {
            emit_agent_done(&app, &session_id);
            r.content
        }
        Err(_) => {
            emit_agent_done(&app, &session_id);
            String::new()
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_tool_calls(
    app: &AppHandle,
    session_id: &str,
    calls: &[NormalizedToolCall],
    registry: &Arc<ToolRegistry>,
    ctx_factory: &Arc<ToolContextFactory>,
    workspace_root: &PathBuf,
    app_cache_dir: &Option<PathBuf>,
    app_bundle_resource_dir: &Option<PathBuf>,
    conversation_id: &str,
    approval_state: &Arc<ToolApprovalState>,
    cancel: &CancellationToken,
    config: &AgentLoopConfig,
) -> Vec<(String, String, bool)> {
    // Emit tool-call-start for each
    for tc in calls {
        let input_summary = tc.arguments.to_string();
        let summary = truncate_str(&input_summary, 200);
        let _ = app.emit(
            "llm:tool-call-start",
            json!({
                "sessionId": session_id,
                "toolCallId": tc.id,
                "toolName": tc.name,
                "inputSummary": summary,
            }),
        );
    }

    let tool_timeout = Duration::from_millis(config.timeout_ms);
    let results = join_all(calls.iter().map(|tc| {
        let cancel = cancel.clone();
        let registry = registry.clone();
        let ctx_factory = ctx_factory.clone();
        let workspace_root = workspace_root.clone();
        let app_cache_dir = app_cache_dir.clone();
        let app_bundle_resource_dir = app_bundle_resource_dir.clone();
        let conversation_id = conversation_id.to_string();
        let approval_state = approval_state.clone();
        let app = app.clone();
        let session_id = session_id.to_string();
        let tc = tc.clone();
        async move {
            let nesting_depth = config.nesting_depth;
            let exec_start = std::time::Instant::now();
            let result = tokio::select! {
                res = tokio::time::timeout(tool_timeout, agent_loop::execute_tool(
                    &app, &session_id, &registry, &ctx_factory,
                    &workspace_root, app_cache_dir, app_bundle_resource_dir,
                    &conversation_id, &tc, &approval_state, &cancel, nesting_depth,
                )) => {
                    match res {
                        Ok(r) => r,
                        Err(_) => Err(format!("tool '{}' timed out", tc.name)),
                    }
                }
                _ = cancel.cancelled() => Err("cancelled".to_string())
            };
            let duration_ms = exec_start.elapsed().as_millis() as u64;
            let success = result.is_ok();
            let content = match &result {
                Ok(v) => v.to_string(),
                Err(e) => format!("error: {e}"),
            };

            let _ = app.emit(
                "llm:tool-call-done",
                json!({
                    "sessionId": session_id,
                    "toolCallId": tc.id,
                    "success": success,
                    "resultSummary": truncate_str(&content, 200),
                    "durationMs": duration_ms,
                }),
            );

            (tc.name.clone(), content, success)
        }
    }))
    .await;

    results
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

fn emit_agent_done(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        "llm:agent-done",
        json!({ "sessionId": session_id }),
    );
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

    fn user_msg(content: &str) -> LlmChatMessage {
        LlmChatMessage {
            role: "user".to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn privacy_strips_note_body() {
        let note_system = "The user is editing the following Markdown file `notes/test.md`. \
            Use it as the primary source.\n\n```markdown\n# Secret Title\nThis is private content.\n```";
        let msgs = vec![
            sys(note_system),
            sys("Response depth: MEDIUM."),
            user_msg("What is in my note?"),
        ];

        let result = build_tiered_planning_messages(&msgs);

        // Cloud system prompt is first
        assert_eq!(result[0].content, TIERED_CLOUD_SYSTEM);

        // Note body must be stripped
        let all_content: String = result.iter().map(|m| m.content.clone()).collect();
        assert!(!all_content.contains("This is private content"));
        assert!(!all_content.contains("Secret Title"));
        assert!(all_content.contains("[note content omitted for privacy]"));

        // User message preserved
        assert!(result.iter().any(|m| m.role == "user" && m.content == "What is in my note?"));
    }

    #[test]
    fn privacy_preserves_non_note_system() {
        let msgs = vec![
            sys("Response depth: MEDIUM. Keep your answer moderate."),
            user_msg("hello"),
        ];
        let result = build_tiered_planning_messages(&msgs);
        assert!(result.iter().any(|m| m.content.contains("MEDIUM")));
    }

    #[test]
    fn privacy_strips_kf_private_note() {
        let private_note = "The user has a Markdown note open that is marked private (kf-private). \
            Its full content is not included in this request.";
        let msgs = vec![sys(private_note), user_msg("hi")];
        let result = build_tiered_planning_messages(&msgs);
        // The privacy placeholder should pass through (it's already stripped)
        assert!(result.iter().any(|m| m.content.contains("kf-private")));
    }

    #[test]
    fn build_generation_includes_tool_results() {
        let msgs = vec![user_msg("find my notes")];
        let results = vec![
            ("note.list".to_string(), "[\"a.md\", \"b.md\"]".to_string(), true),
        ];
        let gen_msgs = build_generation_messages(&msgs, &results, "Use note.list to find files");
        // Original messages preserved
        assert!(gen_msgs.iter().any(|m| m.role == "user"));
        // Tool results included
        assert!(gen_msgs.iter().any(|m| m.content.contains("note.list") && m.content.contains("a.md")));
        // Cloud guidance included
        assert!(gen_msgs.iter().any(|m| m.content.contains("Planning guidance")));
    }

    #[test]
    fn tool_result_metadata_format() {
        let meta = build_tool_result_metadata("note.read", true, 1234);
        assert_eq!(meta, "Tool 'note.read': status=ok, result_length=1234 chars");
    }
}
