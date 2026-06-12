use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use super::LlmChatMessage;
use crate::vault_config::{ActiveProvider, AiConfig};

#[derive(Debug, Clone)]
pub struct NormalizedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

pub struct ChatStreamResult {
    pub tool_calls: Option<Vec<NormalizedToolCall>>,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionOverrides {
    pub temperature: Option<f64>,
    pub top_p: Option<Option<f64>>,
    pub timeout_ms: Option<u64>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_stream(
        &self,
        app: &AppHandle,
        session_id: &str,
        messages: Vec<LlmChatMessage>,
        tools: Option<Vec<Value>>,
        cancel: CancellationToken,
    ) -> Result<ChatStreamResult, String>;

    async fn chat_completion(
        &self,
        messages: &[LlmChatMessage],
        overrides: Option<&CompletionOverrides>,
    ) -> Result<String, String>;

    #[allow(dead_code)]
    async fn list_models(&self) -> Result<Vec<String>, String>;

    fn convert_tools(&self, manifests: &[Value]) -> Vec<Value>;

    fn build_tool_result_message(
        &self,
        call_id: &str,
        tool_name: &str,
        content: &str,
    ) -> LlmChatMessage;

    #[allow(dead_code)]
    fn provider_name(&self) -> &'static str;
}

pub fn resolve_model_name(last_used: Option<&str>, default_model: &str) -> Option<String> {
    last_used
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let d = default_model.trim();
            if d.is_empty() {
                None
            } else {
                Some(d.to_string())
            }
        })
}

pub fn create_provider(
    config: &AiConfig,
    model_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, String> {
    match config.active_provider {
        ActiveProvider::Ollama => {
            let model = model_override
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    resolve_model_name(
                        config.ollama.last_used_model.as_deref(),
                        &config.ollama.default_model,
                    )
                })
                .ok_or("No model selected. Choose a model in settings.")?;
            Ok(Arc::new(super::provider_ollama::OllamaProvider::new(
                config.ollama.base_url.clone(),
                model,
                config.parameters.temperature,
                config.parameters.top_p,
                config.request.timeout_ms,
            )))
        }
        ActiveProvider::Openai => {
            let model = model_override
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    resolve_model_name(
                        config.openai_compatible.last_used_model.as_deref(),
                        &config.openai_compatible.default_model,
                    )
                })
                .ok_or("No OpenAI model selected. Choose a model in settings.")?;
            if config.openai_compatible.api_key.trim().is_empty() {
                return Err("OpenAI API key is required. Set it in settings.".to_string());
            }
            Ok(Arc::new(
                super::provider_openai::OpenAiCompatibleProvider::new(
                    config.openai_compatible.base_url.clone(),
                    config.openai_compatible.api_key.clone(),
                    model,
                    config.parameters.temperature,
                    config.parameters.top_p,
                    config.request.timeout_ms,
                    config.openai_compatible.organization_id.clone(),
                ),
            ))
        }
    }
}

pub fn create_cloud_provider(config: &AiConfig) -> Result<Arc<dyn LlmProvider>, String> {
    let model = resolve_model_name(
        config.openai_compatible.last_used_model.as_deref(),
        &config.openai_compatible.default_model,
    )
    .ok_or("No OpenAI model configured for cloud planning.")?;
    if config.openai_compatible.api_key.trim().is_empty() {
        return Err("OpenAI API key is required for tiered mode.".to_string());
    }
    Ok(Arc::new(
        super::provider_openai::OpenAiCompatibleProvider::new(
            config.openai_compatible.base_url.clone(),
            config.openai_compatible.api_key.clone(),
            model,
            config.parameters.temperature,
            config.parameters.top_p,
            config.request.timeout_ms,
            config.openai_compatible.organization_id.clone(),
        ),
    ))
}

pub fn create_local_provider(config: &AiConfig) -> Result<Arc<dyn LlmProvider>, String> {
    let model = resolve_model_name(
        config.ollama.last_used_model.as_deref(),
        &config.ollama.default_model,
    )
    .ok_or("No Ollama model configured for local generation.")?;
    Ok(Arc::new(super::provider_ollama::OllamaProvider::new(
        config.ollama.base_url.clone(),
        model,
        config.parameters.temperature,
        config.parameters.top_p,
        config.request.timeout_ms,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_last_used() {
        assert_eq!(
            resolve_model_name(Some("llama3"), "default"),
            Some("llama3".to_string())
        );
    }

    #[test]
    fn resolve_falls_back_to_default() {
        assert_eq!(
            resolve_model_name(None, "default"),
            Some("default".to_string())
        );
    }

    #[test]
    fn resolve_skips_empty_last_used() {
        assert_eq!(
            resolve_model_name(Some("  "), "default"),
            Some("default".to_string())
        );
    }

    #[test]
    fn resolve_returns_none_when_both_empty() {
        assert_eq!(resolve_model_name(None, ""), None);
        assert_eq!(resolve_model_name(Some(""), "  "), None);
    }
}
