//! Iter 4 Skill runtime: launches an **isolated** agent_loop sub-turn with a Skill's
//! system prompt + filtered tool whitelist + custom limits. The sub-turn does not
//! read or write the parent conversation's message history.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

use crate::llm::agent_loop::{self, AgentLoopConfig};
use crate::llm::approval::ToolApprovalState;
use crate::llm::provider::LlmProvider;
use crate::llm::LlmChatMessage;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::{ToolRegistry, ToolScope};

use super::types::SkillManifest;

const SKILL_LANGUAGE_MATCH_SYSTEM: &str = "IMPORTANT: Always respond in the same language the user writes in. \
    If the user writes in Chinese, respond entirely in Chinese. \
    If the user writes in English, respond in English. \
    Match the user's language exactly.";

/// Derive an audit/approval scope id for a skill sub-turn so that:
/// - Approvals granted inside the Skill don't leak into the parent conversation.
/// - Audit entries are clearly tagged.
pub fn skill_conversation_id(skill_id: &str, parent_conversation_id: &str) -> String {
    format!("skill:{}:{}", skill_id, parent_conversation_id)
}

/// Build the initial message stack injected into the agent loop for this Skill turn.
/// Always starts with the rendered system prompt; adds language match, the discover-
/// before-read hint (Iter 3.5 P0-2), and the user input.
pub fn build_initial_messages(
    manifest: &SkillManifest,
    workspace_name: &str,
    workspace_root: &str,
    user_input: &str,
) -> Vec<LlmChatMessage> {
    let prompt = manifest.render_system_prompt(workspace_name, workspace_root);
    vec![
        LlmChatMessage {
            role: "system".to_string(),
            content: prompt,
            ..Default::default()
        },
        LlmChatMessage {
            role: "system".to_string(),
            content: SKILL_LANGUAGE_MATCH_SYSTEM.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "system".to_string(),
            content: agent_loop::TOOL_USE_DISCOVERY_HINT.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".to_string(),
            content: user_input.to_string(),
            ..Default::default()
        },
    ]
}

/// Filter the registry's LLM-facing manifest list down to the Skill's allowed_tools.
/// Unknown names are silently skipped here (validated at registration time, but a tool
/// might be deprecated/removed between registration and invocation).
pub fn filter_tools_for_skill(registry: &ToolRegistry, manifest: &SkillManifest) -> Vec<Value> {
    let allowed: std::collections::HashSet<&str> =
        manifest.allowed_tools.iter().map(String::as_str).collect();
    registry
        .list_for_llm(ToolScope::Global)
        .into_iter()
        .filter(|v| {
            v.get("name")
                .and_then(|n| n.as_str())
                .map(|n| allowed.contains(n))
                .unwrap_or(false)
        })
        .collect()
}

/// Spawn the agent loop for this Skill turn. Returns once the agent loop completes;
/// `llm:stream-chunk` / `llm:tool-*` / `llm:agent-done` events stream out under `session_id`.
///
/// Returns the skill's final assistant text (empty when the run was cancelled / errored /
/// hit limits). Forwarded to `SkillAsTool` for parent-LLM tool-result summary.
#[allow(clippy::too_many_arguments)]
pub async fn run_skill(
    app: AppHandle,
    session_id: String,
    manifest: Arc<SkillManifest>,
    parent_conversation_id: String,
    workspace_root: PathBuf,
    workspace_name: String,
    user_input: String,
    registry: Arc<ToolRegistry>,
    ctx_factory: Arc<ToolContextFactory>,
    approval_state: Arc<ToolApprovalState>,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    provider: Arc<dyn LlmProvider>,
    cancel: CancellationToken,
    max_context_tokens: Option<u64>,
) -> String {
    run_skill_with_depth(
        app,
        session_id,
        manifest,
        parent_conversation_id,
        workspace_root,
        workspace_name,
        user_input,
        registry,
        ctx_factory,
        approval_state,
        app_cache_dir,
        app_bundle_resource_dir,
        provider,
        cancel,
        1,
        max_context_tokens,
    )
    .await
}

/// Iter 5 #4: same as [`run_skill`] but lets the caller pin the nesting depth
/// stamped onto every ToolContext spawned inside the skill loop. Used by
/// SkillAsTool so the depth=1 cap holds even when the parent call originated
/// from the main agent loop at depth=0.
///
/// Returns the skill's final assistant text (forwarded from agent_loop).
#[allow(clippy::too_many_arguments)]
pub async fn run_skill_with_depth(
    app: AppHandle,
    session_id: String,
    manifest: Arc<SkillManifest>,
    parent_conversation_id: String,
    workspace_root: PathBuf,
    workspace_name: String,
    user_input: String,
    registry: Arc<ToolRegistry>,
    ctx_factory: Arc<ToolContextFactory>,
    approval_state: Arc<ToolApprovalState>,
    app_cache_dir: Option<PathBuf>,
    app_bundle_resource_dir: Option<PathBuf>,
    provider: Arc<dyn LlmProvider>,
    cancel: CancellationToken,
    nesting_depth: u8,
    max_context_tokens: Option<u64>,
) -> String {
    let workspace_root_str = workspace_root.to_string_lossy().to_string();
    let messages = build_initial_messages(
        &manifest,
        &workspace_name,
        &workspace_root_str,
        &user_input,
    );
    let tools_json = provider.convert_tools(&filter_tools_for_skill(&registry, &manifest));

    let config = AgentLoopConfig {
        max_tool_calls: manifest.max_tool_calls,
        timeout_ms: manifest.timeout_secs.saturating_mul(1000),
        max_single_result_chars: manifest.max_tool_result_chars as usize,
        nesting_depth,
        max_context_tokens,
        summarize_threshold: crate::llm::tool_result_processor::DEFAULT_SUMMARIZE_THRESHOLD,
    };

    let conv_id = skill_conversation_id(&manifest.id, &parent_conversation_id);

    agent_loop::run_agent_stream(
        app,
        session_id,
        messages,
        tools_json,
        registry,
        ctx_factory,
        workspace_root,
        app_cache_dir,
        app_bundle_resource_dir,
        provider,
        cancel,
        config,
        conv_id,
        approval_state,
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::SkillUiEntry;
    use crate::tools::register_builtin_tools;

    fn sample_manifest(allowed: Vec<&str>) -> SkillManifest {
        SkillManifest {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            version: "0.1.0".to_string(),
            description: "x".to_string(),
            system_prompt_template: "Hi {{workspace_name}}".to_string(),
            allowed_tools: allowed.into_iter().map(String::from).collect(),
            max_tool_calls: 3,
            timeout_secs: 25,
            ui_entry: SkillUiEntry::Standalone,
            tags: vec![],
            auto_invocable: false,
            when_to_use: None,
            max_tool_result_chars: 8000,
        }
    }

    #[test]
    fn builds_messages_with_system_user() {
        let m = sample_manifest(vec!["time.now"]);
        let msgs = build_initial_messages(&m, "vault-x", "/tmp/v", "ask me");
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("vault-x"));
        assert_eq!(msgs[1].role, "system");
        assert_eq!(msgs[2].role, "system");
        assert!(
            msgs[2].content.contains("note.list") && msgs[2].content.contains("vault.search_keyword"),
            "expected discover-before-read hint at msgs[2], got: {}",
            msgs[2].content,
        );
        assert_eq!(msgs[3].role, "user");
        assert_eq!(msgs[3].content, "ask me");
    }

    #[test]
    fn filters_tools_by_whitelist() {
        let r = ToolRegistry::new();
        register_builtin_tools(&r, None).unwrap();
        let m = sample_manifest(vec!["time.now"]);
        let filtered = filter_tools_for_skill(&r, &m);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].get("name").and_then(|n| n.as_str()), Some("time.now"));
    }

    #[test]
    fn skips_unknown_allowed_tools_at_filter_time() {
        let r = ToolRegistry::new();
        register_builtin_tools(&r, None).unwrap();
        let m = sample_manifest(vec!["time.now", "nonexistent.tool"]);
        let filtered = filter_tools_for_skill(&r, &m);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn skill_conversation_id_is_scoped() {
        let id = skill_conversation_id("writing_coach", "abc-123");
        assert_eq!(id, "skill:writing_coach:abc-123");
    }
}
