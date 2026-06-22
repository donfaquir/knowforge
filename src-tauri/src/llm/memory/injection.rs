use super::types::*;
use super::workspace::is_cjk;

pub(super) const INJECTION_RECENT_SESSIONS: usize = 3;
pub(super) const MAX_INJECTION_TOKENS: usize = 800;
// ── Injection formatting ──

impl AgentMemory {
    pub fn format_for_injection(&self) -> Option<String> {
        let high_domains: Vec<&KnowledgeDomain> = self
            .knowledge_domains
            .iter()
            .filter(|d| !d.archived && d.confidence >= 0.5)
            .collect();
        let low_domains: Vec<&KnowledgeDomain> = self
            .knowledge_domains
            .iter()
            .filter(|d| !d.archived && d.confidence >= 0.3 && d.confidence < 0.5)
            .collect();
        let has_style = self.interaction_style.detail_preference.is_some()
            || self.interaction_style.explanation_style.is_some()
            || self.interaction_style.challenge_tolerance.is_some()
            || self.interaction_style.format.is_some()
            || !self.interaction_style.language.is_empty();
        let has_active_corrections = self.corrections.iter().any(|c| c.is_active());
        let has_content = !high_domains.is_empty()
            || !low_domains.is_empty()
            || has_style
            || has_active_corrections
            || !self.workspace.frequent_paths.is_empty()
            || !self.sessions.is_empty();

        if !has_content {
            return None;
        }

        let header = "# User Model (accumulated across sessions)".to_string();
        let footer = "Adapt your responses to this user's knowledge level and style.\n\
             Do NOT mention this model unless asked.\n\
             Current request overrides any remembered preference."
            .to_string();

        // ## You — knowledge domains + style (core: style never trimmed)
        let you_section = if !high_domains.is_empty()
            || !low_domains.is_empty()
            || has_style
        {
            let mut s = String::from("## You");

            if !high_domains.is_empty() {
                s.push_str("\n- Knowledge: ");
                let strs: Vec<String> =
                    high_domains.iter().map(|d| format_domain_entry(d)).collect();
                s.push_str(&strs.join("; "));
            }

            if !low_domains.is_empty() {
                let names: Vec<&str> = low_domains.iter().map(|d| d.domain.as_str()).collect();
                s.push_str(&format!("\n- Also some interest in: {}", names.join(", ")));
            }

            if has_style {
                let mut style_parts = Vec::new();
                if let Some(ref dp) = self.interaction_style.detail_preference {
                    style_parts.push(dp.clone());
                }
                if let Some(ref es) = self.interaction_style.explanation_style {
                    style_parts.push(format!("prefers {es}"));
                }
                if let Some(ref ct) = self.interaction_style.challenge_tolerance {
                    style_parts.push(format!("{ct} challenge tolerance"));
                }
                if let Some(ref fmt) = self.interaction_style.format {
                    style_parts.push(format!("format: {fmt}"));
                }
                if !style_parts.is_empty() {
                    s.push_str(&format!("\n- Style: {}", style_parts.join(", ")));
                }
                if !self.interaction_style.language.is_empty() {
                    let lang_parts: Vec<String> = self
                        .interaction_style
                        .language
                        .iter()
                        .map(|(k, v)| format!("{k} in {v}"))
                        .collect();
                    s.push_str(&format!("\n- Language: {}", lang_parts.join(", ")));
                }
            }

            Some(s)
        } else {
            None
        };

        // ## You without low-confidence domain summary
        let you_section_trimmed = if !high_domains.is_empty() || has_style {
            let mut s = String::from("## You");

            if !high_domains.is_empty() {
                s.push_str("\n- Knowledge: ");
                let strs: Vec<String> =
                    high_domains.iter().map(|d| format_domain_entry(d)).collect();
                s.push_str(&strs.join("; "));
            }

            if has_style {
                let mut style_parts = Vec::new();
                if let Some(ref dp) = self.interaction_style.detail_preference {
                    style_parts.push(dp.clone());
                }
                if let Some(ref es) = self.interaction_style.explanation_style {
                    style_parts.push(format!("prefers {es}"));
                }
                if let Some(ref ct) = self.interaction_style.challenge_tolerance {
                    style_parts.push(format!("{ct} challenge tolerance"));
                }
                if let Some(ref fmt) = self.interaction_style.format {
                    style_parts.push(format!("format: {fmt}"));
                }
                if !style_parts.is_empty() {
                    s.push_str(&format!("\n- Style: {}", style_parts.join(", ")));
                }
                if !self.interaction_style.language.is_empty() {
                    let lang_parts: Vec<String> = self
                        .interaction_style
                        .language
                        .iter()
                        .map(|(k, v)| format!("{k} in {v}"))
                        .collect();
                    s.push_str(&format!("\n- Language: {}", lang_parts.join(", ")));
                }
            }

            Some(s)
        } else {
            None
        };

        // ## Workspace
        let workspace_section = if !self.workspace.frequent_paths.is_empty()
            || !self.workspace.language_distribution.is_empty()
            || !self.workspace.tag_vocabulary.is_empty()
        {
            let mut ws = String::from("## Workspace");
            if !self.workspace.language_distribution.is_empty() {
                let dist: Vec<String> = self
                    .workspace
                    .language_distribution
                    .iter()
                    .map(|(k, v)| format!("{}% {}", (v * 100.0).round(), k.to_uppercase()))
                    .collect();
                ws.push_str(&format!("\n- Notes: {}", dist.join(" / ")));
            }
            if !self.workspace.frequent_paths.is_empty() {
                let paths: Vec<String> = self
                    .workspace
                    .frequent_paths
                    .iter()
                    .take(5)
                    .map(|fp| format!("{} ({})", fp.path, fp.description))
                    .collect();
                ws.push_str(&format!("\n- Active areas: {}", paths.join(", ")));
            }
            if !self.workspace.topics.is_empty() {
                ws.push_str(&format!(
                    "\n- Topics: {}",
                    self.workspace.topics.join(", ")
                ));
            }
            if !self.workspace.tag_vocabulary.is_empty() {
                let tags: Vec<&str> =
                    self.workspace.tag_vocabulary.iter().take(20).map(|s| s.as_str()).collect();
                ws.push_str(&format!("\n- Tags: {}", tags.join(", ")));
            }
            Some(ws)
        } else {
            None
        };

        // ## Rules (never trimmed, only active corrections)
        let rules_section = if has_active_corrections {
            let mut rules = String::from("## Rules");
            for c in self.corrections.iter().filter(|c| c.is_active()) {
                rules.push_str(&format!("\n- {} — {}", c.rule, c.reason));
            }
            Some(rules)
        } else {
            None
        };

        // Build sessions section with a given limit
        let build_sessions = |limit: usize| -> Option<String> {
            if self.sessions.is_empty() || limit == 0 {
                return None;
            }
            let mut recent = String::from("## Recent");
            let start = self.sessions.len().saturating_sub(limit);
            for s in &self.sessions[start..] {
                let date_short = s.date.get(..10).unwrap_or(&s.date);
                let follow = s
                    .follow_up
                    .as_deref()
                    .map(|f| format!(" → next: {f}"))
                    .unwrap_or_default();
                recent.push_str(&format!("\n- [{}] {}{}", date_short, s.summary, follow));
            }
            Some(recent)
        };

        let assemble =
            |you: &Option<String>, sessions: &Option<String>| -> String {
                let mut parts = vec![header.clone()];
                if let Some(y) = you {
                    parts.push(y.clone());
                }
                if let Some(ref w) = workspace_section {
                    parts.push(w.clone());
                }
                if let Some(ref r) = rules_section {
                    parts.push(r.clone());
                }
                if let Some(s) = sessions {
                    parts.push(s.clone());
                }
                parts.push(footer.clone());
                parts.join("\n\n")
            };

        // Try full content first
        let sessions_full = build_sessions(INJECTION_RECENT_SESSIONS);
        let text = assemble(&you_section, &sessions_full);
        if estimate_tokens(&text) <= MAX_INJECTION_TOKENS {
            return Some(text);
        }

        // Trim 1: sessions → 1
        let sessions_short = build_sessions(1);
        let text = assemble(&you_section, &sessions_short);
        if estimate_tokens(&text) <= MAX_INJECTION_TOKENS {
            return Some(text);
        }

        // Trim 2: drop low-confidence domain summary
        let text = assemble(&you_section_trimmed, &sessions_short);
        if estimate_tokens(&text) <= MAX_INJECTION_TOKENS {
            return Some(text);
        }

        // Trim 3: drop sessions entirely
        let text = assemble(&you_section_trimmed, &None);
        Some(text)
    }
}

pub(super) fn format_domain_entry(d: &KnowledgeDomain) -> String {
    let qualifier = if d.confidence >= 0.8 {
        String::new()
    } else if d.confidence >= 0.5 {
        "likely ".to_string()
    } else {
        "possibly ".to_string()
    };

    let mut entry = format!("{} ({}{}", d.domain, qualifier, d.depth);

    if let Some(ref focus) = d.current_focus {
        entry.push_str(&format!(", focused on {focus}"));
    }
    if let Some(ref motivation) = d.motivation {
        entry.push_str(&format!(" — {motivation}"));
    }
    entry.push(')');
    entry
}

pub(super) fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for c in text.chars() {
        if is_cjk(c) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other / 3
}
