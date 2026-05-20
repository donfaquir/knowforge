//! P2 Tool Calling Loop：在原文字流的基础上，识别 Ollama 返回的 tool_calls，
//! 经 ToolContext（含 privacy_filter）执行工具，并把结果回喂下一轮模型对话。
//!
//! 关键约束：
//! - 工具调用必须经过 [`ToolRegistry`] + [`ToolContextFactory`]，绝不可绕过。
//! - tool_call 追踪 ID 由服务端生成（uuid v7）。
//! - tool 结果消息使用 `tool_name` 字段（非 `tool_call_id`），与 Ollama 协议保持一致。

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::join_all;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::ollama::{self, OllamaToolCallRaw};
use super::{LlmChatMessage, LlmToolCall, LlmToolCallFunction};
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;
use crate::tools::types::ToolManifest;

/// Agent Loop 上限配置；任一项达到上限即终止循环并 emit `llm:agent-done`。
#[allow(dead_code)]
pub struct AgentLoopConfig {
    /// 整轮 agent 中允许执行的 tool_call 总次数上限。
    pub max_tool_calls: u8,
    /// 每轮模型流式请求的默认超时（毫秒）；现代码从调用点的 ai.request.timeout_ms 传入。
    pub timeout_ms: u64,
    /// 整轮中累计追加给模型的 tool 结果总字符数上限（防止上下文爆炸）。
    pub max_tool_result_chars: usize,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_calls: 8,
            timeout_ms: 60_000,
            max_tool_result_chars: 8000,
        }
    }
}

/// 启动 Tool Calling Loop。当 LLM 不再返回 tool_calls 时正常结束并 emit `llm:agent-done`。
pub async fn run_agent_stream(
    app: AppHandle,
    session_id: String,
    initial_messages: Vec<LlmChatMessage>,
    tools_json: Vec<Value>,
    registry: Arc<ToolRegistry>,
    ctx_factory: Arc<ToolContextFactory>,
    workspace_root: PathBuf,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    base_url: String,
    model: String,
    temperature: f64,
    top_p: Option<f64>,
    timeout_ms: u64,
    cancel: CancellationToken,
    config: AgentLoopConfig,
    conversation_id: String,
) {
    let mut messages = initial_messages;
    let mut total_tool_result_chars: usize = 0;
    let mut tool_call_count: u8 = 0;

    loop {
        if cancel.is_cancelled() {
            return;
        }

        // 1. 流式请求（携带 tools 字段；本轮文字会通过 emit_chunk/emit_done 推给前端）
        let raw_tool_calls = match ollama::run_chat_stream(
            app.clone(),
            session_id.clone(),
            base_url.clone(),
            model.clone(),
            messages.clone(),
            temperature,
            top_p,
            timeout_ms,
            cancel.clone(),
            Some(tools_json.clone()),
        )
        .await
        {
            Ok(tc) => tc,
            Err(_) => {
                // run_chat_stream 内部已 emit_error；此处补一个 agent-done 收尾。
                emit_agent_done(&app, &session_id);
                return;
            }
        };

        // 2. 无工具调用 → agent 循环完成
        let raw_calls = match raw_tool_calls {
            Some(calls) if !calls.is_empty() => calls,
            _ => {
                emit_agent_done(&app, &session_id);
                return;
            }
        };

        // 3. 为每个 tool_call 生成追踪 ID（UUID v7）
        let calls_with_ids: Vec<(String, OllamaToolCallRaw)> = raw_calls
            .into_iter()
            .map(|tc| (uuid::Uuid::now_v7().to_string(), tc))
            .collect();

        for (id, tc) in &calls_with_ids {
            emit_tool_call_start(&app, &session_id, id, &tc.function.name);
        }

        // 4. 并行执行工具
        let results = join_all(calls_with_ids.iter().map(|(id, tc)| {
            execute_tool(
                &registry,
                &ctx_factory,
                &workspace_root,
                app_cache_dir.clone(),
                app_bundle_resource_dir.clone(),
                &conversation_id,
                id,
                tc,
            )
        }))
        .await;

        for (i, (id, _)) in calls_with_ids.iter().enumerate() {
            let success = results.get(i).map(|r| r.is_ok()).unwrap_or(false);
            emit_tool_call_done(&app, &session_id, id, success);
        }

        // 5. 把含 tool_calls 的 assistant 消息追加到历史
        let llm_tool_calls: Vec<LlmToolCall> = calls_with_ids
            .iter()
            .map(|(id, tc)| LlmToolCall {
                id: id.clone(),
                function: LlmToolCallFunction {
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                },
            })
            .collect();

        messages.push(LlmChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(llm_tool_calls),
            tool_name: None,
        });

        // 6. 把每个 tool 结果以 role=tool 的消息追加到历史（tool_name 字段对齐 Ollama 协议）
        for (i, (_, tc)) in calls_with_ids.iter().enumerate() {
            let raw_content = match results.get(i) {
                Some(Ok(val)) => val.to_string(),
                Some(Err(e)) => format!("error: {}", e),
                None => "error: no result".to_string(),
            };
            // 在 max_tool_result_chars 预算内截断本条结果，避免上下文爆炸
            let remaining = config
                .max_tool_result_chars
                .saturating_sub(total_tool_result_chars);
            let content = if raw_content.len() > remaining {
                // 安全截断到字符边界
                let mut end = remaining;
                while end > 0 && !raw_content.is_char_boundary(end) {
                    end -= 1;
                }
                raw_content[..end].to_string()
            } else {
                raw_content
            };
            total_tool_result_chars = total_tool_result_chars.saturating_add(content.len());

            messages.push(LlmChatMessage {
                role: "tool".to_string(),
                content,
                tool_calls: None,
                tool_name: Some(tc.function.name.clone()),
            });
        }

        // 7. 上限检查
        tool_call_count = tool_call_count.saturating_add(calls_with_ids.len() as u8);
        if tool_call_count >= config.max_tool_calls
            || total_tool_result_chars >= config.max_tool_result_chars
        {
            emit_agent_done(&app, &session_id);
            return;
        }

        if cancel.is_cancelled() {
            return;
        }
        // 继续 loop：下一轮流式请求会带上完整的 messages 历史
    }
}

async fn execute_tool(
    registry: &Arc<ToolRegistry>,
    ctx_factory: &Arc<ToolContextFactory>,
    workspace_root: &PathBuf,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    conversation_id: &str,
    _call_id: &str,
    tc: &OllamaToolCallRaw,
) -> Result<Value, String> {
    let tool = registry
        .get(&tc.function.name)
        .ok_or_else(|| format!("tool not found: {}", tc.function.name))?;

    tool.validate_input(&tc.function.arguments)
        .map_err(|e| format!("validation failed: {}", e.message))?;

    let ctx = ctx_factory.create_context(
        workspace_root.clone(),
        conversation_id,
        app_cache_dir,
        app_bundle_resource_dir,
    );

    let result = tool.invoke(&ctx, tc.function.arguments.clone()).await;
    match result {
        crate::tools::types::ToolResult::Ok { data, .. } => Ok(data),
        crate::tools::types::ToolResult::PartialOk { data, .. } => Ok(data),
        crate::tools::types::ToolResult::Err { error } => Err(format!("{:?}", error.code)),
    }
}

/// 把单个 manifest 转换为 Ollama `tools` 字段要求的格式。
#[allow(dead_code)]
pub fn manifest_to_ollama_tool(manifest: &ToolManifest) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": manifest.name,
            "description": manifest.description,
            "parameters": manifest.input_schema,
        }
    })
}

/// 从 [`ToolRegistry::list_for_llm`] 返回的精简 JSON 列表批量转 Ollama tools 数组。
pub fn list_for_llm_to_ollama_tools(manifests: &[Value]) -> Vec<Value> {
    manifests
        .iter()
        .map(|m| {
            json!({
                "type": "function",
                "function": {
                    "name": m.get("name").cloned().unwrap_or(Value::Null),
                    "description": m.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": m.get("input_schema").cloned().unwrap_or(Value::Null),
                }
            })
        })
        .collect()
}

fn emit_tool_call_start(app: &AppHandle, session_id: &str, tool_call_id: &str, tool_name: &str) {
    let _ = app.emit(
        "llm:tool-call-start",
        json!({
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "toolName": tool_name,
        }),
    );
}

fn emit_tool_call_done(app: &AppHandle, session_id: &str, tool_call_id: &str, success: bool) {
    let _ = app.emit(
        "llm:tool-call-done",
        json!({
            "sessionId": session_id,
            "toolCallId": tool_call_id,
            "success": success,
        }),
    );
}

fn emit_agent_done(app: &AppHandle, session_id: &str) {
    let _ = app.emit(
        "llm:agent-done",
        json!({ "sessionId": session_id }),
    );
}
