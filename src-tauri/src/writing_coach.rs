//! 写作教练：段落级论证检查 + Vault 关键词关联（Ollama JSON）

use crate::llm::create_provider;
use crate::llm::LlmChatMessage;
use crate::lock_workspace_root;
use crate::note_privacy;
use crate::challenge_review;
use crate::thought_retrieval::{self, SearchThoughtArgs};
use crate::vault_config::{self, DepthMode};
use crate::vault_context_search::{self, SearchWorkspaceContextArgs, SearchWorkspaceLimits};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::path::PathBuf;
use tauri::State;

const SYSTEM_WRITING_COACH: &str = r#"You are a writing coach in a personal knowledge app. The user is drafting a paragraph. Your job is ONLY:
1) Ask concise questions about possible logical gaps, vague terms, or missing premises (no rewriting).
2) Optionally suggest connections to the provided candidate notes/thoughts using wikilink-style titles.

Hard rules:
- Output MUST be one JSON object only (no markdown fences, no prose outside JSON). Use exactly these camelCase keys:
  - "reasoningQuestions": array of strings (0–5 short questions, same language as the paragraph when possible)
  - "links": array of objects, each { "title": string, "relPath": string, "kind": "note" | "thought", "thoughtId": optional string (required when kind is thought), "excerpt": optional string } — only use entries from the candidate list
- NEVER rewrite the user's text, NEVER give "change it to..." suggestions, NEVER judge quality (no "unclear", "bad writing", "poorly written").
- If nothing useful, return {"reasoningQuestions":[],"links":[]}."#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeWritingCoachArgs {
    pub paragraph_text: String,
    pub rel_path: String,
    /// 与界面语言对齐（如 `zh` / `en`），用于红线过滤后兜底问句语言
    #[serde(default)]
    pub ui_locale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingCoachLinkItem {
    pub title: String,
    pub rel_path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeWritingCoachResponse {
    pub reasoning_questions: Vec<String>,
    pub links: Vec<WritingCoachLinkItem>,
    /// true when vault has fewer than 5 markdown files — 前端隐藏知识连接区
    pub knowledge_module_skipped: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoachJsonRaw {
    #[serde(default)]
    reasoning_questions: Vec<String>,
    #[serde(default)]
    links: Vec<CoachLinkRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoachLinkRaw {
    title: String,
    rel_path: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    thought_id: Option<String>,
    excerpt: Option<String>,
}

struct PrepareOutcome {
    knowledge_module_skipped: bool,
    whitelist_keys: std::collections::HashSet<String>,
    user_body: String,
    ui_locale: Option<String>,
}

/// 模型输出问句全被过滤时的一条中性兜底（与 SYSTEM 要求「不评判、只提问」一致）
const FALLBACK_REASONING_QUESTION_EN: &str =
    "Which key term or claim here would benefit from a one-line definition?";
const FALLBACK_REASONING_QUESTION_ZH: &str = "这段话里，哪个关键术语或论断最需要先用一句话界定清楚？";

/// 若命中代写/评判等红线则丢弃该条提问
fn is_reasoning_question_allowed(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_lowercase();
    const BAD_EN: &[&str] = &[
        "rewrite",
        "rephrase",
        "replace this",
        "change this to",
        "suggest you change",
        "you should change",
        "poorly written",
        "bad writing",
        "not clear enough",
    ];
    const BAD_ZH: &[&str] = &[
        "建议你改成",
        "建议改成",
        "重写",
        "改写",
        "替换为",
        "不够清晰",
        "写得不好",
        "太差",
        "你应该把",
        "直接修改为",
    ];
    if BAD_EN.iter().any(|p| lower.contains(p)) {
        return false;
    }
    if BAD_ZH.iter().any(|p| t.contains(p)) {
        return false;
    }
    true
}

fn norm_rel_path(s: &str) -> String {
    s.trim().replace('\\', "/")
}

fn whitelist_key_note(rel_path: &str) -> String {
    format!("note:{}", norm_rel_path(rel_path))
}

fn whitelist_key_thought(rel_path: &str, thought_id: &str) -> String {
    format!("thought:{}:{}", norm_rel_path(rel_path), thought_id.trim())
}

fn title_from_rel_path(rel_path: &str) -> String {
    Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| rel_path.to_string())
}

/// 冷却期内为 true
fn writing_coach_cooldown_active(until: &Option<String>) -> bool {
    let Some(s) = until.as_ref().map(|x| x.trim()).filter(|x| !x.is_empty()) else {
        return false;
    };
    if let Ok(dt) = s.parse::<chrono::DateTime<chrono::Utc>>() {
        return dt > chrono::Utc::now();
    }
    false
}

fn extract_json_object(raw: &str) -> Result<String, String> {
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
    Ok(s[start..=end].to_string())
}

/// 仅用于 `knowledge_module_skipped`（少于 5 个 Markdown 则跳过知识模块）：收集到 `at_least` 个即停，避免大 Vault 全量枚举
fn count_markdown_files_at_least(canonical_root: &Path, at_least: usize) -> Result<usize, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(canonical_root, canonical_root, &mut paths, at_least)?;
    Ok(paths.len())
}

/// 从段落抽简单检索 query（与 vault_context_search::tokenize_query 配合）
fn paragraph_to_search_query(paragraph: &str) -> String {
    let flat: String = paragraph.chars().filter(|c| !c.is_control()).collect();
    let flat = flat.trim();
    if flat.is_empty() {
        return String::new();
    }
    let slice: String = flat.chars().take(200).collect();
    slice.split_whitespace().take(12).collect::<Vec<_>>().join(" ")
}

fn prepare_blocking(
    root: &Path,
    args: &AnalyzeWritingCoachArgs,
) -> Result<Option<PrepareOutcome>, String> {
    let cog = vault_config::load_cognitive_merged(root)?;

    if !cog.writing_coach_enabled {
        return Ok(None);
    }
    if cog.depth_mode == DepthMode::Shallow {
        return Ok(None);
    }
    if writing_coach_cooldown_active(&cog.writing_coach_cooldown_until) {
        return Ok(None);
    }
    let rel_path = norm_rel_path(&args.rel_path);
    note_privacy::validate_workspace_rel_path(&rel_path)?;

    let paragraph = args.paragraph_text.trim();
    if paragraph.is_empty() {
        return Err("paragraph_text is empty".to_string());
    }

    let md_count = count_markdown_files_at_least(root, 5)?;
    let knowledge_module_skipped = md_count < 5;

    let mut candidates: Vec<WritingCoachLinkItem> = Vec::new();
    let mut whitelist_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    let query = paragraph_to_search_query(paragraph);
    if !knowledge_module_skipped && !query.is_empty() {
        let vault_res = vault_context_search::search_workspace_context_blocking(
            root,
            SearchWorkspaceContextArgs {
                query: query.clone(),
                exclude_rel_paths: vec![rel_path.clone()],
                limits: Some(SearchWorkspaceLimits {
                    max_files_to_scan: Some(120),
                    max_snippets: Some(3),
                    max_chars_per_snippet: Some(400),
                    max_total_chars: Some(4000),
                    read_bytes_per_file: Some(48 * 1024),
                    max_duration_ms: Some(3000),
                }),
            },
        )?;

        for sn in vault_res.snippets.into_iter().take(3) {
            if matches!(sn.kind, vault_context_search::VaultSnippetKind::PrivateOmitted) {
                continue;
            }
            let rp = norm_rel_path(&sn.rel_path);
            let title = title_from_rel_path(&rp);
            let excerpt = sn.excerpt.clone();
            let item = WritingCoachLinkItem {
                title,
                rel_path: rp.clone(),
                kind: "note".to_string(),
                thought_id: None,
                excerpt,
            };
            whitelist_keys.insert(whitelist_key_note(&rp));
            candidates.push(item);
        }

        let thought_res = thought_retrieval::search_thought_blocking(
            root,
            SearchThoughtArgs {
                query,
                exclude_rel_paths: vec![rel_path.clone()],
                max_results: 3,
            },
        )?;

        for th in thought_res.thoughts.into_iter().take(3) {
            let rp = norm_rel_path(&th.rel_path);
            let title = format!("{} — {}", title_from_rel_path(&rp), th.thought_id);
            let excerpt = Some(th.excerpt.clone());
            let tid = th.thought_id.clone();
            let item = WritingCoachLinkItem {
                title,
                rel_path: rp.clone(),
                kind: "thought".to_string(),
                thought_id: Some(tid.clone()),
                excerpt,
            };
            whitelist_keys.insert(whitelist_key_thought(&rp, &tid));
            candidates.push(item);
        }
    }

    let mut cand_lines = String::new();
    for (i, c) in candidates.iter().enumerate() {
        cand_lines.push_str(&format!(
            "{}. title={:?} relPath={:?} kind={:?} thoughtId={:?} excerpt={:?}\n",
            i + 1,
            c.title,
            c.rel_path,
            c.kind,
            c.thought_id.as_deref().unwrap_or(""),
            c.excerpt.as_deref().unwrap_or("")
        ));
    }

    let user_body = format!(
        "Current file (relative path): {rel_path}\n\nParagraph:\n---\n{paragraph}\n---\n\nCandidate links (you MUST only output links matching these relPath values):\n{cand_lines}"
    );

    Ok(Some(PrepareOutcome {
        knowledge_module_skipped,
        whitelist_keys,
        user_body,
        ui_locale: args.ui_locale.clone(),
    }))
}

fn filter_response(
    parsed: CoachJsonRaw,
    prep: &PrepareOutcome,
) -> AnalyzeWritingCoachResponse {
    let mut reasoning_questions: Vec<String> = parsed
        .reasoning_questions
        .into_iter()
        .filter(|s| is_reasoning_question_allowed(s))
        .take(5)
        .collect();

    if reasoning_questions.is_empty() {
        // 模型输出若全被红线过滤，给一条中性提问，避免空白浮层（语言随界面）
        let q = if challenge_review::ui_locale_is_zh(prep.ui_locale.as_deref()) {
            FALLBACK_REASONING_QUESTION_ZH
        } else {
            FALLBACK_REASONING_QUESTION_EN
        };
        reasoning_questions.push(q.to_string());
    }

    let mut links: Vec<WritingCoachLinkItem> = Vec::new();
    for raw in parsed.links {
        let rp = norm_rel_path(&raw.rel_path);
        let kind = raw.kind.to_lowercase();
        let allowed = if kind == "thought" {
            let tid = raw.thought_id.as_deref().unwrap_or("").trim();
            if tid.is_empty() {
                false
            } else {
                prep
                    .whitelist_keys
                    .contains(&whitelist_key_thought(&rp, tid))
            }
        } else if kind == "note" {
            prep.whitelist_keys.contains(&whitelist_key_note(&rp))
        } else {
            false
        };
        if !allowed {
            continue;
        }
        links.push(WritingCoachLinkItem {
            title: raw.title.trim().to_string(),
            rel_path: rp,
            kind,
            thought_id: raw.thought_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            excerpt: raw.excerpt,
        });
    }

    AnalyzeWritingCoachResponse {
        reasoning_questions,
        links,
        knowledge_module_skipped: prep.knowledge_module_skipped,
    }
}

#[tauri::command]
pub async fn analyze_writing_coach(
    workspace: State<'_, crate::WorkspaceState>,
    args: AnalyzeWritingCoachArgs,
) -> Result<AnalyzeWritingCoachResponse, String> {
    let root = lock_workspace_root(&workspace)?;
    let root_for_prep = root.clone();
    let prep_opt = tauri::async_runtime::spawn_blocking(move || prepare_blocking(&root_for_prep, &args))
        .await
        .map_err(|e| e.to_string())??;

    let Some(prep) = prep_opt else {
        return Ok(AnalyzeWritingCoachResponse {
            reasoning_questions: vec![],
            links: vec![],
            knowledge_module_skipped: false,
        });
    };

    let ai = vault_config::load_ai_config_internal(&root)
        .map_err(|e| e.to_string())?;
    let provider = create_provider(&ai, None)?;

    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_WRITING_COACH.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".into(),
            content: prep.user_body.clone(),
            ..Default::default()
        },
    ];

    let raw = provider.chat_completion(&msgs, None).await
        .map_err(|e| e.to_string())?;

    let slice = extract_json_object(&raw).map_err(|_| "invalid coach JSON".to_string())?;
    let parsed: CoachJsonRaw =
        serde_json::from_str(&slice).map_err(|e| format!("failed to parse coach JSON: {e}"))?;

    Ok(filter_response(parsed, &prep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_neutral_question() {
        assert!(is_reasoning_question_allowed(
            "你提到的「高性能」在这里具体指延迟还是吞吐？"
        ));
    }

    #[test]
    fn blocks_rewrite_suggestion_zh() {
        assert!(!is_reasoning_question_allowed("建议你改成更简洁的表述。"));
    }

    #[test]
    fn blocks_judgment_zh() {
        assert!(!is_reasoning_question_allowed("这段写得不够清晰。"));
    }

    #[test]
    fn blocks_rewrite_en() {
        assert!(!is_reasoning_question_allowed("Please rewrite this paragraph."));
    }

    #[test]
    fn filter_response_fallback_respects_ui_locale_zh() {
        let prep = PrepareOutcome {
            knowledge_module_skipped: false,
            whitelist_keys: std::collections::HashSet::new(),
            user_body: String::new(),
            ui_locale: Some("zh-CN".to_string()),
        };
        let parsed = CoachJsonRaw {
            reasoning_questions: vec!["建议你改成更简洁的表述。".to_string()],
            links: vec![],
        };
        let out = filter_response(parsed, &prep);
        assert_eq!(out.reasoning_questions.len(), 1);
        assert!(
            out.reasoning_questions[0].contains("术语"),
            "expected Chinese fallback, got {:?}",
            out.reasoning_questions[0]
        );
    }

    #[test]
    fn filter_response_fallback_uses_en_when_not_zh_locale() {
        let prep = PrepareOutcome {
            knowledge_module_skipped: false,
            whitelist_keys: std::collections::HashSet::new(),
            user_body: String::new(),
            ui_locale: Some("en".to_string()),
        };
        let parsed = CoachJsonRaw {
            reasoning_questions: vec!["Please rewrite this paragraph.".to_string()],
            links: vec![],
        };
        let out = filter_response(parsed, &prep);
        assert_eq!(out.reasoning_questions.len(), 1);
        assert!(out.reasoning_questions[0].starts_with("Which"));
    }
}
