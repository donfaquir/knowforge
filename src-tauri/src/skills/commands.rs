//! Iter 4 Tauri commands: `list_skills` returns all registered SkillManifests;
//! `invoke_skill` launches a Skill sub-turn through the agent_loop and streams
//! results back through the existing `llm:stream-*` / `llm:tool-*` events.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio_util::sync::CancellationToken;

use crate::llm::approval::ToolApprovalState;
use crate::llm::LlmSessionState;
use crate::lock_workspace_root;
use crate::semantic_index;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;
use crate::vault_config::{self, ActiveProvider};
use crate::WorkspaceState;

use super::registry::SkillRegistry;
use super::runtime;
use super::types::SkillManifest;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillManifest>,
}

#[tauri::command]
pub fn list_skills(registry: State<'_, Arc<SkillRegistry>>) -> ListSkillsResponse {
    ListSkillsResponse {
        skills: registry.list(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeSkillArgs {
    pub skill_id: String,
    pub input: String,
    /// Parent conversation id used to scope approval cache and audit entries.
    /// When absent, the spawned session_id is used as the scope.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Optional model override; falls back to ai config's last_used / default model.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeSkillResponse {
    pub session_id: String,
    pub skill_id: String,
}

#[tauri::command]
pub async fn invoke_skill(
    app: AppHandle,
    workspace: State<'_, WorkspaceState>,
    sessions: State<'_, Arc<LlmSessionState>>,
    tool_registry: State<'_, Arc<ToolRegistry>>,
    ctx_factory: State<'_, Arc<ToolContextFactory>>,
    approval: State<'_, Arc<ToolApprovalState>>,
    skills: State<'_, Arc<SkillRegistry>>,
    args: InvokeSkillArgs,
) -> Result<InvokeSkillResponse, String> {
    let manifest = skills
        .get(&args.skill_id)
        .ok_or_else(|| format!("skill not found: {}", args.skill_id))?;

    if args.input.trim().is_empty() {
        return Err("skill input must not be empty".to_string());
    }

    let root = lock_workspace_root(&workspace)?;
    let root_for_config = root.clone();
    let ai = tauri::async_runtime::spawn_blocking(move || {
        vault_config::load_ai_config_internal(&root_for_config)
    })
    .await
    .map_err(|e| e.to_string())??;

    if ai.active_provider == ActiveProvider::Openai {
        return Err("OpenAI provider is not implemented yet.".to_string());
    }

    let model = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            ai.ollama
                .last_used_model
                .clone()
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| Some(ai.ollama.default_model.clone()).filter(|s| !s.trim().is_empty()))
        .ok_or_else(|| "No model selected. Choose a model in settings.".to_string())?;

    let workspace_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let session_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    sessions.register(session_id.clone(), cancel.clone());

    let parent_conv_id = args
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session_id.clone());

    let cache = semantic_index::default_model_cache_dir();
    let bundle = semantic_index::resolve_bundle_model_dir(&app);

    let app_h = app.clone();
    let sid = session_id.clone();
    let sessions_arc = Arc::clone(sessions.inner());
    let tool_registry_arc = Arc::clone(tool_registry.inner());
    let ctx_factory_arc = Arc::clone(ctx_factory.inner());
    let approval_arc = Arc::clone(approval.inner());
    let manifest_for_task = manifest.clone();
    let workspace_root = root.clone();
    let base = ai.ollama.base_url.clone();
    let temp = ai.parameters.temperature;
    let top_p = ai.parameters.top_p;
    let input = args.input.clone();

    tokio::spawn(async move {
        runtime::run_skill(
            app_h,
            sid.clone(),
            manifest_for_task,
            parent_conv_id,
            workspace_root,
            workspace_name,
            input,
            tool_registry_arc,
            ctx_factory_arc,
            approval_arc,
            Some(cache),
            Some(bundle),
            base,
            model,
            temp,
            top_p,
            cancel,
        )
        .await;
        sessions_arc.remove_session(&sid);
    });

    Ok(InvokeSkillResponse {
        session_id,
        skill_id: args.skill_id,
    })
}
