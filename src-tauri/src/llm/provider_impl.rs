use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use super::provider::{ChatStreamResult, CompletionOverrides, LlmProvider, NormalizedToolCall};
use super::{emit_chunk, emit_done, emit_error, LlmChatMessage};

pub struct UnifiedProvider {
    client: Arc<reqwest::Client>,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f64,
    top_p: Option<f64>,
    timeout_ms: u64,
    organization_id: Option<String>,
}

// OpenAI API enforces ^[a-zA-Z0-9_-]+$ for function names — dots are not
// allowed.  Internal tool names use dots as namespace separators
// (e.g. "note.read"), so we translate at the API boundary only.
// Hyphens are safe for round-tripping because the internal naming regex
// forbids them, making the mapping bijective.

fn to_api_tool_name(internal: &str) -> String {
    internal.replace('.', "-")
}

fn from_api_tool_name(api: &str) -> String {
    api.replace('-', ".")
}

impl UnifiedProvider {
    pub fn new(
        client: Arc<reqwest::Client>,
        base_url: String,
        api_key: String,
        model: String,
        temperature: f64,
        top_p: Option<f64>,
        timeout_ms: u64,
        organization_id: Option<String>,
    ) -> Self {
        Self {
            client,
            base_url,
            api_key,
            model,
            temperature,
            top_p,
            timeout_ms,
            organization_id,
        }
    }

    fn build_auth_headers(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        let builder = if self.api_key.is_empty() {
            builder
        } else {
            builder.bearer_auth(&self.api_key)
        };
        if let Some(ref org) = self.organization_id {
            if !org.trim().is_empty() {
                return builder.header("OpenAI-Organization", org);
            }
        }
        builder
    }

    fn serialize_messages(messages: &[LlmChatMessage]) -> Vec<Value> {
        messages
            .iter()
            .map(|m| {
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), json!(m.role));

                if m.role == "tool" {
                    obj.insert("content".into(), json!(m.content));
                    if let Some(ref id) = m.tool_call_id {
                        obj.insert("tool_call_id".into(), json!(id));
                    }
                    return Value::Object(obj);
                }

                // OpenAI API: content:null is only valid when tool_calls are present.
                if m.content.is_empty() {
                    if m.tool_calls.as_ref().map_or(true, |tc| tc.is_empty()) {
                        obj.insert("content".into(), json!(""));
                    } else {
                        obj.insert("content".into(), Value::Null);
                    }
                } else {
                    obj.insert("content".into(), json!(m.content));
                }
                if let Some(ref tc) = m.tool_calls {
                    let arr: Vec<Value> = tc
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": to_api_tool_name(&c.function.name),
                                    "arguments": if c.function.arguments.is_object() {
                                        c.function.arguments.to_string()
                                    } else {
                                        c.function.arguments.as_str().unwrap_or("{}").to_string()
                                    },
                                }
                            })
                        })
                        .collect();
                    obj.insert("tool_calls".into(), Value::Array(arr));
                }
                Value::Object(obj)
            })
            .collect()
    }
}

// --- SSE Response Types ---

#[derive(Debug, Deserialize)]
struct SseChunk {
    choices: Option<Vec<SseChoice>>,
}

#[derive(Debug, Deserialize)]
struct SseChoice {
    delta: Option<SseDelta>,
    #[allow(dead_code)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SseDelta {
    content: Option<String>,
    tool_calls: Option<Vec<SseToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct SseToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<SseFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct SseFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

struct PendingToolCall {
    id: String,
    name: String,
    arguments_buf: String,
}

// --- Non-streaming response types ---

#[derive(Debug, Deserialize)]
struct CompletionResponse {
    choices: Option<Vec<CompletionChoice>>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct CompletionChoice {
    message: Option<CompletionMessage>,
}

#[derive(Debug, Deserialize)]
struct CompletionMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<ModelEntry>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

const MAX_SSE_LINE_BYTES: usize = 2 * 1024 * 1024;

#[async_trait]
impl LlmProvider for UnifiedProvider {
    async fn chat_stream(
        &self,
        app: &AppHandle,
        session_id: &str,
        messages: Vec<LlmChatMessage>,
        tools: Option<Vec<Value>>,
        cancel: CancellationToken,
    ) -> Result<ChatStreamResult, String> {
        let url = format!(
            "{}/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let messages_json = Self::serialize_messages(&messages);

        let mut body = json!({
            "model": self.model,
            "messages": messages_json,
            "stream": true,
            "temperature": self.temperature,
        });
        if let Some(tp) = self.top_p {
            body["top_p"] = json!(tp);
        }
        if let Some(ref tools_val) = tools {
            if !tools_val.is_empty() {
                body["tools"] = Value::Array(tools_val.clone());
            }
        }

        let msg_count = messages.len();
        let est_tokens: usize = messages_json.iter().map(|v| v.to_string().len() / 3).sum();
        let idle_timeout = std::time::Duration::from_millis(self.timeout_ms);
        eprintln!(
            "[provider] session={} chat_stream start: model={} msgs={} est_tokens={} idle_timeout={}ms tools={}",
            &session_id[..8.min(session_id.len())],
            self.model,
            msg_count,
            est_tokens,
            self.timeout_ms,
            tools.as_ref().map_or(0, |t| t.len()),
        );
        let request_start = std::time::Instant::now();

        // No request-level timeout — use idle detection in the SSE loop instead.
        // The connect timeout on the shared client (15s) still protects against
        // unreachable hosts.
        let req = self.build_auth_headers(self.client.post(&url)).json(&body);
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("OpenAI connection error: {e}");
                eprintln!(
                    "[provider] session={} connection error after {:.1}s: {}",
                    &session_id[..8.min(session_id.len())],
                    request_start.elapsed().as_secs_f64(),
                    e,
                );
                emit_error(app, session_id, Some("connection_error"), &msg);
                return Err(msg);
            }
        };

        eprintln!(
            "[provider] session={} HTTP {} after {:.1}s, starting SSE stream",
            &session_id[..8.min(session_id.len())],
            resp.status(),
            request_start.elapsed().as_secs_f64(),
        );

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let msg = if text.len() > 400 {
                format!("OpenAI HTTP {status}: {}…", &text[..400])
            } else {
                format!("OpenAI HTTP {status}: {text}")
            };
            emit_error(app, session_id, Some("http_status"), &msg);
            return Err(msg);
        }

        let mut stream = resp.bytes_stream();
        let mut line_buf = String::new();
        let mut accumulated_content = String::new();
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
        let tools_provided = tools.as_ref().map_or(false, |t| !t.is_empty());

        let mut last_chunk_at = std::time::Instant::now();
        let mut chunk_count: u64 = 0;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    eprintln!(
                        "[provider] session={} SSE cancelled after {:.1}s, {} chunks, content_len={}",
                        &session_id[..8.min(session_id.len())],
                        request_start.elapsed().as_secs_f64(),
                        chunk_count,
                        accumulated_content.len(),
                    );
                    emit_error(app, session_id, Some("cancelled"), "Request aborted");
                    return Err("cancelled".to_string());
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(last_chunk_at + idle_timeout)) => {
                    let idle_secs = last_chunk_at.elapsed().as_secs_f64();
                    let msg = format!(
                        "SSE idle timeout: no data received for {:.0}s",
                        idle_secs,
                    );
                    eprintln!(
                        "[provider] session={} {} after {:.1}s total, {} chunks, content_len={}",
                        &session_id[..8.min(session_id.len())],
                        msg,
                        request_start.elapsed().as_secs_f64(),
                        chunk_count,
                        accumulated_content.len(),
                    );
                    emit_error(app, session_id, Some("idle_timeout"), &msg);
                    return Err(msg);
                }
                item = stream.next() => {
                    match item {
                        None => {
                            eprintln!(
                                "[provider] session={} SSE stream ended (None): {:.1}s total, {} chunks, content_len={} pending_tools={}",
                                &session_id[..8.min(session_id.len())],
                                request_start.elapsed().as_secs_f64(),
                                chunk_count,
                                accumulated_content.len(),
                                pending_tool_calls.len(),
                            );
                            break;
                        }
                        Some(Err(e)) => {
                            let idle_secs = last_chunk_at.elapsed().as_secs_f64();
                            let msg = format!("OpenAI stream error: {e}");
                            eprintln!(
                                "[provider] session={} SSE error after {:.1}s (idle {:.1}s): {}",
                                &session_id[..8.min(session_id.len())],
                                request_start.elapsed().as_secs_f64(),
                                idle_secs,
                                e,
                            );
                            emit_error(app, session_id, Some("stream_error"), &msg);
                            return Err(msg);
                        }
                        Some(Ok(bytes)) => {
                            chunk_count += 1;
                            last_chunk_at = std::time::Instant::now();
                            line_buf.push_str(&String::from_utf8_lossy(&bytes));
                            if line_buf.len() > MAX_SSE_LINE_BYTES {
                                let msg = "SSE buffer exceeded 2 MiB; aborting.".to_string();
                                emit_error(app, session_id, Some("line_too_long"), &msg);
                                return Err(msg);
                            }

                            while let Some(pos) = line_buf.find('\n') {
                                let raw_line = line_buf[..pos].trim().to_string();
                                line_buf.drain(..=pos);

                                if raw_line.is_empty() {
                                    continue;
                                }

                                let data = if let Some(stripped) = raw_line.strip_prefix("data: ") {
                                    stripped.trim()
                                } else {
                                    continue;
                                };

                                if data == "[DONE]" {
                                    break;
                                }

                                let chunk: SseChunk = match serde_json::from_str(data) {
                                    Ok(c) => c,
                                    Err(_) => continue,
                                };

                                if let Some(choices) = chunk.choices {
                                    for choice in choices {
                                        if let Some(delta) = choice.delta {
                                            if let Some(text) = delta.content {
                                                accumulated_content.push_str(&text);
                                                emit_chunk(app, session_id, &text);
                                            }
                                            if let Some(tc_deltas) = delta.tool_calls {
                                                if tools_provided {
                                                    for tc_delta in tc_deltas {
                                                        let idx = tc_delta.index;
                                                        while pending_tool_calls.len() <= idx {
                                                            pending_tool_calls.push(PendingToolCall {
                                                                id: String::new(),
                                                                name: String::new(),
                                                                arguments_buf: String::new(),
                                                            });
                                                        }
                                                        let pending = &mut pending_tool_calls[idx];
                                                        if let Some(id) = tc_delta.id {
                                                            pending.id = id;
                                                        }
                                                        if let Some(func) = tc_delta.function {
                                                            if let Some(name) = func.name {
                                                                pending.name = name;
                                                            }
                                                            if let Some(args) = func.arguments {
                                                                pending.arguments_buf.push_str(&args);
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    for tc_delta in tc_deltas {
                                                        if let Some(func) = tc_delta.function {
                                                            if let Some(args) = func.arguments {
                                                                accumulated_content.push_str(&args);
                                                                emit_chunk(app, session_id, &args);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let _ = emit_done(app, session_id);

        if pending_tool_calls.is_empty() {
            Ok(ChatStreamResult {
                tool_calls: None,
                content: accumulated_content,
            })
        } else {
            let tool_calls: Vec<NormalizedToolCall> = pending_tool_calls
                .into_iter()
                .filter(|p| !p.name.is_empty())
                .map(|p| {
                    let arguments = serde_json::from_str(&p.arguments_buf)
                        .unwrap_or(Value::Object(serde_json::Map::new()));
                    NormalizedToolCall {
                        id: if p.id.is_empty() {
                            uuid::Uuid::now_v7().to_string()
                        } else {
                            p.id
                        },
                        name: from_api_tool_name(&p.name),
                        arguments,
                    }
                })
                .collect();
            Ok(ChatStreamResult {
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                content: accumulated_content,
            })
        }
    }

    async fn chat_completion(
        &self,
        messages: &[LlmChatMessage],
        overrides: Option<&CompletionOverrides>,
    ) -> Result<String, String> {
        let temp = overrides
            .and_then(|o| o.temperature)
            .unwrap_or(self.temperature);
        let top_p_val = match overrides.and_then(|o| o.top_p) {
            Some(v) => v,
            None => self.top_p,
        };
        let timeout = overrides
            .and_then(|o| o.timeout_ms)
            .unwrap_or(self.timeout_ms);

        let url = format!(
            "{}/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let messages_json = Self::serialize_messages(messages);

        let mut body = json!({
            "model": self.model,
            "messages": messages_json,
            "temperature": temp,
        });
        if let Some(tp) = top_p_val {
            body["top_p"] = json!(tp);
        }
        if overrides.map_or(false, |o| o.json_mode) {
            body["response_format"] = json!({"type": "json_object"});
        }

        let req = self.build_auth_headers(
            self.client
                .post(&url)
                .timeout(std::time::Duration::from_millis(timeout.max(3000).min(45_000))),
        )
        .json(&body);
        let resp = req
            .send()
            .await
            .map_err(|e| format!("OpenAI request error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("OpenAI HTTP {status}: {text}"));
        }

        let body: CompletionResponse = resp
            .json()
            .await
            .map_err(|e| format!("OpenAI parse error: {e}"))?;

        if let Some(err) = body.error {
            return Err(format!("OpenAI API error: {}", err.message));
        }

        body.choices
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.message)
            .and_then(|m| m.content)
            .ok_or_else(|| "OpenAI returned empty response".to_string())
    }

    async fn list_models(&self) -> Result<Vec<String>, String> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let req = self.build_auth_headers(
            self.client
                .get(&url)
                .timeout(std::time::Duration::from_millis(self.timeout_ms)),
        );
        let resp = req
            .send()
            .await
            .map_err(|e| format!("OpenAI models request error: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("OpenAI HTTP {status}: {text}"));
        }

        let body: ModelsResponse = resp
            .json()
            .await
            .map_err(|e| format!("OpenAI models parse error: {e}"))?;

        Ok(body.data.unwrap_or_default().into_iter().map(|m| m.id).collect())
    }

    fn convert_tools(&self, manifests: &[Value]) -> Vec<Value> {
        manifests
            .iter()
            .map(|m| {
                let api_name = m
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(to_api_tool_name)
                    .map(Value::String)
                    .unwrap_or(Value::Null);
                json!({
                    "type": "function",
                    "function": {
                        "name": api_name,
                        "description": m.get("description").cloned().unwrap_or(Value::Null),
                        "parameters": m.get("input_schema").cloned().unwrap_or(Value::Null),
                    }
                })
            })
            .collect()
    }

    fn build_tool_result_message(
        &self,
        call_id: &str,
        _tool_name: &str,
        content: &str,
    ) -> LlmChatMessage {
        LlmChatMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_name: None,
            tool_call_id: Some(call_id.to_string()),
        }
    }

    fn provider_name(&self) -> &'static str {
        "openai-compatible"
    }

    fn model_context_window(&self) -> Option<usize> {
        infer_context_window(&self.model)
    }
}

const MAX_INFERRED_CONTEXT: usize = 524_288;

fn infer_context_window(model: &str) -> Option<usize> {
    let m = model.to_lowercase();

    let raw = if m.contains("qwen-long") {
        10_000_000
    } else if m.contains("qwen-max") || m.contains("qwen-plus") {
        131_072
    } else if m.contains("qwen-turbo") {
        131_072
    } else if m.contains("qwen2") || m.contains("qwen3") {
        131_072
    } else if m.contains("qwen") {
        32_768
    } else if m.contains("gpt-4o") || m.contains("gpt-4-turbo") {
        128_000
    } else if m.contains("gpt-4") {
        8_192
    } else if m.contains("gpt-3.5") {
        16_385
    } else if m.contains("llama-4") || m.contains("llama4") {
        if m.contains("scout") {
            10_000_000
        } else {
            1_000_000
        }
    } else if m.contains("llama-3") || m.contains("llama3") {
        if m.contains("3.1") || m.contains("3.2") || m.contains("3.3") {
            128_000
        } else {
            8_192
        }
    } else if m.contains("llama2") || m.contains("llama-2") {
        4_096
    } else if m.contains("deepseek") {
        128_000
    } else if m.contains("gemma4") || m.contains("gemma-4") {
        262_144
    } else if m.contains("gemma3") || m.contains("gemma-3") {
        128_000
    } else if m.contains("gemma") {
        8_192
    } else {
        return None;
    };

    Some(raw.min(MAX_INFERRED_CONTEXT))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> Arc<reqwest::Client> {
        Arc::new(reqwest::Client::new())
    }

    #[test]
    fn serialize_tool_result_message() {
        let provider = UnifiedProvider::new(
            test_client(),
            "https://api.openai.com/v1".to_string(),
            "test-key".to_string(),
            "gpt-4o".to_string(),
            0.7,
            None,
            30000,
            None,
        );
        let msg = provider.build_tool_result_message("call_123", "web.search", "some result");
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
        assert!(msg.tool_name.is_none());
    }

    #[test]
    fn serialize_messages_tool_role() {
        let messages = vec![LlmChatMessage {
            role: "tool".to_string(),
            content: "result text".to_string(),
            tool_calls: None,
            tool_name: None,
            tool_call_id: Some("call_abc".to_string()),
        }];
        let json = UnifiedProvider::serialize_messages(&messages);
        assert_eq!(json.len(), 1);
        let obj = json[0].as_object().unwrap();
        assert_eq!(obj.get("tool_call_id").unwrap(), "call_abc");
    }

    #[test]
    fn serialize_messages_assistant_with_tool_calls() {
        use super::super::{LlmToolCall, LlmToolCallFunction};
        let messages = vec![LlmChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![LlmToolCall {
                id: "call_xyz".to_string(),
                function: LlmToolCallFunction {
                    name: "web.search".to_string(),
                    arguments: json!({"query": "test"}),
                },
            }]),
            tool_name: None,
            tool_call_id: None,
        }];
        let json = UnifiedProvider::serialize_messages(&messages);
        let obj = json[0].as_object().unwrap();
        assert!(obj.get("content").unwrap().is_null());
        let tcs = obj.get("tool_calls").unwrap().as_array().unwrap();
        assert_eq!(tcs[0]["id"], "call_xyz");
        assert_eq!(tcs[0]["type"], "function");
        assert!(tcs[0]["function"]["arguments"].is_string());
        assert_eq!(tcs[0]["function"]["name"], "web-search");
    }

    #[test]
    fn to_api_tool_name_replaces_dots() {
        assert_eq!(to_api_tool_name("note.read"), "note-read");
        assert_eq!(to_api_tool_name("vault.search_keyword"), "vault-search_keyword");
        assert_eq!(to_api_tool_name("time.now"), "time-now");
        assert_eq!(to_api_tool_name("skill.writing_coach"), "skill-writing_coach");
    }

    #[test]
    fn from_api_tool_name_restores_dots() {
        assert_eq!(from_api_tool_name("note-read"), "note.read");
        assert_eq!(from_api_tool_name("vault-search_keyword"), "vault.search_keyword");
        assert_eq!(from_api_tool_name("time-now"), "time.now");
        assert_eq!(from_api_tool_name("skill-writing_coach"), "skill.writing_coach");
    }

    #[test]
    fn tool_name_round_trip() {
        let names = [
            "note.read", "note.list", "note.write_section", "note.create", "note.append",
            "vault.search_keyword", "vault.semantic_search",
            "thought.list", "thought.create",
            "web.search", "web.read_page", "web.download", "web.read_pdf",
            "graph.query_topic_network", "index.status", "link.suggest_related",
            "memory.save", "memory.forget", "time.now",
            "skill.writing_coach", "skill.web_research",
        ];
        for name in &names {
            assert_eq!(&from_api_tool_name(&to_api_tool_name(name)), name);
        }
    }

    #[test]
    fn convert_tools_maps_names() {
        let provider = UnifiedProvider::new(
            test_client(),
            "https://api.openai.com/v1".to_string(),
            "k".to_string(),
            "m".to_string(),
            0.7,
            None,
            30000,
            None,
        );
        let manifests = vec![json!({
            "name": "web.search",
            "description": "Search the web",
            "input_schema": {"type": "object"}
        })];
        let tools = provider.convert_tools(&manifests);
        assert_eq!(tools[0]["function"]["name"], "web-search");
    }

    #[test]
    fn infer_context_window_known_models() {
        assert_eq!(infer_context_window("qwen-plus"), Some(131_072));
        assert_eq!(infer_context_window("qwen-turbo-latest"), Some(131_072));
        assert_eq!(infer_context_window("qwen3-235b-a22b"), Some(131_072));
        assert_eq!(infer_context_window("gpt-4o"), Some(128_000));
        assert_eq!(infer_context_window("gpt-4"), Some(8_192));
        assert_eq!(infer_context_window("gpt-3.5-turbo"), Some(16_385));
        assert_eq!(infer_context_window("deepseek-chat"), Some(128_000));
        assert_eq!(infer_context_window("llama3.3-70b"), Some(128_000));
        assert_eq!(infer_context_window("llama3-8b"), Some(8_192));
        assert_eq!(infer_context_window("gemma3-27b"), Some(128_000));
    }

    #[test]
    fn infer_context_window_clamped_to_max() {
        // qwen-long has 10M native window but should be clamped to 512K
        assert_eq!(infer_context_window("qwen-long"), Some(MAX_INFERRED_CONTEXT));
        assert_eq!(infer_context_window("llama4-scout"), Some(MAX_INFERRED_CONTEXT));
        assert_eq!(MAX_INFERRED_CONTEXT, 524_288);
    }

    #[test]
    fn infer_context_window_unknown_returns_none() {
        assert_eq!(infer_context_window("my-custom-model"), None);
        assert_eq!(infer_context_window("some-random-name"), None);
    }

    #[test]
    fn serialize_messages_empty_content_no_tool_calls() {
        let messages = vec![LlmChatMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_name: None,
            tool_call_id: None,
        }];
        let json = UnifiedProvider::serialize_messages(&messages);
        let obj = json[0].as_object().unwrap();
        assert_eq!(obj.get("content").unwrap(), "");
    }
}
