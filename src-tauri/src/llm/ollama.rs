//! Ollama HTTP：`/api/tags`、流式 `/api/chat`（NDJSON）。

use super::{emit_chunk, emit_done, emit_error, LlmChatMessage};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

/// NDJSON 单行缓冲上限（字节）。无换行的异常响应无法无限堆积内存。
const MAX_STREAM_LINE_BYTES: usize = 2 * 1024 * 1024;

fn map_reqwest_err(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Request timed out. Check Ollama or increase timeout in settings.".to_string()
    } else if e.is_connect() {
        "Cannot connect to Ollama. Is the service running?".to_string()
    } else {
        format!("Network error: {e}")
    }
}

fn http_client(timeout_ms: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.max(1000)))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))
}

/// `GET {base}/api/tags`
pub async fn list_models(base_url: &str, timeout_ms: u64) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = http_client(timeout_ms)?;
    let resp = client.get(&url).send().await.map_err(map_reqwest_err)?;
    if !resp.status().is_success() {
        return Err(format!("Ollama returned HTTP {}", resp.status()));
    }

    #[derive(Deserialize)]
    struct TagsBody {
        models: Option<Vec<TagModel>>,
    }
    #[derive(Deserialize)]
    struct TagModel {
        name: String,
    }

    let body: TagsBody = resp.json().await.map_err(|e| format!("Invalid response JSON: {e}"))?;
    Ok(body
        .models
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.name)
        .collect())
}

#[derive(Deserialize)]
struct OllamaStreamLine {
    message: Option<OllamaStreamMessage>,
    /// Ollama 末包常为 `true`；当前仅依赖空行结束流，字段保留便于日后提前结束。
    #[serde(default)]
    #[allow(dead_code)]
    done: bool,
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaStreamMessage {
    content: Option<String>,
    /// Ollama tools 调用出现在 `done:false` 中间行上。
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCallRaw>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct OllamaToolCallRaw {
    pub function: OllamaToolFunctionRaw,
}

#[derive(Deserialize, Debug, Clone)]
pub struct OllamaToolFunctionRaw {
    pub name: String,
    /// Ollama 返回已解析的 JSON 参数。
    pub arguments: serde_json::Value,
}

/// `Ok(Some(calls))` 本行携带工具调用；`Ok(None)` 仅含文本/控制信息；
/// 返回负载中包含 `error` 字段时，函数内部会 `emit_error`，调用方可以允许返回 `Ok(None)` 后继续。
///
/// Side effect: appends any emitted text content for this line to `accumulated_content`
/// so the caller can return the full assistant response (used by skill_tool to surface
/// the skill's reply back to the parent LLM as a tool result summary).
fn handle_json_line(
    app: &AppHandle,
    session_id: &str,
    line: &str,
    accumulated_content: &mut String,
) -> Result<(Option<Vec<OllamaToolCallRaw>>, bool), String> {
    let parsed: OllamaStreamLine =
        serde_json::from_str(line).map_err(|e| format!("NDJSON parse error: {e}"))?;

    if let Some(err) = parsed.error {
        emit_error(app, session_id, Some("ollama_error"), &err);
        return Ok((None, true));
    }

    let mut tool_calls_out: Option<Vec<OllamaToolCallRaw>> = None;
    if let Some(msg) = parsed.message {
        if let Some(content) = msg.content.filter(|c| !c.is_empty()) {
            accumulated_content.push_str(&content);
            emit_chunk(app, session_id, &content);
        }
        if let Some(tc) = msg.tool_calls.filter(|v| !v.is_empty()) {
            tool_calls_out = Some(tc);
        }
    }

    Ok((tool_calls_out, false))
}

/// 流式 chat：经 `emit` 推送增量；`cancel` 触发时尽快结束。
///
/// 返回值语义：
/// - `Ok((Some(calls), text))`：本轮流中累计到了 tool_calls，调用方（agent_loop）需执行并接续下一轮。
///   `text` 是本轮模型在 tool_calls 之外同时产出的文本（通常为空字符串）。
/// - `Ok((None, text))`：仅产出了文本、并已 `emit_done`。`text` 是完整 assistant 回复。
/// - `Err(msg)`：出现错误、已 `emit_error`。
pub async fn run_chat_stream(
    app: AppHandle,
    session_id: String,
    base_url: String,
    model: String,
    messages: Vec<LlmChatMessage>,
    temperature: f64,
    top_p: Option<f64>,
    timeout_ms: u64,
    cancel: CancellationToken,
    tools: Option<Vec<serde_json::Value>>,
) -> Result<(Option<Vec<OllamaToolCallRaw>>, String), String> {
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let client = match http_client(timeout_ms) {
        Ok(c) => c,
        Err(e) => {
            emit_error(&app, &session_id, Some("client_error"), &e);
            return Err(e);
        }
    };

    let messages_json: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), json!(m.role));
            obj.insert("content".into(), json!(m.content));
            if let Some(ref tc) = m.tool_calls {
                let arr: Vec<serde_json::Value> = tc
                    .iter()
                    .map(|c| json!({
                        "function": {
                            "name": c.function.name,
                            "arguments": c.function.arguments,
                        }
                    }))
                    .collect();
                obj.insert("tool_calls".into(), serde_json::Value::Array(arr));
            }
            if let Some(ref name) = m.tool_name {
                obj.insert("tool_name".into(), json!(name));
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    let mut options = json!({ "temperature": temperature });
    if let Some(tp) = top_p {
        if let Some(obj) = options.as_object_mut() {
            obj.insert("top_p".into(), json!(tp));
        }
    }

    let mut body = json!({
        "model": model,
        "messages": messages_json,
        "stream": true,
        "options": options,
    });
    if let Some(tools_json) = tools.as_ref().filter(|v| !v.is_empty()) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("tools".into(), serde_json::Value::Array(tools_json.clone()));
        }
    }

    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = map_reqwest_err(e);
            emit_error(
                &app,
                &session_id,
                Some("connection_error"),
                &msg,
            );
            return Err(msg);
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg = if text.len() > 400 {
            format!("Ollama returned HTTP {status}: {}…", &text[..400])
        } else {
            format!("Ollama returned HTTP {status}: {text}")
        };
        emit_error(&app, &session_id, Some("http_status"), &msg);
        return Err(msg);
    }

    let mut stream = resp.bytes_stream();
    let mut line_buf = String::new();
    let mut accumulated_tool_calls: Vec<OllamaToolCallRaw> = Vec::new();
    let mut accumulated_content = String::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                emit_error(&app, &session_id, Some("cancelled"), "Request aborted");
                return Err("cancelled".to_string());
            }
            item = stream.next() => {
                match item {
                    None => break,
                    Some(Err(e)) => {
                        let msg = map_reqwest_err(e);
                        emit_error(&app, &session_id, Some("stream_error"), &msg);
                        return Err(msg);
                    }
                    Some(Ok(bytes)) => {
                        line_buf.push_str(&String::from_utf8_lossy(&bytes));
                        if line_buf.len() > MAX_STREAM_LINE_BYTES {
                            let msg = format!(
                                "Single NDJSON line exceeded {} MiB without newline; aborting.",
                                MAX_STREAM_LINE_BYTES / (1024 * 1024)
                            );
                            emit_error(
                                &app,
                                &session_id,
                                Some("line_too_long"),
                                &msg,
                            );
                            return Err(msg);
                        }
                        while let Some(pos) = line_buf.find('\n') {
                            let raw_line = line_buf[..pos].trim().to_string();
                            line_buf.drain(..=pos);
                            if raw_line.is_empty() {
                                continue;
                            }
                            match handle_json_line(
                                &app,
                                &session_id,
                                &raw_line,
                                &mut accumulated_content,
                            ) {
                                Ok((tc_opt, should_stop)) => {
                                    if let Some(mut tc) = tc_opt {
                                        accumulated_tool_calls.append(&mut tc);
                                    }
                                    if should_stop {
                                        return Err("ollama_error".to_string());
                                    }
                                }
                                Err(e) => {
                                    emit_error(&app, &session_id, Some("parse_error"), &e);
                                    return Err(e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let tail = line_buf.trim();
    if !tail.is_empty() {
        match handle_json_line(&app, &session_id, tail, &mut accumulated_content) {
            Ok((tc_opt, should_stop)) => {
                if let Some(mut tc) = tc_opt {
                    accumulated_tool_calls.append(&mut tc);
                }
                if should_stop {
                    return Err("ollama_error".to_string());
                }
            }
            Err(e) => {
                emit_error(&app, &session_id, Some("parse_error"), &e);
                return Err(e);
            }
        }
    }

    let _ = emit_done(&app, &session_id);
    if accumulated_tool_calls.is_empty() {
        Ok((None, accumulated_content))
    } else {
        Ok((Some(accumulated_tool_calls), accumulated_content))
    }
}

#[derive(Deserialize)]
struct OllamaNonStreamBody {
    message: Option<OllamaNonStreamMessage>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaNonStreamMessage {
    content: Option<String>,
}

/// 单次 `/api/chat`（`stream: false`），返回助手 `message.content` 文本
pub async fn run_chat_completion(
    base_url: &str,
    model: &str,
    messages: &[LlmChatMessage],
    temperature: f64,
    top_p: Option<f64>,
    timeout_ms: u64,
    json_mode: bool,
) -> Result<String, String> {
    let timeout_ms = timeout_ms.max(3000).min(45_000);
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let client = http_client(timeout_ms)?;
    let messages_json: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    let mut options = json!({ "temperature": temperature });
    if let Some(tp) = top_p {
        if let Some(obj) = options.as_object_mut() {
            obj.insert("top_p".into(), json!(tp));
        }
    }

    let mut body = json!({
        "model": model,
        "messages": messages_json,
        "stream": false,
        "options": options,
    });
    if json_mode {
        body["format"] = json!("json");
    }

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(map_reqwest_err)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg = if text.len() > 400 {
            format!("Ollama returned HTTP {status}: {}…", &text[..400])
        } else {
            format!("Ollama returned HTTP {status}: {text}")
        };
        return Err(msg);
    }

    let parsed: OllamaNonStreamBody = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response JSON: {e}"))?;

    if let Some(err) = parsed.error {
        return Err(err);
    }

    Ok(parsed
        .message
        .and_then(|m| m.content)
        .unwrap_or_default())
}
