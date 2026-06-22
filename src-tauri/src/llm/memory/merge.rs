use chrono::{NaiveDate, Utc};
use std::collections::HashMap;

use super::types::*;

pub(super) const MAX_KNOWLEDGE_DOMAINS: usize = 15;
pub(super) const MAX_CORRECTIONS: usize = 20;
pub(super) const MAX_SESSIONS: usize = 10;
pub(super) const MAX_ARCHIVES_PER_REFLECTION: usize = 3;
pub(super) const SUPERSEDED_RETENTION_DAYS: i64 = 90;
// ── Confidence decay ──

impl AgentMemory {
    pub fn apply_confidence_decay(&mut self) {
        let today = Utc::now().date_naive();
        for domain in &mut self.knowledge_domains {
            if domain.archived {
                continue;
            }
            if let Ok(last) = NaiveDate::parse_from_str(&domain.last_evidence, "%Y-%m-%d") {
                let days = (today - last).num_days();
                if days > 30 {
                    let periods = (days / 30) as f64;
                    let decay_rate =
                        0.1 / (domain.evidence_count as f64 + 1.0).ln().max(1.0);
                    domain.confidence -= decay_rate * periods;
                    domain.confidence = domain.confidence.max(0.0);
                    if domain.confidence < 0.3 {
                        domain.archived = true;
                    }
                }
            }
        }
    }

    pub fn expire_superseded_corrections(&mut self) {
        let cutoff = (Utc::now() - chrono::Duration::days(SUPERSEDED_RETENTION_DAYS))
            .format("%Y-%m-%d")
            .to_string();
        self.corrections.retain(|c| {
            c.is_active()
                || c.superseded_at
                    .as_ref()
                    .map(|d| d.as_str() > cutoff.as_str())
                    .unwrap_or(true)
        });
    }

    pub fn expire_pending_styles(&mut self) {
        const PENDING_TTL_DAYS: i64 = 30;
        let cutoff = Utc::now() - chrono::Duration::days(PENDING_TTL_DAYS);
        self.interaction_style.pending.retain(|_, entry| {
            chrono::DateTime::parse_from_rfc3339(&entry.observed_at)
                .map(|dt| dt > cutoff)
                .unwrap_or(true)
        });
    }
}

fn pending_values_match(pending: &str, new: &str) -> bool {
    let a = pending.to_lowercase();
    let b = new.to_lowercase();
    a == b || a.contains(&b) || b.contains(&a)
}
// ── Merge algorithm (Spec 3) ──

pub(super) fn depth_rank(depth: &str) -> u32 {
    match depth {
        "curious" => 1,
        "learning" => 2,
        "practitioner" => 3,
        "expert" => 4,
        _ => 0,
    }
}

impl AgentMemory {
    pub fn merge_user_model(&mut self, update: UserModelUpdate) {
        self.merge_knowledge_domains(update.knowledge_domains);
        self.merge_interaction_style(update.interaction_style_updates);
        self.merge_corrections(update.new_corrections, update.remove_corrections);
        self.merge_session(
            update.session_summary,
            update.session_domains_touched,
            update.follow_up,
        );
        self.last_updated = Some(Utc::now().to_rfc3339());
    }

    fn merge_knowledge_domains(&mut self, updates: Vec<DomainUpdate>) {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        for new in updates {
            if let Some(existing) = self
                .knowledge_domains
                .iter_mut()
                .find(|d| d.domain.to_lowercase() == new.domain.to_lowercase())
            {
                existing.evidence_count += 1;
                existing.last_evidence = today.clone();
                existing.archived = false;

                if depth_rank(&new.depth) > depth_rank(&existing.depth)
                    && new.confidence >= 0.7
                {
                    existing.depth = new.depth;
                }

                existing.confidence =
                    (existing.confidence + (1.0 - existing.confidence) * 0.15).min(0.95);

                if new.current_focus.is_some() {
                    existing.current_focus = new.current_focus;
                }
                if new.motivation.is_some() {
                    existing.motivation = new.motivation;
                }
            } else {
                self.knowledge_domains.push(KnowledgeDomain {
                    domain: new.domain,
                    depth: new.depth,
                    current_focus: new.current_focus,
                    motivation: new.motivation,
                    confidence: new.confidence,
                    last_evidence: today.clone(),
                    evidence_count: 1,
                    archived: false,
                });
            }
        }

        if self.knowledge_domains.len() > MAX_KNOWLEDGE_DOMAINS {
            self.knowledge_domains
                .sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
            self.knowledge_domains.truncate(MAX_KNOWLEDGE_DOMAINS);
        }
    }

    fn merge_interaction_style(&mut self, updates: HashMap<String, Option<String>>) {
        for (key, value) in updates {
            let value = match value {
                Some(v) => v,
                None => continue,
            };

            let current = match key.as_str() {
                "detail_preference" => &self.interaction_style.detail_preference,
                "explanation_style" => &self.interaction_style.explanation_style,
                "challenge_tolerance" => &self.interaction_style.challenge_tolerance,
                "format" => &self.interaction_style.format,
                _ => &None,
            };
            if current.as_deref() == Some(&value) {
                continue;
            }

            let matches_pending = self
                .interaction_style
                .pending
                .get(&key)
                .map(|p| pending_values_match(&p.value, &value))
                .unwrap_or(false);

            if matches_pending {
                match key.as_str() {
                    "detail_preference" => {
                        self.interaction_style.detail_preference = Some(value.clone())
                    }
                    "explanation_style" => {
                        self.interaction_style.explanation_style = Some(value.clone())
                    }
                    "challenge_tolerance" => {
                        self.interaction_style.challenge_tolerance = Some(value.clone())
                    }
                    "format" => self.interaction_style.format = Some(value.clone()),
                    _ => {}
                }
                self.interaction_style.pending.remove(&key);
            } else {
                self.interaction_style
                    .pending
                    .insert(key, PendingStyleEntry::new(value));
            }
        }
    }

    fn merge_corrections(
        &mut self,
        new_corrections: Vec<NewCorrection>,
        remove_corrections: Vec<String>,
    ) {
        let today = Utc::now().format("%Y-%m-%d").to_string();

        for rule_text in &remove_corrections {
            for c in self.corrections.iter_mut() {
                if c.is_active() && c.rule == *rule_text {
                    c.superseded_by = Some("user_forget".to_string());
                    c.superseded_at = Some(today.clone());
                }
            }
        }

        for nc in new_corrections {
            if let Some(existing) = self
                .corrections
                .iter_mut()
                .find(|c| c.is_active() && c.rule == nc.rule)
            {
                existing.date = today.clone();
                existing.reason = nc.reason;
            } else {
                self.corrections.push(MemoryCorrection {
                    rule: nc.rule,
                    reason: nc.reason,
                    date: today.clone(),
                    source: "explicit".to_string(),
                    superseded_by: None,
                    superseded_at: None,
                });
            }
        }

        let active_count = self.corrections.iter().filter(|c| c.is_active()).count();
        if active_count > MAX_CORRECTIONS {
            self.corrections
                .sort_by(|a, b| a.date.cmp(&b.date));
            let mut to_remove = active_count - MAX_CORRECTIONS;
            for c in self.corrections.iter_mut() {
                if to_remove == 0 {
                    break;
                }
                if c.is_active() {
                    c.superseded_by = Some("capacity".to_string());
                    c.superseded_at = Some(today.clone());
                    to_remove -= 1;
                }
            }
        }
    }

    fn merge_session(
        &mut self,
        summary: Option<String>,
        domains_touched: Vec<String>,
        follow_up: Option<String>,
    ) {
        if let Some(summary) = summary {
            self.sessions.push(MemorySession {
                date: Utc::now().to_rfc3339(),
                summary,
                domains_touched,
                follow_up,
            });

            if self.sessions.len() > MAX_SESSIONS {
                let excess = self.sessions.len() - MAX_SESSIONS;
                self.sessions.drain(0..excess);
            }
        }
    }
}
