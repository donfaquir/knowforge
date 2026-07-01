//! LLM integration: model listing, streaming chat, agent loop, session abort.

pub(crate) mod agent_loop;
pub mod approval;
pub(crate) mod context_guard;
pub(crate) mod planning;
pub(crate) mod provider;
pub(crate) mod provider_impl;
pub(crate) mod tool_result_processor;
pub mod memory;

pub use provider::{
    build_shared_http_client, create_provider, CompletionOverrides,
    LlmProvider,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentMode {
    Direct,
    Planning,
}

fn determine_agent_mode(ai: &vault_config::AiConfig) -> AgentMode {
    if ai.planning_enabled {
        AgentMode::Planning
    } else {
        AgentMode::Direct
    }
}

use crate::depth_decisions;
use crate::lock_workspace_root;
use crate::note_privacy;
use crate::semantic_index;
use crate::skills::SkillRegistry;
use crate::tools::context::ToolContextFactory;
use crate::tools::registry::{ToolFilter, ToolRegistry};
use crate::vault_config::{self, DepthMode};
use crate::vault_context_search::{self, VaultSnippetKind};
use std::path::PathBuf;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
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
    pub arguments: Value,
}

/// 附带当前笔记时由前端传入；**笔记正文出站裁决在 Rust**（任务 09）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContextIn {
    pub rel_path: String,
    pub markdown_for_gate: String,
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
pub struct ChatStreamStartArgs {
    /// 仅 `user` / `assistant`；笔记 system 由本模块根据 `note_context` 与配置拼装。
    pub messages: Vec<LlmChatMessage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub note_context: Option<NoteContextIn>,
    #[serde(default)]
    pub include_vault_context: Option<bool>,
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
    /// 是否启用 Tool Calling Loop（P2）；
    /// `None` 时回退到 `AiConfig::tools_enabled`(Iter 5 #4 起改由配置页面控制)。
    #[serde(default)]
    pub tools_enabled: Option<bool>,
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
pub struct ChatStreamStartResponse {
    pub session_id: String,
    /// 仅当请求 `depthMode` 为 `auto` 时返回：本次启发式解析出的具体档位。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_depth: Option<DepthMode>,
    pub reply_context_sources: ReplyContextSources,
    pub provider_label: String,
    pub model_name: String,
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

struct AssembleFastOutcome {
    messages: Vec<LlmChatMessage>,
    context_insert_pos: usize,
    resolved_depth: Option<DepthMode>,
    auto_resolve_reason: Option<String>,
    reply_context_sources: ReplyContextSources,
}

#[derive(Default)]
struct ContextBlocks {
    vault_block: Option<String>,
    vault_snippets: Vec<vault_context_search::VaultSnippetRecord>,
    vault_meta: Option<vault_context_search::SearchWorkspaceContextMeta>,
    vault_reply_sources: Option<ReplyVaultKeywordSource>,
    semantic_block: Option<String>,
    semantic_reply_sources: ReplySemanticSource,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextReadyPayload {
    session_id: String,
    snippets: Vec<vault_context_search::VaultSnippetRecord>,
    meta: Option<vault_context_search::SearchWorkspaceContextMeta>,
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

fn assemble_messages_fast(
    ai: &vault_config::AiConfig,
    args: &ChatStreamStartArgs,
    auto_invocable_skills: &[(String, String, Option<String>)],
) -> Result<AssembleFastOutcome, String> {
    let mut out: Vec<LlmChatMessage> = Vec::new();
    let mut reply_context_sources = ReplyContextSources::default();

    if let Some(nc) = &args.note_context {
        note_privacy::validate_workspace_rel_path(&nc.rel_path)?;
        let is_private = note_privacy::markdown_treat_as_kf_private(&nc.markdown_for_gate);
        let redact = is_private && ai.should_redact_private();
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

    let context_insert_pos = out.len();

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

    if let Some(depth) = depth_for_prompt {
        let depth_instruction = build_depth_system_instruction(depth);
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: depth_instruction,
            ..Default::default()
        });
    }

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

    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: "IMPORTANT: Always respond in the same language the user writes in. \
                  If the user writes in Chinese, respond entirely in Chinese. \
                  If the user writes in English, respond in English. \
                  Match the user's language exactly."
            .to_string(),
        ..Default::default()
    });

    out.push(LlmChatMessage {
        role: "system".to_string(),
        content: CHAT_ANTI_REPETITION_SYSTEM.to_string(),
        ..Default::default()
    });

    let tools_enabled_eff = args.tools_enabled.unwrap_or(ai.tools_enabled);
    if tools_enabled_eff {
        out.push(LlmChatMessage {
            role: "system".to_string(),
            content: agent_loop::TOOL_USE_DISCOVERY_HINT.to_string(),
            ..Default::default()
        });
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
    Ok(AssembleFastOutcome {
        messages: out,
        context_insert_pos,
        resolved_depth,
        auto_resolve_reason,
        reply_context_sources,
    })
}

fn inject_context_blocks(outcome: &mut AssembleFastOutcome, blocks: &ContextBlocks) {
    let pos = outcome.context_insert_pos;
    if let Some(ref sb) = blocks.semantic_block {
        outcome.messages.insert(
            pos,
            LlmChatMessage {
                role: "system".to_string(),
                content: sb.clone(),
                ..Default::default()
            },
        );
        outcome.reply_context_sources.semantic = blocks.semantic_reply_sources.clone();
    }
    if let Some(ref vb) = blocks.vault_block {
        outcome.messages.insert(
            pos,
            LlmChatMessage {
                role: "system".to_string(),
                content: vb.clone(),
                ..Default::default()
            },
        );
        if let Some(ref vs) = blocks.vault_reply_sources {
            outcome.reply_context_sources.vault_keyword = Some(vs.clone());
        }
    }
}

async fn build_context_pipeline(
    canonical_root: PathBuf,
    ai: vault_config::AiConfig,
    last_user_query: Option<String>,
    note_context_rel_path: Option<String>,
    include_vault_context: bool,
    semantic_enabled: bool,
    embed_cache: Arc<semantic_index::EmbeddingCache>,
    cache_dir: PathBuf,
    bundle_dir: PathBuf,
    note_chars_count: usize,
    conversation_chars_count: usize,
) -> ContextBlocks {
    let mut blocks = ContextBlocks::default();
    let query = match last_user_query {
        Some(q) if !q.trim().is_empty() => q,
        _ => return blocks,
    };

    let mut kw_paths: Vec<String> = Vec::new();

    if include_vault_context {
        let root = canonical_root.clone();
        let query_clone = query.clone();
        let exclude = note_context_rel_path
            .clone()
            .map(|p| vec![p])
            .unwrap_or_default();

        let vault_result = tauri::async_runtime::spawn_blocking(move || {
            vault_context_search::search_workspace_context_blocking(
                &root,
                vault_context_search::SearchWorkspaceContextArgs {
                    query: query_clone,
                    exclude_rel_paths: exclude,
                    limits: None,
                },
            )
            .ok()
        })
        .await
        .ok()
        .flatten();

        if let Some(ref vr) = vault_result {
            blocks.vault_meta = Some(vr.meta.clone());
            blocks.vault_snippets = vr.snippets.clone();
            kw_paths = vr.snippets.iter().map(|s| s.rel_path.clone()).collect();

            let root = canonical_root.clone();
            let snippets = vr.snippets.clone();
            let q = query.clone();
            let ai_for_rebuild = ai.clone();
            let rebuilt = tauri::async_runtime::spawn_blocking(move || {
                vault_context_search::rebuild_vault_snippets_for_llm(
                    &root,
                    &ai_for_rebuild,
                    &snippets,
                    &q,
                    1200,
                    96 * 1024,
                )
            })
            .await
            .unwrap_or_default();

            if !rebuilt.is_empty() {
                let default_total: usize = 32_000;
                let total_budget = ai
                    .request
                    .max_context_tokens
                    .map(|m| (m as usize).saturating_mul(4))
                    .unwrap_or(default_total)
                    .clamp(4_000, 100_000);
                let vault_cap = total_budget
                    .saturating_sub(note_chars_count)
                    .saturating_sub(conversation_chars_count)
                    .min(12_000)
                    .max(400);
                if let Some((vault_block, truncated, used_rel_paths)) =
                    vault_context_search::build_vault_context_system_block(&rebuilt, vault_cap)
                {
                    let used_set: HashSet<&str> =
                        used_rel_paths.iter().map(String::as_str).collect();
                    let entries: Vec<ReplyVaultKeywordEntry> = rebuilt
                        .iter()
                        .filter(|s| used_set.contains(s.rel_path.as_str()))
                        .map(|s| ReplyVaultKeywordEntry {
                            rel_path: s.rel_path.clone(),
                            kind: s.kind.clone(),
                        })
                        .collect();
                    blocks.vault_reply_sources = Some(ReplyVaultKeywordSource {
                        entries,
                        truncated,
                    });
                    blocks.vault_block = Some(vault_block);
                } else {
                    blocks.vault_reply_sources = Some(ReplyVaultKeywordSource {
                        entries: Vec::new(),
                        truncated: true,
                    });
                }
            }
        }
    }

    if semantic_enabled {
        let root = canonical_root.clone();
        let q = query.clone();
        let omit = note_context_rel_path.map(|p| vec![p]).unwrap_or_default();
        let ec = Arc::clone(&embed_cache);
        let cd = cache_dir;
        let bd = bundle_dir;

        let semantic = tauri::async_runtime::spawn_blocking(move || {
            let sem_cfg = vault_config::load_semantic_merged(&root).ok()?;
            if !sem_cfg.enabled {
                return None;
            }
            semantic_index::build_semantic_context_for_llm(
                &root, &cd, &bd, &q, &sem_cfg, &kw_paths, &omit, &ec,
            )
        })
        .await
        .ok()
        .flatten();

        if let Some(sem) = semantic {
            blocks.semantic_block = Some(sem.block);
            blocks.semantic_reply_sources = ReplySemanticSource {
                injected: true,
                document_paths: sem.used.document_paths,
                thought_ids: sem.used.thought_ids,
            };
        }
    }

    blocks
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
pub struct ListModelsArgs {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[tauri::command]
pub async fn list_models(
    state: State<'_, crate::WorkspaceState>,
    http_client: State<'_, Arc<reqwest::Client>>,
    args: ListModelsArgs,
) -> Result<Vec<String>, String> {
    let root = lock_workspace_root(&state)?;
    let ai = tauri::async_runtime::spawn_blocking(move || vault_config::load_ai_config_internal(&root))
        .await
        .map_err(|e| e.to_string())??;

    let id = args.provider_id.as_deref().unwrap_or(&ai.active_provider_id);
    let profile = ai.providers.iter().find(|p| p.id == id).or_else(|| ai.active_profile());

    let base = match args.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => vault_config::normalize_openai_base_url(raw),
        None => profile.map(|p| p.base_url.clone()).unwrap_or_default(),
    };
    let key = match args.api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => k.to_string(),
        None => profile.map(|p| p.api_key.clone()).unwrap_or_default(),
    };

    let provider = provider_impl::UnifiedProvider::new(
        Arc::clone(http_client.inner()),
        base,
        key,
        String::new(),
        ai.parameters.temperature,
        ai.parameters.top_p,
        ai.request.timeout_ms,
        profile.and_then(|p| p.organization_id.clone()),
    );
    provider.list_models().await
}

#[tauri::command]
pub async fn start_chat_stream(
    app: AppHandle,
    workspace: State<'_, crate::WorkspaceState>,
    sessions: State<'_, Arc<LlmSessionState>>,
    registry: State<'_, Arc<ToolRegistry>>,
    ctx_factory: State<'_, Arc<ToolContextFactory>>,
    approval: State<'_, Arc<approval::ToolApprovalState>>,
    skills: State<'_, Arc<SkillRegistry>>,
    http_client: State<'_, Arc<reqwest::Client>>,
    embed_cache_state: State<'_, Arc<semantic_index::EmbeddingCache>>,
    args: ChatStreamStartArgs,
) -> Result<ChatStreamStartResponse, String> {
    let root = lock_workspace_root(&workspace)?;
    let root_for_config = root.clone();
    let ai = tauri::async_runtime::spawn_blocking(move || vault_config::load_ai_config_internal(&root_for_config))
        .await
        .map_err(|e| e.to_string())??;

    if args.messages.is_empty() {
        return Err("At least one message is required.".to_string());
    }

    let model_override = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let active_profile = ai.active_profile()
        .ok_or("No active provider configured.")?;
    let resp_provider_label = active_profile.label.clone();
    let resp_model_name = model_override
        .map(str::to_string)
        .or_else(|| provider::resolve_model_name(
            active_profile.last_used_model.as_deref(),
            &active_profile.default_model,
        ))
        .unwrap_or_default();
    let http_client_arc = Arc::clone(http_client.inner());
    let provider = create_provider(&ai, model_override.map(|s| s), &http_client_arc)?;

    let cache = semantic_index::default_model_cache_dir();
    let bundle = semantic_index::resolve_bundle_model_dir(&app);
    let tools_enabled = args.tools_enabled.unwrap_or(ai.tools_enabled);
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

    let mut fast_outcome = assemble_messages_fast(&ai, &args, &skills_for_prompt)?;
    let resolved_depth = fast_outcome.resolved_depth;
    let reply_context_sources = fast_outcome.reply_context_sources.clone();

    if let (Some(d), Some(reason)) = (resolved_depth, fast_outcome.auto_resolve_reason.as_ref()) {
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

    let session_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    sessions.register(session_id.clone(), cancel.clone());

    let app_h = app.clone();
    let sid = session_id.clone();
    let sessions_arc = Arc::clone(sessions.inner());
    let registry_arc = Arc::clone(registry.inner());
    let ctx_factory_arc = Arc::clone(ctx_factory.inner());
    let approval_arc = Arc::clone(approval.inner());
    let workspace_root = root.clone();
    let conversation_id = args
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session_id.clone());

    let agent_mode = if tools_enabled {
        determine_agent_mode(&ai)
    } else {
        AgentMode::Direct
    };

    let memory_enabled = ai.memory_enabled;
    let reflection_mode = ai.memory_reflection_mode.clone();
    let ai_for_memory = ai.clone();

    let include_vault_context = args.include_vault_context.unwrap_or(true);
    let semantic_enabled = args.semantic_context_enabled.unwrap_or(true);
    let note_context_rel_path = args.note_context.as_ref().map(|nc| nc.rel_path.clone());
    let last_user_query: Option<String> = args
        .messages
        .iter()
        .rev()
        .find(|m| m.role.trim() == "user")
        .map(|m| m.content.clone());
    let note_chars_count: usize = fast_outcome
        .messages
        .first()
        .map(|m| m.content.chars().count())
        .unwrap_or(0);
    let conversation_chars_count: usize = args
        .messages
        .iter()
        .map(|m| m.content.chars().count() + 8)
        .sum();
    let embed_cache_arc = Arc::clone(embed_cache_state.inner());

    tokio::spawn(async move {
        let (context_blocks, memory_manager) = tokio::join!(
            build_context_pipeline(
                workspace_root.clone(),
                ai.clone(),
                last_user_query,
                note_context_rel_path,
                include_vault_context,
                semantic_enabled,
                embed_cache_arc,
                cache.clone(),
                bundle.clone(),
                note_chars_count,
                conversation_chars_count,
            ),
            async {
                if memory_enabled {
                    let extraction_provider =
                        provider::create_provider(&ai_for_memory, None, &http_client_arc).ok();
                    let mgr =
                        memory::MemoryManager::new(workspace_root.clone(), extraction_provider);
                    Some(Arc::new(tokio::sync::Mutex::new(mgr)))
                } else {
                    None
                }
            },
        );

        inject_context_blocks(&mut fast_outcome, &context_blocks);

        if let Some(ref mm) = memory_manager {
            let mgr = mm.lock().await;
            if let Some(mem_msg) = mgr.format_for_injection() {
                let pos = if fast_outcome.messages.is_empty() { 0 } else { 1 };
                fast_outcome.messages.insert(
                    pos,
                    LlmChatMessage {
                        role: "system".to_string(),
                        content: mem_msg,
                        ..Default::default()
                    },
                );
            }
            drop(mgr);
        }

        let _ = app_h.emit(
            "llm:context-ready",
            ContextReadyPayload {
                session_id: sid.clone(),
                snippets: context_blocks.vault_snippets,
                meta: context_blocks.vault_meta,
                reply_context_sources: fast_outcome.reply_context_sources.clone(),
            },
        );

        let messages = fast_outcome.messages;
        let memory_manager: agent_loop::SharedMemoryManager = memory_manager;

        if tools_enabled {
            let manifests = registry_arc.list_for_llm_filtered(&ToolFilter::all());
            if std::env::var("KNOWFORGE_DEBUG_TOOLS").is_ok() {
                let tool_names: Vec<&str> = manifests.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                    .collect();
                eprintln!(
                    "[tool-filter] mode={:?} injected {}/{} tools: {:?}",
                    agent_mode,
                    tool_names.len(),
                    registry_arc.list_for_llm_filtered(&ToolFilter::all()).len(),
                    tool_names,
                );
            }
            let tools_json = provider.convert_tools(&manifests);
            let loop_config = agent_loop::AgentLoopConfig {
                timeout_ms: ai.request.timeout_ms,
                max_context_tokens: ai.request.max_context_tokens,
                ..Default::default()
            };

            match agent_mode {
                AgentMode::Direct => {
                    let _ = agent_loop::run_agent_stream(
                        app_h.clone(),
                        sid.clone(),
                        messages,
                        tools_json,
                        registry_arc,
                        ctx_factory_arc,
                        workspace_root,
                        Some(cache),
                        Some(bundle),
                        provider.clone(),
                        cancel,
                        loop_config,
                        conversation_id,
                        approval_arc,
                        memory_manager.clone(),
                    )
                    .await;
                }
                AgentMode::Planning => {
                    let _ = planning::run_planned_agent(
                        app_h.clone(),
                        sid.clone(),
                        messages,
                        tools_json,
                        registry_arc,
                        ctx_factory_arc,
                        workspace_root,
                        Some(cache),
                        Some(bundle),
                        provider.clone(),
                        cancel,
                        loop_config,
                        conversation_id,
                        approval_arc,
                        memory_manager.clone(),
                    )
                    .await;
                }
            }

        } else {
            let msgs_for_extraction = messages.clone();
            let result = provider
                .chat_stream(&app_h, &sid, messages, None, cancel)
                .await;
            if let Ok(ref r) = result {
                let mut final_msgs = msgs_for_extraction;
                if !r.content.is_empty() {
                    final_msgs.push(LlmChatMessage {
                        role: "assistant".to_string(),
                        content: r.content.clone(),
                        ..Default::default()
                    });
                }
                agent_loop::store_extraction_msgs(&memory_manager, &final_msgs).await;
            }
        }

        if let Some(mm) = memory_manager {
            let reflection_mode = reflection_mode.clone();
            let app_for_reflect = app_h.clone();
            let sid_for_reflect = sid.clone();
            tokio::spawn(async move {
                let mut mgr = mm.lock().await;
                let msgs = match mgr.take_extraction_messages() {
                    Some(m) => m,
                    None => return,
                };

                let update = match mgr.extract_session_update(&msgs).await {
                    Ok(Some(u)) => u,
                    Ok(None) => return,
                    Err(e) => {
                        emit_error(
                            &app_for_reflect,
                            &sid_for_reflect,
                            Some("memory_extraction_failed"),
                            &e,
                        );
                        return;
                    }
                };

                match reflection_mode.as_str() {
                    "off" => {
                        mgr.memory.merge_user_model(update);
                        if let Err(e) = mgr.memory.save(mgr.workspace_root()) {
                            eprintln!("[memory] Save failed: {e}");
                        }
                    }
                    "auto" => {
                        if memory::should_reflect(&msgs, &mgr.memory) {
                            let proposals = mgr.reflect_on_memory(&update).await;
                            if let Err(e) = mgr.create_snapshot() {
                                eprintln!("[memory] Snapshot failed: {e}");
                            }
                            mgr.memory.merge_user_model(update);
                            for p in &proposals {
                                if let Err(e) = memory::apply_single_proposal(&mut mgr.memory, p) {
                                    eprintln!("[memory] Apply proposal failed: {e}");
                                }
                            }
                        } else {
                            mgr.memory.merge_user_model(update);
                        }
                        if let Err(e) = mgr.memory.save(mgr.workspace_root()) {
                            eprintln!("[memory] Save failed: {e}");
                        }
                        mgr.delete_snapshot();
                    }
                    _ => {
                        // "confirm" (default)
                        if memory::should_reflect(&msgs, &mgr.memory) {
                            let proposals = mgr.reflect_on_memory(&update).await;
                            if proposals.is_empty() {
                                mgr.memory.merge_user_model(update);
                                if let Err(e) = mgr.memory.save(mgr.workspace_root()) {
                                    eprintln!("[memory] Save failed: {e}");
                                }
                            } else {
                                if let Err(e) = mgr.create_snapshot() {
                                    eprintln!("[memory] Snapshot failed: {e}");
                                }
                                mgr.memory.merge_user_model(update);
                                if let Err(e) = mgr.memory.save(mgr.workspace_root()) {
                                    eprintln!("[memory] Save failed: {e}");
                                }
                                let batch = memory::MemoryProposalBatch {
                                    session_id: sid_for_reflect.clone(),
                                    proposals,
                                    created_at: chrono::Utc::now().to_rfc3339(),
                                };
                                if let Err(e) = mgr.save_pending_proposals(&batch) {
                                    eprintln!("[memory] Save pending failed: {e}");
                                }
                                let _ = app_for_reflect.emit("llm:memory-proposals", &batch);
                            }
                        } else {
                            mgr.memory.merge_user_model(update);
                            if let Err(e) = mgr.memory.save(mgr.workspace_root()) {
                                eprintln!("[memory] Save failed: {e}");
                            }
                        }
                    }
                }
            });
        }

        sessions_arc.remove_session(&sid);
    });

    Ok(ChatStreamStartResponse {
        session_id,
        resolved_depth,
        reply_context_sources,
        provider_label: resp_provider_label,
        model_name: resp_model_name,
    })
}

#[tauri::command]
pub fn abort_llm_stream(session_id: String, sessions: State<'_, Arc<LlmSessionState>>) -> Result<(), String> {
    if let Some(token) = sessions.take_cancel(&session_id) {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub fn clear_agent_memory(
    workspace: State<'_, crate::WorkspaceState>,
) -> Result<(), String> {
    let root = lock_workspace_root(&workspace)?;
    memory::clear_memory_file(&root)
}

#[tauri::command]
pub fn apply_memory_proposals(
    workspace: State<'_, crate::WorkspaceState>,
    accepted_ids: Vec<String>,
) -> Result<(), String> {
    let root = lock_workspace_root(&workspace)?;
    let batch = memory::load_pending_proposals(&root)
        .ok_or_else(|| "No pending proposals".to_string())?;

    let mut mem = memory::AgentMemory::load(&root);

    for proposal in &batch.proposals {
        if accepted_ids.contains(&proposal.id) {
            memory::apply_single_proposal(&mut mem, proposal)?;
        }
    }

    mem.save(&root)?;
    memory::delete_pending_proposals(&root);
    memory::delete_snapshot(&root);

    Ok(())
}

#[tauri::command]
pub fn get_pending_memory_proposals(
    workspace: State<'_, crate::WorkspaceState>,
) -> Result<Option<memory::MemoryProposalBatch>, String> {
    let root = lock_workspace_root(&workspace)?;
    Ok(memory::load_pending_proposals(&root))
}

#[tauri::command]
pub fn dismiss_memory_proposals(
    workspace: State<'_, crate::WorkspaceState>,
) -> Result<(), String> {
    let root = lock_workspace_root(&workspace)?;
    memory::delete_pending_proposals(&root);
    memory::delete_snapshot(&root);
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

#[cfg(test)]
mod agent_mode_tests {
    use super::*;

    fn base_config() -> vault_config::AiConfig {
        vault_config::AiConfig::default()
    }

    #[test]
    fn direct_when_planning_disabled() {
        let ai = base_config();
        assert_eq!(determine_agent_mode(&ai), AgentMode::Direct);
    }

    #[test]
    fn planning_when_enabled() {
        let mut ai = base_config();
        ai.planning_enabled = true;
        assert_eq!(determine_agent_mode(&ai), AgentMode::Planning);
    }
}

