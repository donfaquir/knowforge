use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use super::provider::{ChatStreamResult, CompletionOverrides, LlmProvider, NormalizedToolCall};
use super::ollama;
use super::LlmChatMessage;

pub struct OllamaProvider {
    base_url: String,
    model: String,
    temperature: f64,
    top_p: Option<f64>,
    timeout_ms: u64,
}

impl OllamaProvider {
    pub fn new(
        base_url: String,
        model: String,
        temperature: f64,
        top_p: Option<f64>,
        timeout_ms: u64,
    ) -> Self {
        Self {
            base_url,
            model,
            temperature,
            top_p,
            timeout_ms,
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn chat_stream(
        &self,
        app: &AppHandle,
        session_id: &str,
        messages: Vec<LlmChatMessage>,
        tools: Option<Vec<Value>>,
        cancel: CancellationToken,
    ) -> Result<ChatStreamResult, String> {
        let (raw_calls, content) = ollama::run_chat_stream(
            app.clone(),
            session_id.to_string(),
            self.base_url.clone(),
            self.model.clone(),
            messages,
            self.temperature,
            self.top_p,
            self.timeout_ms,
            cancel,
            tools,
        )
        .await?;

        let tool_calls = raw_calls.map(|calls| {
            calls
                .into_iter()
                .map(|tc| NormalizedToolCall {
                    id: uuid::Uuid::now_v7().to_string(),
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                })
                .collect()
        });

        Ok(ChatStreamResult {
            tool_calls,
            content,
        })
    }

    async fn chat_completion(
        &self,
        messages: &[LlmChatMessage],
        overrides: Option<&CompletionOverrides>,
    ) -> Result<String, String> {
        let temp = overrides
            .and_then(|o| o.temperature)
            .unwrap_or(self.temperature);
        let top_p = match overrides.and_then(|o| o.top_p) {
            Some(v) => v,
            None => self.top_p,
        };
        let timeout = overrides
            .and_then(|o| o.timeout_ms)
            .unwrap_or(self.timeout_ms);
        let json_mode = overrides.map_or(false, |o| o.json_mode);

        ollama::run_chat_completion(
            &self.base_url,
            &self.model,
            messages,
            temp,
            top_p,
            timeout,
            json_mode,
        )
        .await
    }

    async fn list_models(&self) -> Result<Vec<String>, String> {
        ollama::list_models(&self.base_url, self.timeout_ms).await
    }

    fn convert_tools(&self, manifests: &[Value]) -> Vec<Value> {
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

    fn build_tool_result_message(
        &self,
        _call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> LlmChatMessage {
        LlmChatMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_name: Some(tool_name.to_string()),
            tool_call_id: None,
        }
    }

    fn provider_name(&self) -> &'static str {
        "ollama"
    }
}
