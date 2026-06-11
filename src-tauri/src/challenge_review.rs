//! 挑战式回顾：写回磁盘、Ollama 非流式问句/点评与后续队列 IPC（迭代 4）。

use chrono::{Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::llm::{create_provider, CompletionOverrides};
use crate::llm::LlmChatMessage;
use crate::thought_parser;
use crate::thought_retrieval;
use crate::vault_config::{self, DepthMode};
use crate::{is_markdown_path, join_under_root, sanitize_io_error};

// --- 写回 ---

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyChallengePassArgs {
    pub rel_path: String,
    pub thought_id: String,
    /// 未通过或敷衍时不写回元数据
    #[serde(default = "default_passed_true")]
    pub passed: bool,
}

fn default_passed_true() -> bool {
    true
}

/// 读改写落盘：将挑战通过状态写入笔记 Markdown。
///
/// 写入采用同目录临时文件 + `rename`（与 `atomic_write_string_in_parent` / `vault_config::atomic_write_json` 同类），
/// 避免并发 `fs::write` 同一路径导致截断或读到半成品；**不**解决两路读改写逻辑冲突（仍依赖调用方串行或业务层协调）。
///
/// 若成熟度发生变化，返回 `Some(core)` 供上层派发 `thought-maturity-changed`。
pub fn apply_challenge_pass_blocking(
    canonical_root: &Path,
    args: ApplyChallengePassArgs,
) -> Result<Option<thought_parser::ThoughtMaturityChangedCore>, String> {
    let rel_path = args.rel_path.trim().to_string();
    let joined = join_under_root(canonical_root, &rel_path)?;
    if !is_markdown_path(&joined) {
        return Err("not a markdown file".to_string());
    }
    let canonical_file =
        fs::canonicalize(&joined).map_err(|e| sanitize_io_error(e, "resolving file path"))?;
    if !canonical_file.starts_with(canonical_root) {
        return Err("path escapes root".to_string());
    }
    let content =
        fs::read_to_string(&canonical_file).map_err(|e| sanitize_io_error(e, "reading file"))?;
    let outcome = thought_parser::apply_challenge_pass_to_markdown_vault(
        canonical_root,
        &rel_path,
        &content,
        &args.thought_id,
        args.passed,
    )?;
    if outcome.markdown == content {
        return Ok(None);
    }
    atomic_write_string_in_parent(&canonical_file, &outcome.markdown)?;
    Ok(outcome.maturity_change)
}

// --- LLM：生成挑战问句 ---

/// 与主流式隔离的 system 提示（英文），输出 JSON。
const SYSTEM_CHALLENGE_GENERATE: &str = r#"You design ONE short challenge question to help the user revisit a saved thought from their notes.

Pick the best template kind:
- "compare": contrast two ideas or test whether a distinction still holds in a scenario.
- "apply": ask them to apply the thought to a new concrete situation.
- "critique": challenge an implicit assumption politely.
- "transfer": ask whether an idea from domain A could inform domain B.

Rules:
- The question must be answerable in a few sentences; no multi-part essays.
- If the user message includes a "UI locale" line, write the `question` in that language (English vs Chinese) regardless of excerpt language.
- Otherwise match the thought excerpt language (Chinese excerpt → Chinese question; English → English).
- Respond with ONE JSON object only (no markdown fences, no prose). Keys (camelCase):
  - "question": string (non-empty unless skipped)
  - "templateKind": one of compare | apply | critique | transfer
  - "skipped": boolean — true if the excerpt is too thin or unsafe to challenge; then set question to "".

Example: {"question":"...","templateKind":"apply","skipped":false}"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateJson {
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    template_kind: Option<String>,
    #[serde(default)]
    skipped: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateChallengeQuestionArgs {
    pub thought_excerpt: String,
    pub rel_path: String,
    #[serde(default)]
    pub conversation_query: Option<String>,
    #[serde(default)]
    pub depth_mode: Option<DepthMode>,
    /// 与前端 Knowforge 语言一致：`en` / `zh`（可选，缺省则按摘录语言推断问句语言）
    #[serde(default)]
    pub ui_locale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateChallengeQuestionResponse {
    /// 空表示通道二应静默（跳过展示）
    pub question: String,
    pub template_kind: String,
    /// 使用固定模板或模型明确降级
    pub degraded: bool,
    /// true 时不应展示内联回顾（Ollama 不可用、解析失败、模型主动 skipped）
    pub should_skip: bool,
}

// --- LLM：评估作答 ---

const SYSTEM_CHALLENGE_EVALUATE: &str = r#"You review the user's short answer to a challenge question about their own saved thought.

Decide:
- "passed": true if the answer shows genuine engagement (not empty, not single-word).
- "sloppy": true if the answer is too short, dismissive, or clearly placeholder — do NOT treat as passed even if partially right.

Output ONE JSON object (camelCase keys, no markdown fences):
- "passed": boolean
- "sloppy": boolean
- "commentaryMd": string — brief markdown: affirm what was strong, note gaps, optional new angle. If the user message includes a "UI locale" line, write commentaryMd in that language; otherwise match the user's answer language.
- "templateKind": optional echo of the template used (compare|apply|critique|transfer)

If the answer is empty, return {"passed":false,"sloppy":true,"commentaryMd":"…","templateKind":null} with a gentle nudge in the UI locale when provided, else the answer's language."#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateJson {
    #[serde(default)]
    passed: bool,
    #[serde(default)]
    sloppy: bool,
    #[serde(default)]
    commentary_md: Option<String>,
    #[serde(default)]
    template_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateChallengeAnswerArgs {
    pub question: String,
    pub user_answer: String,
    pub thought_excerpt: String,
    #[serde(default)]
    pub depth_mode: Option<DepthMode>,
    #[serde(default)]
    pub ui_locale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateChallengeAnswerResponse {
    pub passed: bool,
    pub sloppy: bool,
    pub commentary_md: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_kind: Option<String>,
}

/// 通道一/二共用的降级问句（中文，与产品文档一致）
pub const FALLBACK_CHALLENGE_QUESTION_ZH: &str = "你之前写过这个想法，现在还同意这个观点吗？";

pub const FALLBACK_CHALLENGE_QUESTION_EN: &str =
    "You wrote this idea before — do you still agree with it?";

pub(crate) fn ui_locale_is_zh(ui_locale: Option<&str>) -> bool {
    matches!(
        ui_locale.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("zh" | "zh-cn" | "zh-hans" | "zh-hant" | "zh-tw")
    )
}

fn ui_locale_is_en(ui_locale: Option<&str>) -> bool {
    matches!(
        ui_locale.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("en") | Some("en-us") | Some("en-gb")
    )
}

/// 注入用户消息块，约束问句自然语言与 Knowforge 界面一致。
fn generate_ui_locale_paragraph(ui_locale: Option<&str>) -> &'static str {
    if ui_locale_is_zh(ui_locale) {
        "UI locale: Chinese (Simplified). Write the JSON `question` field in natural Chinese (简体中文), even if the excerpt is in another language."
    } else if ui_locale_is_en(ui_locale) {
        "UI locale: English. Write the JSON `question` field in English, even if the excerpt is in another language."
    } else {
        "Language: If no UI locale was specified, match the thought excerpt language for the question."
    }
}

fn evaluate_ui_locale_paragraph(ui_locale: Option<&str>) -> &'static str {
    if ui_locale_is_zh(ui_locale) {
        "UI locale: Chinese (Simplified). Write the JSON `commentaryMd` field in natural Chinese (简体中文)."
    } else if ui_locale_is_en(ui_locale) {
        "UI locale: English. Write the JSON `commentaryMd` field in English."
    } else {
        "Language: Match the user's answer language for commentaryMd when possible."
    }
}

fn extract_json_object_slice(raw: &str) -> Result<&str, String> {
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
    Ok(&s[start..=end])
}

fn resolve_depth_for_challenge(depth: Option<DepthMode>, query_opt: Option<&str>) -> DepthMode {
    match depth {
        None => DepthMode::Medium,
        Some(DepthMode::Auto) => {
            let q = query_opt.unwrap_or("").trim();
            let n = q.chars().count();
            if n <= 40 {
                DepthMode::Shallow
            } else if n <= 200 {
                DepthMode::Medium
            } else {
                DepthMode::Deep
            }
        }
        Some(d) => d,
    }
}

fn depth_tone_line(d: DepthMode) -> &'static str {
    match d {
        DepthMode::Shallow => "Keep the challenge question very short (one sentence).",
        DepthMode::Medium => "Keep the challenge question concise (1-2 sentences).",
        DepthMode::Deep => "You may use a slightly richer challenge question (still under 3 sentences).",
        DepthMode::Auto => "Keep the challenge question concise (1-2 sentences).",
    }
}

fn normalize_template_kind(raw: Option<&str>) -> String {
    let s = raw.unwrap_or("apply").trim().to_ascii_lowercase();
    match s.as_str() {
        "compare" | "comparison" => "compare".to_string(),
        "critique" | "critical" => "critique".to_string(),
        "transfer" | "migration" => "transfer".to_string(),
        "apply" | "application" | _ => "apply".to_string(),
    }
}

/// 生成挑战问句（失败时 `should_skip=true` 供通道二静默）
#[tauri::command]
pub async fn generate_challenge_question(
    workspace: tauri::State<'_, crate::WorkspaceState>,
    args: GenerateChallengeQuestionArgs,
) -> Result<GenerateChallengeQuestionResponse, String> {
    let root = crate::lock_workspace_root(&workspace)?;
    let ai = tauri::async_runtime::spawn_blocking(move || {
        let ai = vault_config::load_ai_config_internal(&root)?;
        Ok::<_, String>(ai)
    })
    .await
    .map_err(|e| e.to_string())??;

    let provider = match create_provider(&ai, None) {
        Ok(p) => p,
        Err(_) => {
            return Ok(GenerateChallengeQuestionResponse {
                question: String::new(),
                template_kind: "apply".to_string(),
                degraded: false,
                should_skip: true,
            });
        }
    };

    let excerpt = args.thought_excerpt.trim();
    if excerpt.chars().count() < 8 {
        return Ok(GenerateChallengeQuestionResponse {
            question: String::new(),
            template_kind: "apply".to_string(),
            degraded: false,
            should_skip: true,
        });
    }

    let depth = resolve_depth_for_challenge(
        args.depth_mode,
        args.conversation_query.as_deref(),
    );
    let tone = depth_tone_line(depth);
    let mut user_block = format!(
        "Source note path (for context only): `{}`\n\nSaved thought excerpt:\n---\n{}\n---\n",
        args.rel_path.trim(),
        excerpt
    );
    if let Some(ref q) = args.conversation_query {
        let t = q.trim();
        if !t.is_empty() {
            user_block.push_str(&format!("\nRecent user query in the chat (optional tie-in):\n---\n{t}\n---\n"));
        }
    }
    user_block.push_str(&format!(
        "\n{}\n\nDepth hint: {tone}",
        generate_ui_locale_paragraph(args.ui_locale.as_deref())
    ));

    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_CHALLENGE_GENERATE.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".into(),
            content: user_block,
            ..Default::default()
        },
    ];

    let overrides = CompletionOverrides {
        temperature: Some((ai.parameters.temperature * 0.85).clamp(0.0, 1.0)),
        ..Default::default()
    };
    let raw = provider.chat_completion(&msgs, Some(&overrides)).await;

    let fallback_q = if ui_locale_is_en(args.ui_locale.as_deref()) {
        FALLBACK_CHALLENGE_QUESTION_EN
    } else {
        FALLBACK_CHALLENGE_QUESTION_ZH
    };

    let raw = match raw {
        Ok(s) => s,
        Err(_) => {
            return Ok(GenerateChallengeQuestionResponse {
                question: fallback_q.to_string(),
                template_kind: "apply".to_string(),
                degraded: true,
                should_skip: false,
            });
        }
    };

    let parsed: GenerateJson = match extract_json_object_slice(&raw)
        .and_then(|slice| serde_json::from_str(slice).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(_) => {
            return Ok(GenerateChallengeQuestionResponse {
                question: fallback_q.to_string(),
                template_kind: "apply".to_string(),
                degraded: true,
                should_skip: false,
            });
        }
    };

    if parsed.skipped {
        return Ok(GenerateChallengeQuestionResponse {
            question: String::new(),
            template_kind: normalize_template_kind(parsed.template_kind.as_deref()),
            degraded: false,
            should_skip: true,
        });
    }

    let q = parsed.question.unwrap_or_default().trim().to_string();
    if q.is_empty() {
        return Ok(GenerateChallengeQuestionResponse {
            question: fallback_q.to_string(),
            template_kind: normalize_template_kind(parsed.template_kind.as_deref()),
            degraded: true,
            should_skip: false,
        });
    }

    Ok(GenerateChallengeQuestionResponse {
        question: q,
        template_kind: normalize_template_kind(parsed.template_kind.as_deref()),
        degraded: false,
        should_skip: false,
    })
}

/// 评估用户作答（仅 Ollama；网络/解析失败时 passed=false、简短降级点评）
#[tauri::command]
pub async fn evaluate_challenge_answer(
    workspace: tauri::State<'_, crate::WorkspaceState>,
    args: EvaluateChallengeAnswerArgs,
) -> Result<EvaluateChallengeAnswerResponse, String> {
    let root = crate::lock_workspace_root(&workspace)?;
    let ai = tauri::async_runtime::spawn_blocking(move || vault_config::load_ai_config_internal(&root))
        .await
        .map_err(|e| e.to_string())??;

    let prefer_zh_copy = !ui_locale_is_en(args.ui_locale.as_deref());

    let empty_commentary = |zh: bool| {
        if zh {
            "暂时无法连接模型完成点评，请稍后在网络正常时再试。"
        } else {
            "Could not reach the model for commentary; please try again later."
        }
    };

    let provider = match create_provider(&ai, None) {
        Ok(p) => p,
        Err(_) => {
            return Ok(EvaluateChallengeAnswerResponse {
                passed: false,
                sloppy: false,
                commentary_md: empty_commentary(prefer_zh_copy).to_string(),
                template_kind: None,
            });
        }
    };

    let depth = resolve_depth_for_challenge(args.depth_mode, None);
    let tone = depth_tone_line(depth);
    let user_block = format!(
        "Challenge question:\n---\n{}\n---\n\nUser answer:\n---\n{}\n---\n\nOriginal thought excerpt:\n---\n{}\n---\n\n{}\n\nReviewer hint: {tone}",
        args.question.trim(),
        args.user_answer.trim(),
        args.thought_excerpt.trim(),
        evaluate_ui_locale_paragraph(args.ui_locale.as_deref())
    );

    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_CHALLENGE_EVALUATE.to_string(),
            ..Default::default()
        },
        LlmChatMessage {
            role: "user".into(),
            content: user_block,
            ..Default::default()
        },
    ];

    let raw = provider.chat_completion(&msgs, None).await;

    let raw = match raw {
        Ok(s) => s,
        Err(_) => {
            return Ok(EvaluateChallengeAnswerResponse {
                passed: false,
                sloppy: false,
                commentary_md: empty_commentary(prefer_zh_copy).to_string(),
                template_kind: None,
            });
        }
    };

    let parse_fail_msg = if prefer_zh_copy {
        "无法解析模型输出，请简要重述你的观点后再试一次。"
    } else {
        "Could not parse the model output. Briefly restate your point and try again."
    };

    let parsed: EvaluateJson = match extract_json_object_slice(&raw)
        .and_then(|slice| serde_json::from_str(slice).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(_) => {
            return Ok(EvaluateChallengeAnswerResponse {
                passed: false,
                sloppy: true,
                commentary_md: parse_fail_msg.to_string(),
                template_kind: None,
            });
        }
    };

    let mut passed = parsed.passed && !parsed.sloppy;
    let sloppy = parsed.sloppy;
    let mut commentary = parsed.commentary_md.unwrap_or_default();
    if commentary.trim().is_empty() {
        commentary = if prefer_zh_copy {
            if passed {
                "不错，继续保持这种反思习惯。".to_string()
            } else if sloppy {
                "这个问题值得多想想，下次再试？".to_string()
            } else {
                "可以再展开一点：试着联系一个具体例子。".to_string()
            }
        } else if passed {
            "Nice — keep building this reflection habit.".to_string()
        } else if sloppy {
            "This one deserves a bit more thought. Want to try again?".to_string()
        } else {
            "Try going one step further: connect it to a concrete example.".to_string()
        };
    }

    if sloppy {
        passed = false;
    }

    Ok(EvaluateChallengeAnswerResponse {
        passed,
        sloppy,
        commentary_md: commentary,
        template_kind: parsed.template_kind,
    })
}

// --- 回顾队列：遗忘曲线 MVP + 日 cap 顺延（`.knowforge/challenge-review-cap-state.json`） ---

/// 排期间隔（天）：第 n 次成功回顾后的下一次间隔取下标 `min(n,4)`（与迭代 4 文档 §5 对齐）。
const REVIEW_INTERVALS_DAYS: &[i64] = &[1, 3, 7, 14, 30];

const CAP_STATE_FILE: &str = ".knowforge/challenge-review-cap-state.json";

/// 无积压：每个本地日历日首次分配时，将超出 cap 的候选顺延到次日再参与排序（不写回笔记 YAML）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReviewCapDiskState {
    #[serde(default)]
    allocation_day: Option<String>,
    /// thoughtId -> 顺延到的日历日（仅当该日 `>` 当前本地日才继续排除）
    #[serde(default)]
    deferred_until: HashMap<String, String>,
}

fn review_cap_state_path(root: &Path) -> PathBuf {
    root.join(CAP_STATE_FILE)
}

fn load_review_cap_state(root: &Path) -> ReviewCapDiskState {
    let path = review_cap_state_path(root);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ReviewCapDiskState::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// 同目录临时文件 + rename 落盘，避免并发 `fs::write` 同一路径导致 JSON 截断/交错损坏。
fn atomic_write_string_in_parent(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "review cap state path has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|e| sanitize_io_error(e, "creating .knowforge"))?;
    let tmp = parent.join(format!(".challenge-review-cap-state.{}.tmp", Uuid::new_v4()));
    fs::write(&tmp, content.as_bytes()).map_err(|e| sanitize_io_error(e, "writing review cap state temp"))?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(sanitize_io_error(e, "finalizing review cap state"));
    }
    Ok(())
}

fn save_review_cap_state(root: &Path, state: &ReviewCapDiskState) -> Result<(), String> {
    let path = review_cap_state_path(root);
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    atomic_write_string_in_parent(&path, &json)
}

/// 已释放的顺延条目从 map 中移除，避免无限增长
fn prune_review_deferred_until(deferred: &mut HashMap<String, String>, today: NaiveDate) {
    deferred.retain(|_, date_str| {
        parse_meta_date(date_str)
            .map(|d| d > today)
            .unwrap_or(false)
    });
}

fn is_thought_deferred_past_today(
    deferred: &HashMap<String, String>,
    thought_id: &str,
    today: NaiveDate,
) -> bool {
    deferred
        .get(thought_id)
        .and_then(|s| parse_meta_date(s))
        .map(|d| d > today)
        .unwrap_or(false)
}

/// 今日已在通道二展示过的 thought，通道一当日队列排除（与迭代 4 文档「对话优先」一致）
fn today_inline_thought_blocklist(
    cognitive: &vault_config::CognitiveConfig,
    today_key: &str,
) -> HashSet<String> {
    cognitive
        .challenge_review_inline_dates
        .by_day
        .get(today_key)
        .map(|d| d.thought_ids_inline.iter().cloned().collect())
        .unwrap_or_default()
}

fn list_review_queue_blocking(canonical_root: &Path) -> Result<ListReviewQueueResponse, String> {
    let (entries, meta) =
        thought_retrieval::enumerate_vault_thought_entries_blocking(canonical_root)?;
    let cognitive = vault_config::load_cognitive_merged(canonical_root)?;
    let cap = cognitive
        .challenge_review_daily_cap_independent
        .max(1)
        .min(30) as usize;

    let today = Local::now().date_naive();
    let today_key = today.format("%Y-%m-%d").to_string();

    let mut due_rows: Vec<(i64, NaiveDate, thought_retrieval::VaultThoughtEntry)> = Vec::new();
    for e in entries.iter().cloned() {
        let Some(anchor) =
            review_anchor_date(&e.created, e.last_reviewed_at.as_deref(), e.challenge_pass_count)
        else {
            continue;
        };
        let Some(next_due) = next_due_after_anchor(anchor, e.challenge_pass_count) else {
            continue;
        };
        if next_due > today {
            continue;
        }
        let overdue_days = (today - next_due).num_days();
        due_rows.push((overdue_days, next_due, e));
    }

    due_rows.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.2.created.cmp(&b.2.created))
            .then_with(|| a.2.rel_path.cmp(&b.2.rel_path))
    });

    let mut cap_state = load_review_cap_state(canonical_root);
    prune_review_deferred_until(&mut cap_state.deferred_until, today);

    let inline_block = today_inline_thought_blocklist(&cognitive, &today_key);

    let mut eligible: Vec<(i64, NaiveDate, thought_retrieval::VaultThoughtEntry)> = Vec::new();
    for row in due_rows.iter().cloned() {
        let tid = row.2.thought_id.as_str();
        if is_thought_deferred_past_today(&cap_state.deferred_until, tid, today) {
            continue;
        }
        if inline_block.contains(tid) {
            continue;
        }
        eligible.push(row);
    }

    let total_due = eligible.len();
    let total_thoughts = entries.len();

    let needs_roll = cap_state.allocation_day.as_deref() != Some(today_key.as_str());
    if needs_roll {
        if let Some(tomorrow) = today.checked_add_signed(Duration::days(1)) {
            let t_str = tomorrow.format("%Y-%m-%d").to_string();
            for i in cap..eligible.len() {
                let tid = eligible[i].2.thought_id.clone();
                cap_state.deferred_until.insert(tid, t_str.clone());
            }
        }
        cap_state.allocation_day = Some(today_key.clone());
        save_review_cap_state(canonical_root, &cap_state)?;
    }

    let items: Vec<ReviewQueueItem> = eligible
        .into_iter()
        .take(cap)
        .map(|(overdue_days, next_due, e)| ReviewQueueItem {
            rel_path: e.rel_path,
            thought_id: e.thought_id,
            excerpt: e.excerpt,
            maturity: e.maturity,
            created: e.created,
            last_reviewed_at: e.last_reviewed_at,
            challenge_pass_count: e.challenge_pass_count,
            next_due_at: next_due.format("%Y-%m-%d").to_string(),
            overdue_days,
            private_omitted: e.private_omitted,
        })
        .collect();

    Ok(ListReviewQueueResponse {
        items,
        total_thoughts,
        total_due,
        meta,
    })
}

/// 将元数据中的日期字符串解析为日历日（`NaiveDate`）。
///
/// **格式优先级**（严格短路，不合并、不猜）：只有上一级解析失败时才试下一级，避免同一串在两种格式下都可解析时的语义漂移。
/// 1. **`YYYY-MM-DD`**：本模块写入的 `deferred_until`、队列展示用 `next_due_at` 等的主格式；整串须恰好为日历日（不得带时间后缀）。
/// 2. **`RFC3339`（ISO-8601 带偏移）**：兼容 `created` / `last_reviewed_at` 等可能落盘的完整时间戳；**仅取日期部分**为 `NaiveDate`（丢弃时刻与时区，与既有 `date_naive()` 行为一致）。
fn parse_meta_date(s: &str) -> Option<NaiveDate> {
    let t = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(d);
    }
    chrono::DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|dt| dt.date_naive())
}

/// 作为间隔起点的锚点日期：已有成功回顾则优先 `last_reviewed_at`，否则 `created`。
fn review_anchor_date(created: &str, last: Option<&str>, pass_count: u32) -> Option<NaiveDate> {
    if pass_count > 0 {
        if let Some(l) = last {
            if let Some(d) = parse_meta_date(l) {
                return Some(d);
            }
        }
    }
    parse_meta_date(created)
}

/// `completed_pass_count` 为当前 `challenge_pass_count`；下一到期日 = 锚点 + 间隔[`min(count,4)`]。
fn next_due_after_anchor(anchor: NaiveDate, completed_pass_count: u32) -> Option<NaiveDate> {
    let idx = (completed_pass_count as usize).min(REVIEW_INTERVALS_DAYS.len() - 1);
    let days = REVIEW_INTERVALS_DAYS[idx];
    anchor.checked_add_signed(Duration::days(days))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueueItem {
    pub rel_path: String,
    pub thought_id: String,
    pub excerpt: String,
    pub maturity: thought_parser::ThoughtMaturity,
    pub created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reviewed_at: Option<String>,
    pub challenge_pass_count: u32,
    /// 下一次应回顾的日历日（`YYYY-MM-DD`）
    pub next_due_at: String,
    /// 已相对 `next_due_at` 过期的日历天数（越大越优先）
    pub overdue_days: i64,
    pub private_omitted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountVaultThoughtsForReviewResponse {
    pub total_thoughts: usize,
    pub meta: thought_retrieval::SearchThoughtMeta,
}

/// 仅统计 Vault 内非临时 thought 条数（供通道二门控，避免重复跑完整排期）。
#[tauri::command]
pub async fn count_vault_thoughts_for_review(
    workspace: tauri::State<'_, crate::WorkspaceState>,
) -> Result<CountVaultThoughtsForReviewResponse, String> {
    let root = crate::lock_workspace_root(&workspace)?;
    let (entries, meta) = tauri::async_runtime::spawn_blocking(move || {
        thought_retrieval::enumerate_vault_thought_entries_blocking(&root)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(CountVaultThoughtsForReviewResponse {
        total_thoughts: entries.len(),
        meta,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListReviewQueueResponse {
    pub items: Vec<ReviewQueueItem>,
    /// Vault 内参与排期的非临时 thought 总数
    pub total_thoughts: usize,
    /// 已到期（含当日）的条数，可大于 `items.len()`（「无积压」仅展示前 3 条）
    pub total_due: usize,
    pub meta: thought_retrieval::SearchThoughtMeta,
}

#[tauri::command]
pub async fn list_review_queue(
    workspace: tauri::State<'_, crate::WorkspaceState>,
) -> Result<ListReviewQueueResponse, String> {
    let root = crate::lock_workspace_root(&workspace)?;
    tauri::async_runtime::spawn_blocking(move || list_review_queue_blocking(&root))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_object_slice_trims_noise() {
        let raw = r#"Sure {"question":"Q","templateKind":"compare","skipped":false} thanks"#;
        let s = extract_json_object_slice(raw).unwrap();
        let g: GenerateJson = serde_json::from_str(s).unwrap();
        assert_eq!(g.question.as_deref(), Some("Q"));
        assert!(!g.skipped);
    }

    #[test]
    fn next_due_first_review_one_day_after_created() {
        let created = "2026-01-01T00:00:00Z";
        let anchor = review_anchor_date(created, None, 0).unwrap();
        assert_eq!(anchor, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        let next = next_due_after_anchor(anchor, 0).unwrap();
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    }

    #[test]
    fn next_due_after_one_pass_uses_three_day_gap() {
        let last = "2026-04-10";
        let anchor = review_anchor_date("2026-01-01T00:00:00Z", Some(last), 1).unwrap();
        assert_eq!(anchor, NaiveDate::from_ymd_opt(2026, 4, 10).unwrap());
        let next = next_due_after_anchor(anchor, 1).unwrap();
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 4, 13).unwrap());
    }

    #[test]
    fn prune_review_deferred_until_drops_released_ids() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
        let mut deferred = HashMap::from([
            ("gone".into(), "2026-04-21".into()),
            ("keep".into(), "2026-04-23".into()),
        ]);
        prune_review_deferred_until(&mut deferred, today);
        assert!(!deferred.contains_key("gone"));
        assert!(deferred.contains_key("keep"));
    }

    #[test]
    fn is_thought_deferred_respects_calendar_string() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 21).unwrap();
        let mut deferred = HashMap::from([("x".into(), "2026-04-22".into())]);
        assert!(is_thought_deferred_past_today(&deferred, "x", today));
        deferred.insert("y".into(), "2026-04-21".into());
        assert!(!is_thought_deferred_past_today(&deferred, "y", today));
    }
}
