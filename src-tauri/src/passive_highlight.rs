//! 被动高亮：对用户消息做旁路价值检测（Ollama 非流式 JSON）

use crate::llm::ollama;
use crate::llm::LlmChatMessage;
use crate::lock_workspace_root;
use crate::vault_config::{self, ActiveProvider};
use serde::{Deserialize, Serialize};
use tauri::State;

const SYSTEM_PASSIVE: &str = r#"You analyze a single user message from a knowledge-work app. Decide whether it expresses a valuable thought worth saving as a structured note: integrating ideas, correcting a prior misunderstanding, or making a non-trivial cross-domain connection.

Respond with ONE JSON object only (no markdown fences, no prose). Use exactly these camelCase keys:
- "detected": boolean
- "kind": one of "integrate" | "correct" | "cross_domain"
- "confidence": number from 0 to 1 (how sure you are)
- "summary": short plain-text summary suitable as note seed (1-3 sentences); MUST use the same language and script as the user message — do not translate to English if the user wrote in another language
- "useRawFallback": boolean — set true only if a good summary cannot be produced but the message should still be offered for raw capture

If nothing qualifies, return {"detected":false,"kind":"integrate","confidence":0,"summary":"","useRawFallback":false}."#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetectJson {
    #[serde(default)]
    detected: bool,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    use_raw_fallback: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectPassiveHighlightResponse {
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_raw_fallback: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectPassiveHighlightArgs {
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncrementPassiveHighlightInaccuracyArgs {
    pub kind: String,
}

fn effective_min_for_kind(cog: &vault_config::CognitiveConfig, kind: &str) -> f64 {
    let base = cog.passive_highlight_confidence_min;
    let n = match kind {
        "integrate" => cog.passive_highlight_inaccuracy_counts.integrate,
        "correct" => cog.passive_highlight_inaccuracy_counts.correct,
        "cross_domain" => cog.passive_highlight_inaccuracy_counts.cross_domain,
        _ => 0,
    };
    let bump = (n.min(5) as f64) * 0.03;
    (base + bump).min(1.0)
}

fn extract_detect_json(raw: &str) -> Result<DetectJson, String> {
    let s = raw.trim();
    let start = s
        .find('{')
        .ok_or_else(|| "model output did not contain JSON object".to_string())?;
    let end = s
        .rfind('}')
        .ok_or_else(|| "model output did not contain JSON object".to_string())?;
    if end < start {
        return Err("invalid JSON slice".to_string());
    }
    serde_json::from_str::<DetectJson>(&s[start..=end])
        .map_err(|e| format!("failed to parse detection JSON: {e}"))
}

fn normalize_kind(k: &str) -> Option<&'static str> {
    match k.trim() {
        "integrate" => Some("integrate"),
        "correct" => Some("correct"),
        "cross_domain" => Some("cross_domain"),
        _ => None,
    }
}

#[tauri::command]
pub async fn detect_passive_highlight(
    workspace: State<'_, crate::WorkspaceState>,
    args: DetectPassiveHighlightArgs,
) -> Result<DetectPassiveHighlightResponse, String> {
    let root = lock_workspace_root(&workspace)?;
    let root_for_cfg = root.clone();
    let (ai, cog) = tauri::async_runtime::spawn_blocking(move || {
        let ai = vault_config::load_ai_config_internal(&root_for_cfg)?;
        let cog = vault_config::load_cognitive_merged(&root_for_cfg)?;
        Ok::<_, String>((ai, cog))
    })
    .await
    .map_err(|e| e.to_string())??;

    let empty = || DetectPassiveHighlightResponse {
        detected: false,
        kind: None,
        confidence: None,
        summary: None,
        use_raw_fallback: None,
    };

    if !cog.passive_highlight_enabled {
        return Ok(empty());
    }
    if ai.active_provider != ActiveProvider::Ollama {
        return Ok(empty());
    }

    let trimmed = args.text.trim();
    if trimmed.chars().count() < 20 {
        return Ok(empty());
    }

    let model = ai
        .ollama
        .last_used_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let d = ai.ollama.default_model.trim();
            if d.is_empty() {
                None
            } else {
                Some(ai.ollama.default_model.clone())
            }
        })
        .ok_or_else(|| "No model selected. Choose a model in settings.".to_string())?;

    let timeout_ms = ai.request.timeout_ms;
    let user_content = format!("Analyze the following user message.\n\n---\n{trimmed}\n---");
    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_PASSIVE.to_string(),
        },
        LlmChatMessage {
            role: "user".into(),
            content: user_content,
        },
    ];

    let raw = match ollama::run_chat_completion(
        &ai.ollama.base_url,
        &model,
        &msgs,
        ai.parameters.temperature,
        ai.parameters.top_p,
        timeout_ms,
    )
    .await
    {
        Ok(s) => s,
        Err(_) => return Ok(empty()),
    };

    let parsed = match extract_detect_json(&raw) {
        Ok(v) => v,
        Err(_) => return Ok(empty()),
    };

    if !parsed.detected {
        return Ok(empty());
    }

    let kind_raw = parsed.kind.as_deref().unwrap_or("integrate");
    let Some(kind_norm) = normalize_kind(kind_raw) else {
        return Ok(empty());
    };

    let confidence = parsed.confidence.unwrap_or(0.0).clamp(0.0, 1.0);
    let min_eff = effective_min_for_kind(&cog, kind_norm);
    if confidence < min_eff {
        return Ok(empty());
    }

    let mut summary = parsed.summary.unwrap_or_default();
    let summary_trim = summary.trim();
    let use_fallback = parsed.use_raw_fallback.unwrap_or(false) || summary_trim.is_empty();
    if use_fallback {
        summary = trimmed.to_string();
    }
    if summary.len() > 4000 {
        summary.truncate(4000);
    }

    Ok(DetectPassiveHighlightResponse {
        detected: true,
        kind: Some(kind_norm.to_string()),
        confidence: Some(confidence),
        summary: Some(summary),
        use_raw_fallback: Some(use_fallback),
    })
}

#[tauri::command]
pub async fn increment_passive_highlight_inaccuracy(
    workspace: State<'_, crate::WorkspaceState>,
    args: IncrementPassiveHighlightInaccuracyArgs,
) -> Result<(), String> {
    let root = lock_workspace_root(&workspace)?;
    let kind = args.kind;
    tauri::async_runtime::spawn_blocking(move || vault_config::bump_passive_highlight_inaccuracy(&root, &kind))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_min_increases_with_inaccuracy() {
        let mut cog = vault_config::CognitiveConfig::default();
        cog.passive_highlight_confidence_min = 0.5;
        assert!((effective_min_for_kind(&cog, "integrate") - 0.5).abs() < 1e-6);
        cog.passive_highlight_inaccuracy_counts.integrate = 2;
        assert!((effective_min_for_kind(&cog, "integrate") - 0.56).abs() < 1e-6);
        cog.passive_highlight_inaccuracy_counts.integrate = 10;
        assert!((effective_min_for_kind(&cog, "integrate") - 0.65).abs() < 1e-6);
    }

    #[test]
    fn extract_detect_json_parses_object() {
        let raw = r#"Here is {"detected":true,"kind":"correct","confidence":0.8,"summary":"x","useRawFallback":false} end"#;
        let j = extract_detect_json(raw).unwrap();
        assert!(j.detected);
        assert_eq!(j.kind.as_deref(), Some("correct"));
    }
}
