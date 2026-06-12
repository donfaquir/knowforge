//! Iter 4 Tauri commands: `list_skills` returns all registered SkillManifests;
//! `invoke_skill` launches a Skill sub-turn through the agent_loop and streams
//! results back through the existing `llm:stream-*` / `llm:tool-*` events.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio_util::sync::CancellationToken;

use crate::llm::approval::ToolApprovalState;
use crate::llm::create_provider;
use crate::llm::LlmSessionState;
use crate::lock_workspace_root;
use crate::semantic_index;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::ToolRegistry;
use crate::vault_config;
use crate::WorkspaceState;

use super::registry::SkillRegistry;
use super::runtime;
use super::types::SkillManifest;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListItem {
    #[serde(flatten)]
    pub manifest: SkillManifest,
    pub is_builtin: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSkillsResponse {
    pub skills: Vec<SkillListItem>,
}

#[tauri::command]
pub fn list_skills(registry: State<'_, Arc<SkillRegistry>>) -> ListSkillsResponse {
    ListSkillsResponse {
        skills: registry
            .list()
            .into_iter()
            .map(|m| {
                let is_builtin = registry.is_builtin(&m.id);
                SkillListItem {
                    manifest: m,
                    is_builtin,
                }
            })
            .collect(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSkillArgs {
    pub manifest: SkillManifest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkillArgs {
    pub manifest: SkillManifest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSkillArgs {
    pub skill_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillLoadFailure {
    pub file: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadSkillsResponse {
    pub loaded: Vec<String>,
    pub failed: Vec<SkillLoadFailure>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
}

#[tauri::command]
pub async fn create_custom_skill(
    app: AppHandle,
    workspace: State<'_, WorkspaceState>,
    skills: State<'_, Arc<SkillRegistry>>,
    tools: State<'_, Arc<ToolRegistry>>,
    semaphore: State<'_, Arc<tokio::sync::Semaphore>>,
    args: CreateSkillArgs,
) -> Result<(), String> {
    let manifest = args.manifest;

    let root = workspace
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no workspace open".to_string())?;

    let skills_dir = root.join(".knowforge").join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| format!("create dir: {}", e))?;

    skills
        .register(manifest.clone(), &tools)
        .map_err(|e| format!("{}", e))?;

    let file_path = skills_dir.join(format!("{}.md", manifest.id));
    let content = crate::skills::loader::serialize_skill_markdown(&manifest);
    std::fs::write(&file_path, content).map_err(|e| format!("write file: {}", e))?;

    crate::skills::skill_tool::register_single_skill_tool(
        &app,
        &manifest,
        &tools,
        Arc::clone(&semaphore),
    )?;

    Ok(())
}

#[tauri::command]
pub async fn update_custom_skill(
    app: AppHandle,
    workspace: State<'_, WorkspaceState>,
    skills: State<'_, Arc<SkillRegistry>>,
    tools: State<'_, Arc<ToolRegistry>>,
    semaphore: State<'_, Arc<tokio::sync::Semaphore>>,
    args: UpdateSkillArgs,
) -> Result<(), String> {
    let manifest = args.manifest;

    let root = workspace
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no workspace open".to_string())?;

    let _ = crate::skills::skill_tool::unregister_skill_tool(&manifest.id, &tools);

    skills
        .update(manifest.clone(), &tools)
        .map_err(|e| format!("{}", e))?;

    let skills_dir = root.join(".knowforge").join("skills");
    let file_path = skills_dir.join(format!("{}.md", manifest.id));
    let content = crate::skills::loader::serialize_skill_markdown(&manifest);
    std::fs::write(&file_path, content).map_err(|e| format!("write file: {}", e))?;

    crate::skills::skill_tool::register_single_skill_tool(
        &app,
        &manifest,
        &tools,
        Arc::clone(&semaphore),
    )?;

    Ok(())
}

#[tauri::command]
pub async fn delete_custom_skill(
    workspace: State<'_, WorkspaceState>,
    skills: State<'_, Arc<SkillRegistry>>,
    tools: State<'_, Arc<ToolRegistry>>,
    args: DeleteSkillArgs,
) -> Result<(), String> {
    let _ = crate::skills::skill_tool::unregister_skill_tool(&args.skill_id, &tools);

    skills
        .unregister(&args.skill_id)
        .map_err(|e| format!("{}", e))?;

    let root = workspace
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no workspace open".to_string())?;
    let file_path = root
        .join(".knowforge")
        .join("skills")
        .join(format!("{}.md", args.skill_id));
    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| format!("delete file: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn reload_custom_skills(
    app: AppHandle,
    workspace: State<'_, WorkspaceState>,
    skills: State<'_, Arc<SkillRegistry>>,
    tools: State<'_, Arc<ToolRegistry>>,
    semaphore: State<'_, Arc<tokio::sync::Semaphore>>,
) -> Result<ReloadSkillsResponse, String> {
    let root = workspace
        .root
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "no workspace open".to_string())?;

    let current_skills = skills.list();
    for skill in &current_skills {
        if !skills.is_builtin(&skill.id) {
            let _ = crate::skills::skill_tool::unregister_skill_tool(&skill.id, &tools);
            let _ = skills.unregister(&skill.id);
        }
    }

    let skills_dir = root.join(".knowforge").join("skills");
    let results = crate::skills::load_custom_skills(&skills_dir, &skills, &tools);

    let mut loaded = vec![];
    let mut failed = vec![];

    for r in results {
        match r {
            crate::skills::SkillLoadResult::Loaded(id) => {
                if let Some(manifest) = skills.get(&id) {
                    let _ = crate::skills::skill_tool::register_single_skill_tool(
                        &app,
                        &manifest,
                        &tools,
                        Arc::clone(&semaphore),
                    );
                }
                loaded.push(id);
            }
            crate::skills::SkillLoadResult::Failed { file, error } => {
                failed.push(SkillLoadFailure { file, error });
            }
        }
    }

    Ok(ReloadSkillsResponse { loaded, failed })
}

#[tauri::command]
pub fn list_available_tools(
    registry: State<'_, Arc<ToolRegistry>>,
) -> Vec<ToolSummary> {
    registry
        .list_for_llm(crate::tools::registry::ToolScope::Global)
        .into_iter()
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?.to_string();
            let description = v
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with("skill.") {
                return None;
            }
            Some(ToolSummary { name, description })
        })
        .collect()
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

    let model_override = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let provider = create_provider(&ai, model_override)?;

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
            provider,
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
