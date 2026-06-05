//! Iter 5 #4 (Stage 1): bridge a Skill into the tool surface so the main agent
//! loop can auto-invoke it via `skill.<id>`.
//!
//! On invoke:
//! 1. Bail with PermissionDenied when nesting_depth >= 1 (skills can't nest).
//! 2. Acquire a shared semaphore permit (concurrency cap = 3).
//! 3. Generate a fresh session_id, register a child cancel token.
//! 4. Emit `llm:skill-spawn` so the UI renders a separate skill bubble.
//! 5. Run the skill (await `runtime::run_skill`); chunks stream on the new sid.
//! 6. Return a hardcoded summary so the parent LLM acknowledges without
//!    re-rendering the skill's text (per design directive).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::llm::approval::ToolApprovalState;
use crate::llm::LlmSessionState;
use crate::semantic_index;
use crate::tools::context::ToolContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolError, ToolErrorCode, ToolManifest, ToolMetrics,
    ToolResult,
};
use crate::vault_config::{self, ActiveProvider};
use crate::WorkspaceState;

use super::registry::SkillRegistry;
use super::runtime;
use super::types::SkillManifest;

/// Maximum simultaneous auto-invoked skills across the whole app.
pub const SKILL_CONCURRENCY: usize = 3;

/// Maximum tool nesting depth. Stage 1 caps this at 1 (skills cannot
/// invoke other skills).
const MAX_NESTING_DEPTH: u8 = 1;

pub struct SkillAsTool {
    manifest: ToolManifest,
    skill_id: String,
    skill_name: String,
    app: AppHandle,
    semaphore: Arc<Semaphore>,
}

impl SkillAsTool {
    pub fn new(
        skill: &SkillManifest,
        app: AppHandle,
        semaphore: Arc<Semaphore>,
    ) -> Arc<Self> {
        let tool_name = format!("skill.{}", skill.id);
        let when = skill
            .when_to_use
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        let description = if when.is_empty() {
            skill.description.clone()
        } else {
            format!("{}\n\nWhen to use: {}", skill.description, when)
        };
        let input_schema = json!({
            "type": "object",
            "properties": {
                "input": {
                    "type": "string",
                    "description": "Forward the user's request (or a focused excerpt) to this skill."
                }
            },
            "required": ["input"],
            "additionalProperties": false
        });
        let manifest = ToolManifest {
            name: tool_name,
            version: skill.version.clone(),
            protocol_version: "1.0".to_string(),
            description,
            input_schema,
            output_schema: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string" },
                    "skill_id": { "type": "string" }
                }
            }),
            // Stage 1: model the wrapper as Llm-only — read/write effects are
            // gated by the underlying tools the skill's allow-list invokes.
            effects: vec![Effect::Llm],
            risk: Risk::Caution,
            privacy_aware: false,
            requires_workspace: true,
            default_approval: ApprovalPolicy::Auto,
            examples: vec![],
            tags: vec!["skill".to_string()],
            deprecated: None,
        };
        Arc::new(Self {
            manifest,
            skill_id: skill.id.clone(),
            skill_name: skill.name.clone(),
            app,
            semaphore,
        })
    }
}

#[async_trait]
impl Tool for SkillAsTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        if ctx.nesting_depth >= MAX_NESTING_DEPTH {
            return tool_err(
                ToolErrorCode::PermissionDenied,
                "skills cannot invoke other skills (nesting depth exceeded)",
            );
        }

        let user_input = match input
            .get("input")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => s.to_string(),
            None => {
                return tool_err(
                    ToolErrorCode::InvalidInput,
                    "missing required string field `input`",
                );
            }
        };

        let permit = match self.semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => {
                return tool_err(
                    ToolErrorCode::Internal,
                    "skill concurrency semaphore was closed",
                );
            }
        };

        let skill_registry = self.app.state::<Arc<SkillRegistry>>().inner().clone();
        let manifest = match skill_registry.get(&self.skill_id) {
            Some(m) => m,
            None => {
                drop(permit);
                return tool_err(
                    ToolErrorCode::NotFound,
                    &format!("skill not registered: {}", self.skill_id),
                );
            }
        };

        let workspace_state = self.app.state::<WorkspaceState>();
        let workspace_root = match crate::lock_workspace_root(&workspace_state) {
            Ok(p) => p,
            Err(e) => {
                drop(permit);
                return tool_err(ToolErrorCode::WorkspaceNotOpen, &e);
            }
        };

        let workspace_name = workspace_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let root_for_config = workspace_root.clone();
        let ai = match tauri::async_runtime::spawn_blocking(move || {
            vault_config::load_ai_config_internal(&root_for_config)
        })
        .await
        {
            Ok(Ok(ai)) => ai,
            Ok(Err(e)) => {
                drop(permit);
                return tool_err(ToolErrorCode::Internal, &format!("ai config: {e}"));
            }
            Err(e) => {
                drop(permit);
                return tool_err(ToolErrorCode::Internal, &format!("config join: {e}"));
            }
        };
        if ai.active_provider == ActiveProvider::Openai {
            drop(permit);
            return tool_err(
                ToolErrorCode::Internal,
                "OpenAI provider is not implemented yet.",
            );
        }
        let model = ai
            .ollama
            .last_used_model
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                Some(ai.ollama.default_model.clone()).filter(|s| !s.trim().is_empty())
            });
        let model = match model {
            Some(m) => m,
            None => {
                drop(permit);
                return tool_err(
                    ToolErrorCode::Internal,
                    "No model selected. Choose a model in settings.",
                );
            }
        };

        let session_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let sessions = self.app.state::<Arc<LlmSessionState>>().inner().clone();
        sessions.register(session_id.clone(), cancel.clone());

        let _ = self.app.emit(
            "llm:skill-spawn",
            json!({
                "sessionId": session_id,
                "conversationId": ctx.conversation_id,
                "skillId": self.skill_id,
                "skillName": self.skill_name,
                "parentToolCallId": ctx.call_id,
            }),
        );

        let tool_registry = self.app.state::<Arc<ToolRegistry>>().inner().clone();
        let ctx_factory = self
            .app
            .state::<Arc<crate::tools::context::ToolContextFactory>>()
            .inner()
            .clone();
        let approval_state = self
            .app
            .state::<Arc<ToolApprovalState>>()
            .inner()
            .clone();

        let cache = semantic_index::default_model_cache_dir();
        let bundle = semantic_index::resolve_bundle_model_dir(&self.app);

        let summary = runtime::run_skill_with_depth(
            self.app.clone(),
            session_id.clone(),
            manifest.clone(),
            ctx.conversation_id.clone(),
            workspace_root,
            workspace_name,
            user_input,
            tool_registry,
            ctx_factory,
            approval_state,
            Some(cache),
            Some(bundle),
            ai.ollama.base_url.clone(),
            model,
            ai.parameters.temperature,
            ai.parameters.top_p,
            cancel,
            ctx.nesting_depth.saturating_add(1),
        )
        .await;

        sessions.remove_session(&session_id);
        drop(permit);

        // Iter 5 followup #1A: surface the skill's final assistant text so the parent LLM
        // can reference its recommendations (e.g. quote a wikilink the skill suggested).
        // Empty summary (cancel / error / hit limits) falls back to the original ack note —
        // avoids feeding the parent partial/misleading state.
        let trimmed = summary.trim();
        let data = if trimmed.is_empty() {
            json!({
                "status": "completed",
                "skill_id": self.skill_id,
                "note": "Skill output streamed to user. Acknowledge briefly without repeating its content.",
            })
        } else {
            json!({
                "status": "completed",
                "skill_id": self.skill_id,
                "summary": trimmed,
                "note": "Skill output was streamed to the user separately. The 'summary' field above is the skill's full reply — you may reference its recommendations when continuing, but do not repeat it verbatim.",
            })
        };

        ToolResult::Ok {
            data,
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics::default(),
        }
    }
}

fn tool_err(code: ToolErrorCode, message: &str) -> ToolResult {
    ToolResult::Err {
        error: ToolError {
            code,
            message: message.to_string(),
            retryable: false,
            cause: None,
        },
    }
}

/// 注册单个 Skill 的 Tool 包装到 ToolRegistry（用于动态添加）
pub fn register_single_skill_tool(
    app: &AppHandle,
    manifest: &SkillManifest,
    tool_registry: &ToolRegistry,
    semaphore: Arc<Semaphore>,
) -> Result<(), String> {
    if !manifest.auto_invocable {
        return Ok(());
    }
    let tool = SkillAsTool::new(manifest, app.clone(), semaphore);
    tool_registry.register(tool).map_err(|e| format!("{}", e))
}

/// 注销单个 Skill 的 Tool 包装
pub fn unregister_skill_tool(
    skill_id: &str,
    tool_registry: &ToolRegistry,
) -> Result<(), String> {
    let tool_name = format!("skill.{}", skill_id);
    tool_registry.unregister(&tool_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::SkillUiEntry;

    fn skill(id: &str, when: Option<&str>) -> SkillManifest {
        SkillManifest {
            id: id.to_string(),
            name: format!("{id}-name"),
            version: "0.1.0".to_string(),
            description: "desc".to_string(),
            system_prompt_template: "p".to_string(),
            allowed_tools: vec!["time.now".to_string()],
            max_tool_calls: 4,
            timeout_secs: 30,
            ui_entry: SkillUiEntry::ConversationMode,
            tags: vec![],
            auto_invocable: true,
            when_to_use: when.map(str::to_string),
            max_tool_result_chars: 8000,
        }
    }

    #[test]
    fn manifest_uses_skill_dot_id_name() {
        // We cannot construct an AppHandle outside a Tauri runtime; build the
        // manifest portion via a helper so we can still assert its shape.
        let s = skill("writing_coach", Some("打磨笔记"));
        let when = s.when_to_use.as_deref().unwrap_or("");
        let description = if when.is_empty() {
            s.description.clone()
        } else {
            format!("{}\n\nWhen to use: {}", s.description, when)
        };
        assert!(description.contains("打磨笔记"));
        // skill.<id> conforms to is_valid_tool_name (a–z + . + a–z0–9_).
        let candidate = format!("skill.{}", s.id);
        assert_eq!(candidate, "skill.writing_coach");
    }
}
