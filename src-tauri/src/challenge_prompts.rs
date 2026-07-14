use crate::challenge_feedback::FeedbackStats;
use crate::vault_config::DepthMode;

pub const BASE_SYSTEM_PROMPT: &str = r#"You design ONE short challenge question to help the user revisit a saved thought from their notes.

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

pub const FALLBACK_CHALLENGE_QUESTION_ZH: &str =
    "你之前写过这个想法，现在还同意这个观点吗？";

pub const FALLBACK_CHALLENGE_QUESTION_EN: &str =
    "You wrote this idea before — do you still agree with it?";

pub(crate) fn ui_locale_is_zh(ui_locale: Option<&str>) -> bool {
    matches!(
        ui_locale.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("zh" | "zh-cn" | "zh-hans" | "zh-hant" | "zh-tw")
    )
}

pub(crate) fn ui_locale_is_en(ui_locale: Option<&str>) -> bool {
    matches!(
        ui_locale.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("en") | Some("en-us") | Some("en-gb")
    )
}

pub(crate) fn generate_ui_locale_paragraph(ui_locale: Option<&str>) -> &'static str {
    if ui_locale_is_zh(ui_locale) {
        "UI locale: Chinese (Simplified). Write the JSON `question` field in natural Chinese (简体中文), even if the excerpt is in another language."
    } else if ui_locale_is_en(ui_locale) {
        "UI locale: English. Write the JSON `question` field in English, even if the excerpt is in another language."
    } else {
        "Language: If no UI locale was specified, match the thought excerpt language for the question."
    }
}

pub(crate) fn depth_tone_line(d: DepthMode) -> &'static str {
    match d {
        DepthMode::Shallow => "Keep the challenge question very short (one sentence).",
        DepthMode::Medium => "Keep the challenge question concise (1-2 sentences).",
        DepthMode::Deep => {
            "You may use a slightly richer challenge question (still under 3 sentences)."
        }
        DepthMode::Auto => "Keep the challenge question concise (1-2 sentences).",
    }
}

pub(crate) fn candidate_degraded_question(
    reason: &str,
    _excerpt: &str,
    paired: Option<&str>,
    locale: Option<&str>,
) -> String {
    let is_en = ui_locale_is_en(locale);
    match reason {
        "high_similarity" => {
            if let Some(p) = paired {
                if is_en {
                    format!("Your notes contain similar content in another file ({p}). What's the unique perspective in this paragraph?")
                } else {
                    format!(
                        "你的笔记在另一个文件（{p}）中有类似内容。这段话的独特之处是什么？"
                    )
                }
            } else if is_en {
                "Your notes contain similar paragraphs. What's the key difference between them?"
                    .to_string()
            } else {
                "你的笔记中有多段相似内容，它们的核心区别是什么？".to_string()
            }
        }
        "semantic_isolated" => {
            if is_en {
                "This idea seems isolated from your other notes. What connections can you draw to other topics you've written about?".to_string()
            } else {
                "这个想法和你的其他笔记似乎没有关联。你能找到它与其他主题之间的联系吗？"
                    .to_string()
            }
        }
        "cross_doc_recurrence" => {
            if is_en {
                "A similar concept appears across several of your notes. Has your understanding of it evolved over time?".to_string()
            } else {
                "你在多篇笔记中提到了类似的概念，你对它的理解有变化吗？".to_string()
            }
        }
        _ => {
            if is_en {
                "What's the core insight in this paragraph, and do you still agree with it?"
                    .to_string()
            } else {
                "这段话的核心观点是什么？你现在还同意吗？".to_string()
            }
        }
    }
}

pub(crate) fn normalize_template_kind(raw: Option<&str>) -> String {
    let s = raw.unwrap_or("apply").trim().to_ascii_lowercase();
    match s.as_str() {
        "compare" | "comparison" => "compare".to_string(),
        "critique" | "critical" => "critique".to_string(),
        "transfer" | "migration" => "transfer".to_string(),
        "apply" | "application" | _ => "apply".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Dynamic system prompt construction
// ---------------------------------------------------------------------------

const WEIGHT_MIN_SAMPLES: usize = 10;
const WEIGHT_HIGH_RATE: f64 = 0.7;
const WEIGHT_LOW_RATE: f64 = 0.4;

const ISSUE_THRESHOLD: usize = 5;
const ISSUE_DUPLICATE_THRESHOLD: usize = 3;

fn build_template_weight_hint(stats: &FeedbackStats) -> Option<String> {
    let mut lines = Vec::new();
    for ts in &stats.by_template {
        if ts.total < WEIGHT_MIN_SAMPLES {
            continue;
        }
        if ts.helpful_rate < WEIGHT_LOW_RATE {
            lines.push(format!(
                "- Avoid the \"{}\" template — users find it unhelpful.",
                ts.template
            ));
        } else if ts.helpful_rate > WEIGHT_HIGH_RATE {
            lines.push(format!(
                "- The \"{}\" template works well — consider using it.",
                ts.template
            ));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!(
            "Template preferences based on user feedback:\n{}",
            lines.join("\n")
        ))
    }
}

fn build_issue_hint(stats: &FeedbackStats) -> Option<String> {
    let mut lines = Vec::new();
    for issue in &stats.common_issues {
        match issue.reason.as_str() {
            "too_easy" if issue.count >= ISSUE_THRESHOLD => {
                lines.push(
                    "- Ask at a deeper level; avoid surface-level recall questions.".to_string(),
                );
            }
            "irrelevant" if issue.count >= ISSUE_THRESHOLD => {
                lines.push(
                    "- The question MUST directly reference specific content from the excerpt."
                        .to_string(),
                );
            }
            "too_vague" if issue.count >= ISSUE_THRESHOLD => {
                lines.push(
                    "- Be specific; reference exact concepts, terms, or claims from the text."
                        .to_string(),
                );
            }
            "duplicate" if issue.count >= ISSUE_DUPLICATE_THRESHOLD => {
                lines.push(
                    "- Vary your question style across template kinds.".to_string(),
                );
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!(
            "Additional rules based on past feedback:\n{}",
            lines.join("\n")
        ))
    }
}

pub fn build_system_prompt(stats: Option<&FeedbackStats>) -> String {
    let mut prompt = BASE_SYSTEM_PROMPT.to_string();
    if let Some(s) = stats {
        if let Some(hint) = build_template_weight_hint(s) {
            prompt.push_str("\n\n");
            prompt.push_str(&hint);
        }
        if let Some(hint) = build_issue_hint(s) {
            prompt.push_str("\n\n");
            prompt.push_str(&hint);
        }
    }
    prompt
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge_feedback::{IssueCount, TemplateStats};

    #[test]
    fn normalize_template_kind_aliases() {
        assert_eq!(normalize_template_kind(Some("compare")), "compare");
        assert_eq!(normalize_template_kind(Some("comparison")), "compare");
        assert_eq!(normalize_template_kind(Some("critique")), "critique");
        assert_eq!(normalize_template_kind(Some("critical")), "critique");
        assert_eq!(normalize_template_kind(Some("transfer")), "transfer");
        assert_eq!(normalize_template_kind(Some("migration")), "transfer");
        assert_eq!(normalize_template_kind(Some("apply")), "apply");
        assert_eq!(normalize_template_kind(Some("application")), "apply");
        assert_eq!(normalize_template_kind(Some("unknown")), "apply");
        assert_eq!(normalize_template_kind(None), "apply");
    }

    fn make_stats(
        templates: Vec<(&str, usize, usize)>,
        issues: Vec<(&str, usize)>,
    ) -> FeedbackStats {
        let by_template: Vec<TemplateStats> = templates
            .into_iter()
            .map(|(name, helpful, not_helpful)| {
                let total = helpful + not_helpful;
                TemplateStats {
                    template: name.to_string(),
                    total,
                    helpful,
                    not_helpful,
                    helpful_rate: if total > 0 {
                        helpful as f64 / total as f64
                    } else {
                        0.0
                    },
                }
            })
            .collect();
        let common_issues = issues
            .into_iter()
            .map(|(reason, count)| IssueCount {
                reason: reason.to_string(),
                count,
            })
            .collect();
        let helpful_count: usize = by_template.iter().map(|t: &TemplateStats| t.helpful).sum();
        let not_helpful_count: usize = by_template
            .iter()
            .map(|t: &TemplateStats| t.not_helpful)
            .sum();
        let total = helpful_count + not_helpful_count;
        FeedbackStats {
            total_ratings: total,
            helpful_count,
            not_helpful_count,
            helpful_rate: if total > 0 {
                helpful_count as f64 / total as f64
            } else {
                0.0
            },
            by_template,
            common_issues,
        }
    }

    #[test]
    fn build_template_weight_hint_none_when_insufficient_data() {
        let stats = make_stats(vec![("apply", 3, 2), ("compare", 1, 1)], vec![]);
        assert!(build_template_weight_hint(&stats).is_none());
    }

    #[test]
    fn build_template_weight_hint_surfaces_extreme_templates() {
        let stats = make_stats(
            vec![
                ("apply", 9, 1),     // 10 samples, 0.9 rate → prefer
                ("critique", 2, 10), // 12 samples, 0.17 rate → avoid
                ("compare", 3, 2),   // 5 samples → skip (below threshold)
            ],
            vec![],
        );
        let hint = build_template_weight_hint(&stats).unwrap();
        assert!(hint.contains("\"apply\" template works well"));
        assert!(hint.contains("Avoid the \"critique\""));
        assert!(!hint.contains("compare"));
    }

    #[test]
    fn build_issue_hint_none_when_no_issues() {
        let stats = make_stats(vec![], vec![]);
        assert!(build_issue_hint(&stats).is_none());
    }

    #[test]
    fn build_issue_hint_triggers_on_threshold() {
        let stats = make_stats(
            vec![],
            vec![
                ("too_easy", 6),
                ("too_vague", 5),
                ("irrelevant", 4), // below threshold
                ("duplicate", 3),
            ],
        );
        let hint = build_issue_hint(&stats).unwrap();
        assert!(hint.contains("deeper level"));
        assert!(hint.contains("Be specific"));
        assert!(!hint.contains("MUST directly reference")); // irrelevant below threshold
        assert!(hint.contains("Vary your question style"));
    }

    #[test]
    fn build_system_prompt_base_only_without_stats() {
        let prompt = build_system_prompt(None);
        assert_eq!(prompt, BASE_SYSTEM_PROMPT);
    }

    #[test]
    fn build_system_prompt_appends_hints() {
        let stats = make_stats(
            vec![("apply", 9, 1)], // 10 samples, high rate
            vec![("too_easy", 7)],
        );
        let prompt = build_system_prompt(Some(&stats));
        assert!(prompt.starts_with(BASE_SYSTEM_PROMPT));
        assert!(prompt.contains("Template preferences"));
        assert!(prompt.contains("Additional rules"));
    }
}
