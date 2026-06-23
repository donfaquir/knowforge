use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use super::LlmChatMessage;
use crate::vault_config::{AiConfig, ProviderProfile};

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
    pub json_mode: bool,
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

    fn is_remote(&self) -> bool;
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

/// Create a provider from the active profile in the config.
pub fn create_provider(
    config: &AiConfig,
    model_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, String> {
    let profile = config
        .active_profile()
        .ok_or("No active provider configured. Choose a provider in settings.")?;
    create_provider_from_profile(profile, config, model_override)
}

/// Create a provider for a specific profile identified by `provider_id`.
pub fn create_provider_by_id(
    config: &AiConfig,
    provider_id: &str,
    model_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, String> {
    let profile = config
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found in config.", provider_id))?;
    create_provider_from_profile(profile, config, model_override)
}

fn create_provider_from_profile(
    profile: &ProviderProfile,
    config: &AiConfig,
    model_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, String> {
    let model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| resolve_model_name(profile.last_used_model.as_deref(), &profile.default_model))
        .ok_or("No model selected. Choose a model in settings.")?;

    if profile.is_remote && profile.api_key.trim().is_empty() {
        return Err(format!(
            "API key is required for provider '{}'. Set it in settings.",
            profile.label
        ));
    }

    Ok(Arc::new(
        super::provider_impl::UnifiedProvider::new(
            profile.base_url.clone(),
            profile.api_key.clone(),
            model,
            config.parameters.temperature,
            config.parameters.top_p,
            config.request.timeout_ms,
            profile.organization_id.clone(),
            profile.is_remote,
        ),
    ))
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
