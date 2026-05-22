//! Rust 侧 Ollama 代理：模型列表、流式 chat、会话中止（任务 04）。

pub(crate) mod ollama;
pub(crate) mod agent_loop;
pub mod approval;

use crate::depth_decisions;
use crate::lock_workspace_root;
use crate::note_privacy;
use crate::semantic_index;
use crate::skills::SkillRegistry;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::{ToolRegistry, ToolScope};
use crate::vault_config::{self, ActiveProvider, DepthMode};
use crate::vault_context_search::{self, VaultSnippetKind};
use std::path::PathBuf;
use chrono::Utc;
use ollama::run_chat_stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

// --- 会话：CancellationToken 多 clone 联动取消 ---

pub struct LlmSessionState {
    inner: Mutex<HashMap<String, CancellationToken>>,
}

impl Default for LlmSessionState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl LlmSessionState {
    pub(crate) fn register(&self, id: String, token: CancellationToken) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.insert(id, token);
    }

    fn take_cancel(&self, id: &str) -> Option<CancellationToken> {
        let mut g = self.inner.lock().ok()?;
        g.remove(id)
    }

    /// 流正常/异常结束后摘掉登记（幂等）
    pub fn remove_session(&self, id: &str) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.remove(id);
    }
}

// --- IPC 类型 ---

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LlmChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    /// 服务端生成的 UUID v7，用于追踪一次工具调用的生命周期。
    pub id: String,
    pub function: LlmToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCallFunction {
    pub name: String,
    /// Ollama 返回的已解析 JSON 参数对象。
    pub arguments: Value,
}

/// 附带当前笔记时由前端传入；**笔记正文出站裁决在 Rust**（任务 09）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContextIn {
    pub rel_path: String,
    pub markdown_for_gate: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultContextIn {
    pub snippets: Vec<vault_context_search::VaultSnippetRecord>,
}

/// 想法聚焦对话：由前端与会话持久化传入（迭代 6.1）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtFocusContextIn {
    pub thought_id: String,
    pub thought_body: String,
    pub maturity: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaChatStreamStartArgs {
    /// 仅 `user` / `assistant`；笔记 system 由本模块根据 `note_context` 与配置拼装。
    pub messages: Vec<LlmChatMessage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub note_context: Option<NoteContextIn>,
    /// 由 `search_workspace_context` 结果回传；出站前在 `assemble_ollama_messages` 内按磁盘重算摘录。
    #[serde(default)]
    pub vault_context: Option<VaultContextIn>,
    /// 当前深度模式（迭代 3）；影响系统提示的详细程度与风格。
    #[serde(default)]
    pub depth_mode: Option<DepthMode>,
    /// 深化子轮次的邀请上下文（可选），供 Phase 4 使用。
    #[serde(default)]
    pub invite_context: Option<InviteContextIn>,
    /// 会话绑定的「与某条想法深聊」上下文（迭代 6.1）
    #[serde(default)]
    pub thought_focus_context: Option<ThoughtFocusContextIn>,
    /// 是否在 system 中注入内置语义检索摘录；默认 true（迭代 6.2）
    #[serde(default)]
    pub semantic_context_enabled: Option<bool>,
    /// 是否启用 Tool Calling Loop（P2）；默认 false 走原文字流路径。
    #[serde(default)]
    pub tools_enabled: bool,
    /// 前端会话 ID（用于 ConfirmOncePerSession 审批缓存的作用域）；
    /// 缺省时退化为本次 stream 的 session_id（兼容旧客户端）。
    #[serde(default)]
    pub conversation_id: Option<String>,
}

/// 深化轮次的上下文（由邀请 UI 传入）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteContextIn {
    /// 用户接受的深化问题文本
    pub question: String,
    /// 旧理解摘录（如果有检索结果）
    #[serde(default)]
    pub thought_excerpt: Option<String>,
}

/// 本轮请求实际注入模型的上下文来源摘要（供 UI 在助手回复末尾展示引用）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReplyContextSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_note: Option<ReplyCurrentNoteSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_keyword: Option<ReplyVaultKeywordSource>,
    #[serde(default)]
    pub semantic: ReplySemanticSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_focus: Option<ReplyThoughtFocusSource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyCurrentNoteSource {
    pub rel_path: String,
    pub mode: ReplyCurrentNoteMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplyCurrentNoteMode {
    Full,
    Redacted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyVaultKeywordSource {
    pub entries: Vec<ReplyVaultKeywordEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyVaultKeywordEntry {
    pub rel_path: String,
    pub kind: VaultSnippetKind,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReplySemanticSource {
    #[serde(default)]
    pub injected: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thought_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyThoughtFocusSource {
    pub thought_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaChatStreamStartResponse {
    pub session_id: String,
    /// 仅当请求 `depthMode` 为 `auto` 时返回：本次启发式解析出的具体档位。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_depth: Option<DepthMode>,
    pub reply_context_sources: ReplyContextSources,
}

/// Auto 档：按最近一条用户消息长度做轻量启发式（与 `depth_decisions` 日志 reason 对齐）。
fn resolve_auto_depth_heuristic(query: &str) -> (DepthMode, String) {
    let t = query.trim();
    let n = t.chars().count();
    if n <= 40 {
        return (DepthMode::Shallow, "short_query".to_string());
    }
    if n <= 200 {
        return (DepthMode::Medium, "medium_query".to_string());
    }
    (DepthMode::Deep, "long_query".to_string())
}

struct AssembleOutcome {
    messages: Vec<LlmChatMessage>,
    /// `Some` 仅当本次请求显式为 Auto：解析结果供前端展示与决策日志。
    resolved_depth: Option<DepthMode>,
    auto_resolve_reason: Option<String>,
    reply_context_sources: ReplyContextSources,
}

// --- 事件载荷（与前端 listen 对齐） ---

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmStreamChunkPayload {
    session_id: String,
    delta: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmStreamDonePayload {
    session_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LlmStreamErrorPayload {
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    message: String,
}

pub(super) fn emit_chunk(app: &AppHandle, session_id: &str, delta: &str) {
    let payload = LlmStreamChunkPayload {
        session_id: session_id.to_string(),
        delta: delta.to_string(),
    };
    let _ = app.emit("llm:stream-chunk", payload);
}

pub(super) fn emit_done(app: &AppHandle, session_id: &str) -> Result<(), tauri::Error> {
    app.emit(
        "llm:stream-done",
        LlmStreamDonePayload {
            session_id: session_id.to_string(),
        },
    )
}

/// 全局文风：抑制叠字、同义反复与无信息增量的重复（与 assemble 末尾 system 一致）
const CHAT_ANTI_REPETITION_SYSTEM: &str = "FORM / STYLE: Write economical, fluent prose. \
Avoid redundant repetition: no stuttering (duplicated syllables, characters, or words without purpose), \
no echoing the same phrase or idea in different words unless each adds clarity, \
and no filler stacks (e.g. piling similar intensifiers). Each sentence should add new information. \
In Chinese, avoid gratuitous reduplication (e.g. 叠词堆砌) and needless near-duplicate clauses.";

fn build_note_system_content(rel_path: &str, markdown: &str) -> String {
    format!(
        "The user is editing the following Markdown file `{rel_path}`. Use it as the primary source when answering questions about its contents.\n\
When citing this file, repeat its path exactly once (do not garble or repeat path segments).\n\n\
```markdown\n{markdown}\n```"
    )
}

fn build_thought_focus_system_block(tf: &ThoughtFocusContextIn, has_open_note: bool) -> String {
    let mut s = String::from(
        "The user is focusing on a specific saved thought (from their knowledge vault) for deeper discussion.\n",
    );
    s.push_str(&format!("- thought_id: {}\n", tf.thought_id.trim()));
    s.push_str(&format!("- maturity: {}\n", tf.maturity.trim()));
    s.push_str("Thought content:\n```\n");
    s.push_str(&tf.thought_body);
    s.push_str("\n```\n");
    if has_open_note {
        s.push_str(
            "They may also have a Markdown note open for reference. Unless they clearly switch the topic to that note, prioritize helping them explore and refine the focused thought above.\n",
        );
    }
    s
}

/// 根据深度模式生成系统指令，控制回答篇幅和风格。
fn build_depth_system_instruction(depth: DepthMode) -> String {
    match depth {
        DepthMode::Shallow => {
            "Response depth: SHALLOW. Keep your answer brief and direct (2-4 sentences). \
             Do NOT reference previous thoughts from the user's notes. \
             Do NOT suggest deepening questions or invite further exploration. \
             Focus on answering the immediate question concisely. \
             Do not pad with repeated words or restated points."
                .to_string()
        }
        DepthMode::Medium => {
            "Response depth: MEDIUM. Provide a clear, well-structured answer of moderate length. \
             You may reference relevant context if available. \
             Use a suggestive, non-judgmental tone. Avoid prescriptive language like \
             'you should' or 'you need to'. Prefer 'you might consider' or 'one approach is'. \
             Avoid repeating the same claim in multiple sentences without adding detail."
                .to_string()
        }
        DepthMode::Deep => {
            "Response depth: DEEP. Provide a thorough, nuanced answer that explores the topic in depth. \
             Consider multiple perspectives. Reference related context from the user's notes when relevant. \
             Use a suggestive, non-judgmental tone throughout. Avoid prescriptive or lecturing language. \
             Never use phrases like 'you should', 'you need to', 'it's important that you'. \
             Prefer 'you might consider', 'one perspective is', 'it could be worth exploring'. \
             Depth does not mean verbosity: avoid lexical echo and redundant restatements across paragraphs."
                .to_string()
        }
        DepthMode::Auto => {
            "Adapt your response depth to the complexity of the question. \
             For simple factual questions, be brief. For conceptual questions, provide more nuance. \
             Use a suggestive, non-judgmental tone. Avoid prescriptive language like \
             'you should' or 'you need to'. \
             Regardless of length, avoid stuttering repetition and duplicate phrasing."
                .to_string()
        }
    }
}

/// Iter 5 #4: build the "Available skills" system block. Returns None when no
/// auto_invocable skills are registered (so we don't add an empty section).
fn build_skills_system_block(
    skills: &[(String, String, Option<String>)],
) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut s = String::from(
        "Available skills (call as tools via `skill.<id>` with a single string `input`):\n",
    );
    for (id, name, when) in skills {
        if let Some(when) = when.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            s.push_str(&format!("- skill.{id} ({name}): {when}\n"));
        } else {
            s.push_str(&format!("- skill.{id} ({name})\n"));
        }
    }
    s.push_str(
        "Skills cannot invoke other skills. The skill streams its own output to the user;\n\
         after `skill.<id>` returns, acknowledge briefly without repeating the skill's content.",
    );
    Some(s)
}

/// 合并 `note_context`、可选 `vault_context` 与对话轮次，并校验角色。
fn assemble_ollama_messages(
    canonical_root: &Path,
    ai: &vault_config::AiConfig,
    args: &OllamaChatStreamStartArgs,
    embed_cache_bundle: Option<(PathBuf, PathBuf)>,
    auto_invocable_skills: &[(String, String, Option<String>)],
) -> Result<AssembleOutcome, String> {
    let mut out: Vec<LlmChatMessage> = Vec::new();
    let mut reply_context_sources = ReplyContextSources::default();

    if let Some(nc) = &args.note_context {
        note_privacy::validate_workspace_rel_path(&nc.rel_path)?;
        let is_private = note_privacy::markdown_treat_as_kf_private(&nc.markdown_for_gate);
        let redact = is_private
            && (matches!(ai.active_provider, ActiveProvider::Openai)
                || !ai.privacy.allow_private_content_in_local_llm);
        reply_context_sources.current_note = Some(ReplyCurrentNoteSource {
            rel_path: nc.rel_path.clone(),
            mode: if redact {
                ReplyCurrentNoteMode::Redacted
            } else {
                ReplyCurrentNoteMode::Full
            },
        });
        let system_content = if redact {
            "The user has a Markdown note open that is marked private (kf-private). Its full content is not included in this request. Answer using only the visible user messages and general knowledge.".to_string()
        } else {
            build_note_system_content(&nc.rel_path, &nc.markdown_for_gate)
        };
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: system_content,
            ..Default::default()
        });
    }

    if let Some(ref tf) = args.thought_focus_context {
        if !tf.thought_id.trim().is_empty() && !tf.thought_body.trim().is_empty() {
            reply_context_sources.thought_focus = Some(ReplyThoughtFocusSource {
                thought_id: tf.thought_id.trim().to_string(),
            });
            let has_note = args.note_context.is_some();
            out.push(LlmChatMessage {
                role: "system".to_string(),
                content: build_thought_focus_system_block(tf, has_note),
                ..Default::default()
            });
        }
    }

    let last_user_query: Option<String> = args
        .messages
        .iter()
        .rev()
        .find(|m| m.role.trim() == "user")
        .map(|m| m.content.clone());

    let mut resolved_depth: Option<DepthMode> = None;
    let mut auto_resolve_reason: Option<String> = None;
    let depth_for_prompt: Option<DepthMode> = match args.depth_mode {
        None => None,
        Some(DepthMode::Auto) => {
            let q = last_user_query.as_deref().unwrap_or("");
            let (d, reason) = resolve_auto_depth_heuristic(q);
            resolved_depth = Some(d);
            auto_resolve_reason = Some(reason);
            Some(d)
        }
        Some(d) => Some(d),
    };

    // Vault 摘录：预算与 `maxContextTokens` 粗挂钩（字符≈4×token），当前笔记 system 优先占满后再给摘录。
    if let (Some(vc), Some(q)) = (&args.vault_context, last_user_query.as_deref()) {
        if !vc.snippets.is_empty() {
            let rebuilt = vault_context_search::rebuild_vault_snippets_for_llm(
                canonical_root,
                ai,
                &vc.snippets,
                q,
                1200,
                96 * 1024,
            );
            if !rebuilt.is_empty() {
                let default_total: usize = 32_000;
                let total_budget = ai
                    .request
                    .max_context_tokens
                    .map(|m| (m as usize).saturating_mul(4))
                    .unwrap_or(default_total)
                    .clamp(4_000, 100_000);
                let used_note: usize = out.first().map(|m| m.content.chars().count()).unwrap_or(0);
                let used_msgs: usize = args.messages.iter().map(|m| m.content.chars().count() + 8).sum();
                let vault_cap = total_budget
                    .saturating_sub(used_note)
                    .saturating_sub(used_msgs)
                    .min(12_000)
                    .max(400);
                if let Some((vault_block, truncated, used_rel_paths)) =
                    vault_context_search::build_vault_context_system_block(&rebuilt, vault_cap)
                {
                    let used_set: HashSet<&str> = used_rel_paths.iter().map(String::as_str).collect();
                    let entries: Vec<ReplyVaultKeywordEntry> = rebuilt
                        .iter()
                        .filter(|s| used_set.contains(s.rel_path.as_str()))
                        .map(|s| ReplyVaultKeywordEntry {
                            rel_path: s.rel_path.clone(),
                            kind: s.kind.clone(),
                        })
                        .collect();
                    reply_context_sources.vault_keyword = Some(ReplyVaultKeywordSource {
                        entries,
                        truncated,
                    });
                    out.push(LlmChatMessage {
                        role: "system".to_string(),
                        content: vault_block,
                        ..Default::default()
                    });
                } else {
                    reply_context_sources.vault_keyword = Some(ReplyVaultKeywordSource {
                        entries: Vec::new(),
                        truncated: true,
                    });
                }
            }
        }
    }

    let sem_ctx_on = args.semantic_context_enabled.unwrap_or(true);
    if sem_ctx_on {
        if let Ok(sem_cfg) = vault_config::load_semantic_merged(canonical_root) {
            if sem_cfg.enabled {
                if let (Some(q), Some(paths)) = (last_user_query.as_deref(), embed_cache_bundle.as_ref()) {
                    let (cache, bundle) = paths;
                    let kw_paths: Vec<String> = args
                        .vault_context
                        .as_ref()
                        .map(|vc| vc.snippets.iter().map(|s| s.rel_path.clone()).collect())
                        .unwrap_or_default();
                    let omit_semantic_docs: Vec<String> = args
                        .note_context
                        .as_ref()
                        .map(|n| vec![n.rel_path.clone()])
                        .unwrap_or_default();
                    if let Some(sem_res) = semantic_index::build_semantic_context_for_llm(
                        canonical_root,
                        cache,
                        bundle,
                        q,
                        &sem_cfg,
                        &kw_paths,
                        &omit_semantic_docs,
                    ) {
                        reply_context_sources.semantic = ReplySemanticSource {
                            injected: true,
                            document_paths: sem_res.used.document_paths.clone(),
                            thought_ids: sem_res.used.thought_ids.clone(),
                        };
                        out.push(LlmChatMessage {
                            role: "system".to_string(),
                            content: sem_res.block,
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    // 深度模式系统指令（迭代 3）；Auto 在入模前已解析为浅/中/深
    if let Some(depth) = depth_for_prompt {
        let depth_instruction = build_depth_system_instruction(depth);
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: depth_instruction,
            ..Default::default()
        });
    }

    // 深化子轮次上下文（迭代 3 Phase 4）
    if let Some(invite) = &args.invite_context {
        let mut ctx = format!(
            "The user accepted a deepening question: \"{}\". ",
            invite.question
        );
        if let Some(ref excerpt) = invite.thought_excerpt {
            ctx.push_str(&format!(
                "Relevant previous thought from their notes: \"{}\". ",
                excerpt
            ));
        }
        ctx.push_str(
            "Respond with: (1) acknowledge what they already wrote, \
             (2) offer a complementary angle they haven't considered, \
             (3) connect to broader context. Be suggestive, not prescriptive."
        );
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: ctx,
            ..Default::default()
        });
    }

    // 语言匹配指令：确保 LLM 使用用户相同的语言回复
    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: "IMPORTANT: Always respond in the same language the user writes in. \
                  If the user writes in Chinese, respond entirely in Chinese. \
                  If the user writes in English, respond in English. \
                  Match the user's language exactly."
            .to_string(),
        ..Default::default()
    });

    // 叠词 / 重复词抑制：无笔记上下文时仅靠本条生效；与深度指令互补
    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: CHAT_ANTI_REPETITION_SYSTEM.to_string(),
        ..Default::default()
    });

    // Iter 3.5 P0-2：开启工具调用时,明确告诉 LLM "先发现后读",避免按训练直觉假设文件在根目录。
    if args.tools_enabled {
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: agent_loop::TOOL_USE_DISCOVERY_HINT.to_string(),
            ..Default::default()
        });
        // Iter 5 #4: when auto_invocable skills are registered, surface them so
        // the model can pick `skill.<id>` over recreating the workflow inline.
        if let Some(block) = build_skills_system_block(auto_invocable_skills) {
            out.push(LlmChatMessage {
                role: "system".to_string(),
                content: block,
                ..Default::default()
            });
        }
    }

    for m in &args.messages {
        let role = m.role.trim();
        if role != "user" && role != "assistant" {
            return Err("Invalid message role: expected user or assistant.".to_string());
        }
        out.push(LlmChatMessage {
            role: role.to_string(),
            content: m.content.clone(),
            ..Default::default()
        });
    }
    Ok(AssembleOutcome {
        messages: out,
        resolved_depth,
        auto_resolve_reason,
        reply_context_sources,
    })
}

pub(super) fn emit_error(app: &AppHandle, session_id: &str, code: Option<&str>, message: &str) {
    let payload = LlmStreamErrorPayload {
        session_id: session_id.to_string(),
        code: code.map(str::to_string),
        message: message.to_string(),
    };
    let _ = app.emit("llm:stream-error", payload);
}

// --- 命令 ---

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListOllamaModelsArgs {
    /// 非空时优先使用（设置页未保存的 URL）；否则读磁盘配置
    #[serde(default)]
    pub base_url: Option<String>,
}

/// 列出 Ollama 模型；默认用配置中的 `ollama.baseUrl`，`args.baseUrl` 非空时覆盖（便于设置页探测）。
#[tauri::command]
pub async fn list_ollama_models(
    state: State<'_, crate::WorkspaceState>,
    args: ListOllamaModelsArgs,
) -> Result<Vec<String>, String> {
    let root = lock_workspace_root(&state)?;
    let ai = tauri::async_runtime::spawn_blocking(move || vault_config::load_ai_config_internal(&root))
        .await
        .map_err(|e| e.to_string())??;
    let base = match args.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => vault_config::normalize_ollama_base_url(raw),
        None => ai.ollama.base_url.clone(),
    };
    let timeout_ms = ai.request.timeout_ms;
    ollama::list_models(&base, timeout_ms).await
}

#[tauri::command]
pub async fn start_ollama_chat_stream(
    app: AppHandle,
    workspace: State<'_, crate::WorkspaceState>,
    sessions: State<'_, Arc<LlmSessionState>>,
    registry: State<'_, Arc<ToolRegistry>>,
    ctx_factory: State<'_, Arc<ToolContextFactory>>,
    approval: State<'_, Arc<approval::ToolApprovalState>>,
    skills: State<'_, Arc<SkillRegistry>>,
    args: OllamaChatStreamStartArgs,
) -> Result<OllamaChatStreamStartResponse, String> {
    let root = lock_workspace_root(&workspace)?;
    let root_for_config = root.clone();
    let ai = tauri::async_runtime::spawn_blocking(move || vault_config::load_ai_config_internal(&root_for_config))
        .await
        .map_err(|e| e.to_string())??;

    if ai.active_provider == ActiveProvider::Openai {
        return Err("OpenAI provider is not implemented yet.".to_string());
    }

    if args.messages.is_empty() {
        return Err("At least one message is required.".to_string());
    }

    let cache = semantic_index::default_model_cache_dir();
    let bundle = semantic_index::resolve_bundle_model_dir(&app);
    let embed_paths = Some((cache.clone(), bundle.clone()));
    let tools_enabled = args.tools_enabled;
    // Iter 5 #4: snapshot auto_invocable skills so assemble_ollama_messages can
    // surface them in the chat system prompt.
    let skills_for_prompt: Vec<(String, String, Option<String>)> = if tools_enabled {
        skills
            .list()
            .into_iter()
            .filter(|m| m.auto_invocable)
            .map(|m| (m.id, m.name, m.when_to_use))
            .collect()
    } else {
        Vec::new()
    };
    let outcome = assemble_ollama_messages(&root, &ai, &args, embed_paths, &skills_for_prompt)?;
    let messages = outcome.messages;
    let resolved_depth = outcome.resolved_depth;
    let reply_context_sources = outcome.reply_context_sources;

    if let (Some(d), Some(reason)) = (resolved_depth, outcome.auto_resolve_reason.as_ref()) {
        if matches!(args.depth_mode, Some(DepthMode::Auto)) {
            let entry = depth_decisions::DepthDecisionEntry {
                timestamp: Utc::now(),
                auto_resolved: d,
                reason: reason.clone(),
                user_override: None,
            };
            let _ = depth_decisions::append_decision(&root, &entry);
        }
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

    let session_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    sessions.register(session_id.clone(), cancel.clone());

    let app_h = app.clone();
    let sid = session_id.clone();
    let sessions_arc = Arc::clone(sessions.inner());
    let base = ai.ollama.base_url.clone();
    let timeout_ms = ai.request.timeout_ms;
    let temp = ai.parameters.temperature;
    let top_p = ai.parameters.top_p;
    let registry_arc = Arc::clone(registry.inner());
    let ctx_factory_arc = Arc::clone(ctx_factory.inner());
    let approval_arc = Arc::clone(approval.inner());
    let workspace_root = root.clone();
    // 优先使用前端传入的 conversationId（用于审批缓存作用域）；缺省退化为 session_id。
    let conversation_id = args
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session_id.clone());

    tokio::spawn(async move {
        if tools_enabled {
            // 把工具 manifest 列表转成 Ollama tools 协议格式
            let manifests = registry_arc.list_for_llm(ToolScope::Global);
            let tools_json = agent_loop::list_for_llm_to_ollama_tools(&manifests);

            agent_loop::run_agent_stream(
                app_h.clone(),
                sid.clone(),
                messages,
                tools_json,
                registry_arc,
                ctx_factory_arc,
                workspace_root,
                Some(cache),
                Some(bundle),
                base,
                model,
                temp,
                top_p,
                timeout_ms,
                cancel,
                agent_loop::AgentLoopConfig::default(),
                conversation_id,
                approval_arc,
            )
            .await;
        } else {
            let _ = run_chat_stream(
                app_h.clone(),
                sid.clone(),
                base,
                model,
                messages,
                temp,
                top_p,
                timeout_ms,
                cancel,
                None,
            )
            .await;
        }
        sessions_arc.remove_session(&sid);
    });

    Ok(OllamaChatStreamStartResponse {
        session_id,
        resolved_depth,
        reply_context_sources,
    })
}

#[tauri::command]
pub fn abort_llm_stream(session_id: String, sessions: State<'_, Arc<LlmSessionState>>) -> Result<(), String> {
    if let Some(token) = sessions.take_cancel(&session_id) {
        token.cancel();
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondToolApprovalArgs {
    pub approval_id: String,
    pub decision: bool,
}

/// 前端响应一次审批请求（Allow / Deny）。
#[tauri::command]
pub fn respond_tool_approval(
    args: RespondToolApprovalArgs,
    approval: State<'_, Arc<approval::ToolApprovalState>>,
) -> Result<(), String> {
    approval.resolve(&args.approval_id, args.decision)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearConversationApprovalsArgs {
    pub conversation_id: String,
}

/// 切换或删除会话时清理该会话的 ConfirmOncePerSession 缓存。
#[tauri::command]
pub fn clear_conversation_approvals(
    args: ClearConversationApprovalsArgs,
    approval: State<'_, Arc<approval::ToolApprovalState>>,
) -> Result<(), String> {
    approval.clear_conversation(&args.conversation_id);
    Ok(())
}

#[cfg(test)]
mod skills_block_tests {
    use super::build_skills_system_block;

    #[test]
    fn returns_none_when_empty() {
        assert!(build_skills_system_block(&[]).is_none());
    }

    #[test]
    fn includes_id_name_and_when_to_use() {
        let skills = vec![
            (
                "writing_coach".to_string(),
                "写作教练".to_string(),
                Some("打磨笔记".to_string()),
            ),
            ("review".to_string(), "复盘".to_string(), None),
        ];
        let block = build_skills_system_block(&skills).expect("should build");
        assert!(block.contains("skill.writing_coach"));
        assert!(block.contains("写作教练"));
        assert!(block.contains("打磨笔记"));
        assert!(block.contains("skill.review"));
        assert!(block.contains("复盘"));
        // The trailing instruction must be present so the parent LLM does not
        // re-render the skill's content.
        assert!(block.contains("Skills cannot invoke other skills"));
    }
}
