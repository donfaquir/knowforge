use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::provider::{CompletionOverrides, LlmProvider};
use super::LlmChatMessage;

// ── Capacity constants ──

const MAX_KNOWLEDGE_DOMAINS: usize = 15;
const MAX_CORRECTIONS: usize = 20;
const MAX_SESSIONS: usize = 10;
const MAX_FREQUENT_PATHS: usize = 15;
const MAX_TOPICS: usize = 20;
const INJECTION_RECENT_SESSIONS: usize = 3;

const MEMORY_FILE: &str = "agent_memory.json";
const SNAPSHOT_FILE: &str = "agent_memory.snapshot.json";
const PENDING_FILE: &str = "pending_proposals.json";
const KNOWFORGE_DIR: &str = ".knowforge";
const WORKSPACE_STALENESS_DAYS: i64 = 7;
const PROPOSAL_EXPIRY_DAYS: i64 = 7;

// ── Core user model ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMemory {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default)]
    pub workspace: MemoryWorkspace,
    #[serde(default)]
    pub knowledge_domains: Vec<KnowledgeDomain>,
    #[serde(default)]
    pub interaction_style: InteractionStyle,
    #[serde(default)]
    pub corrections: Vec<MemoryCorrection>,
    #[serde(default)]
    pub sessions: Vec<MemorySession>,
}

impl Default for AgentMemory {
    fn default() -> Self {
        Self {
            version: default_version(),
            last_updated: None,
            workspace: MemoryWorkspace::default(),
            knowledge_domains: Vec::new(),
            interaction_style: InteractionStyle::default(),
            corrections: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

fn default_version() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryWorkspace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub language_distribution: HashMap<String, f64>,
    #[serde(default)]
    pub frequent_paths: Vec<FrequentPath>,
    #[serde(default)]
    pub tag_vocabulary: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrequentPath {
    pub path: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeDomain {
    pub domain: String,
    pub depth: String,
    pub current_focus: Option<String>,
    pub motivation: Option<String>,
    pub confidence: f64,
    pub last_evidence: String,
    pub evidence_count: u32,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_preference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_tolerance: Option<String>,
    #[serde(default)]
    pub language: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub pending: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCorrection {
    pub rule: String,
    pub reason: String,
    pub date: String,
    #[serde(default = "default_explicit")]
    pub source: String,
}

fn default_explicit() -> String {
    "explicit".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemorySession {
    pub date: String,
    pub summary: String,
    #[serde(default)]
    pub domains_touched: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<String>,
}

// ── LLM extraction output ──

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserModelUpdate {
    #[serde(default)]
    pub knowledge_domains: Vec<DomainUpdate>,
    #[serde(default)]
    pub interaction_style_updates: HashMap<String, Option<String>>,
    #[serde(default)]
    pub new_corrections: Vec<NewCorrection>,
    #[serde(default)]
    pub remove_corrections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_summary: Option<String>,
    #[serde(default)]
    pub session_domains_touched: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainUpdate {
    #[serde(alias = "name")]
    pub domain: String,
    #[serde(alias = "level")]
    pub depth: String,
    #[serde(alias = "focus")]
    pub current_focus: Option<String>,
    pub motivation: Option<String>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

fn default_confidence() -> f64 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCorrection {
    #[serde(alias = "instruction")]
    pub rule: String,
    pub reason: String,
}

// ── Reflection proposals (Spec 8) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposal {
    #[serde(default)]
    pub id: String,
    pub action: ProposalAction,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default)]
    pub content: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAction {
    Add,
    Update,
    Archive,
    Merge,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposalBatch {
    pub session_id: String,
    pub proposals: Vec<MemoryProposal>,
    pub created_at: String,
}

const MAX_ARCHIVES_PER_REFLECTION: usize = 3;

fn generate_proposal_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("mp-{ts:x}")
}

pub fn should_reflect(messages: &[LlmChatMessage], memory: &AgentMemory) -> bool {
    let user_msg_count = messages.iter().filter(|m| m.role == "user").count();
    if user_msg_count < 3 {
        return false;
    }
    let has_existing = !memory.knowledge_domains.is_empty()
        || !memory.corrections.is_empty()
        || !memory.sessions.is_empty();
    has_existing
}

pub fn apply_single_proposal(
    memory: &mut AgentMemory,
    proposal: &MemoryProposal,
) -> Result<(), String> {
    match proposal.action {
        ProposalAction::Add => match proposal.category.as_str() {
            "knowledge_domain" => {
                let domain: DomainUpdate = serde_json::from_value(proposal.content.clone())
                    .map_err(|e| format!("Invalid domain content: {e}"))?;
                let update = UserModelUpdate {
                    knowledge_domains: vec![domain],
                    ..Default::default()
                };
                memory.merge_user_model(update);
            }
            "correction" => {
                let corr: NewCorrection = serde_json::from_value(proposal.content.clone())
                    .map_err(|e| format!("Invalid correction content: {e}"))?;
                let update = UserModelUpdate {
                    new_corrections: vec![corr],
                    ..Default::default()
                };
                memory.merge_user_model(update);
            }
            _ => {}
        },
        ProposalAction::Update => {
            if let Some(ref target) = proposal.target {
                if proposal.category == "knowledge_domain" {
                    if let Some(d) = memory
                        .knowledge_domains
                        .iter_mut()
                        .find(|d| d.domain == *target)
                    {
                        if let Ok(upd) =
                            serde_json::from_value::<serde_json::Value>(proposal.content.clone())
                        {
                            if let Some(f) = upd.get("current_focus").and_then(|v| v.as_str()) {
                                d.current_focus = Some(f.to_string());
                            }
                            if let Some(dep) = upd.get("depth").and_then(|v| v.as_str()) {
                                d.depth = dep.to_string();
                            }
                        }
                    }
                }
            }
        }
        ProposalAction::Archive => {
            if let Some(ref target) = proposal.target {
                if let Some(d) = memory
                    .knowledge_domains
                    .iter_mut()
                    .find(|d| d.domain == *target)
                {
                    d.archived = true;
                }
            }
        }
        ProposalAction::Merge => {
            if let Some(ref target) = proposal.target {
                if let Some(d) = memory
                    .knowledge_domains
                    .iter_mut()
                    .find(|d| d.domain == *target)
                {
                    d.archived = true;
                }
            }
            let add_proposal = MemoryProposal {
                action: ProposalAction::Add,
                ..proposal.clone()
            };
            apply_single_proposal(memory, &add_proposal)?;
        }
        ProposalAction::Skip => {}
    }
    Ok(())
}

// ── Load / Save ──

impl AgentMemory {
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(KNOWFORGE_DIR).join(MEMORY_FILE);
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                eprintln!("[memory] Failed to parse agent_memory.json: {e}");
                Self::default()
            }),
            Err(e) => {
                eprintln!("[memory] Failed to read agent_memory.json: {e}");
                Self::default()
            }
        }
    }

    pub fn save(&self, workspace_root: &Path) -> Result<(), String> {
        let dir = workspace_root.join(KNOWFORGE_DIR);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create .knowforge dir: {e}"))?;
        let path = dir.join(MEMORY_FILE);
        let tmp = dir.join(format!("{MEMORY_FILE}.tmp"));
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize memory: {e}"))?;
        std::fs::write(&tmp, format!("{json}\n"))
            .map_err(|e| format!("Failed to write temp memory: {e}"))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("Failed to finalize memory: {e}"))?;
        Ok(())
    }
}

// ── Workspace observation (no LLM) ──

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{3040}'..='\u{309F}'
        | '\u{30A0}'..='\u{30FF}'
        | '\u{AC00}'..='\u{D7AF}'
    )
}

pub fn observe_workspace(
    note_paths: &[String],
    _graph_summary: Option<&str>,
) -> MemoryWorkspace {
    if note_paths.is_empty() {
        return MemoryWorkspace {
            updated_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        };
    }

    // 1. Language distribution: count CJK vs ASCII in filenames
    let mut cjk_count: usize = 0;
    let mut ascii_count: usize = 0;
    for path in note_paths {
        let filename = path.rsplit('/').next().unwrap_or(path);
        let stem = filename.strip_suffix(".md").unwrap_or(filename);
        for c in stem.chars() {
            if is_cjk(c) {
                cjk_count += 1;
            } else if c.is_ascii_alphanumeric() {
                ascii_count += 1;
            }
        }
    }
    let total_chars = (cjk_count + ascii_count).max(1);
    let mut language_distribution = HashMap::new();
    let zh_ratio = cjk_count as f64 / total_chars as f64;
    let en_ratio = ascii_count as f64 / total_chars as f64;
    if zh_ratio > 0.01 {
        language_distribution.insert("zh".to_string(), (zh_ratio * 100.0).round() / 100.0);
    }
    if en_ratio > 0.01 {
        language_distribution.insert("en".to_string(), (en_ratio * 100.0).round() / 100.0);
    }

    // 2. Frequent paths: count notes per parent directory
    let mut dir_counts: HashMap<String, usize> = HashMap::new();
    for path in note_paths {
        let parent = match path.rfind('/') {
            Some(idx) => &path[..idx + 1],
            None => "",
        };
        if !parent.is_empty() {
            *dir_counts.entry(parent.to_string()).or_default() += 1;
        }
    }
    let mut dir_pairs: Vec<(String, usize)> = dir_counts.into_iter().collect();
    dir_pairs.sort_by(|a, b| b.1.cmp(&a.1));
    dir_pairs.truncate(MAX_FREQUENT_PATHS);
    let frequent_paths: Vec<FrequentPath> = dir_pairs
        .into_iter()
        .map(|(path, count)| FrequentPath {
            path: path.clone(),
            description: format!("{count} notes"),
        })
        .collect();

    // 3. Tag vocabulary: not available from note.list (only paths)
    let tag_vocabulary = Vec::new();

    // 4. Topics: derived from directory names as a heuristic
    let mut topics: Vec<String> = frequent_paths
        .iter()
        .take(MAX_TOPICS)
        .filter_map(|fp| {
            let name = fp.path.trim_end_matches('/');
            let name = name.rsplit('/').next().unwrap_or(name);
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();
    topics.truncate(MAX_TOPICS);

    MemoryWorkspace {
        updated_at: Some(Utc::now().to_rfc3339()),
        language_distribution,
        frequent_paths,
        tag_vocabulary,
        topics,
    }
}

fn is_workspace_stale(updated_at: &Option<String>) -> bool {
    let ts = match updated_at {
        Some(s) => s,
        None => return true,
    };
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => (Utc::now() - dt.with_timezone(&Utc)).num_days() >= WORKSPACE_STALENESS_DAYS,
        Err(_) => true,
    }
}

fn scan_md_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    scan_md_recursive(root, root, &mut paths);
    paths
}

fn scan_md_recursive(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            scan_md_recursive(root, &path, out);
        } else if meta.is_file() && name_str.ends_with(".md") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

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
                    domain.confidence -= 0.1 * periods;
                    domain.confidence = domain.confidence.max(0.0);
                    if domain.confidence < 0.3 {
                        domain.archived = true;
                    }
                }
            }
        }
    }
}

// ── Injection formatting ──

impl AgentMemory {
    pub fn format_for_injection(&self) -> Option<String> {
        let active_domains: Vec<&KnowledgeDomain> = self
            .knowledge_domains
            .iter()
            .filter(|d| !d.archived && d.confidence >= 0.3)
            .collect();
        let has_style = self.interaction_style.detail_preference.is_some()
            || self.interaction_style.explanation_style.is_some()
            || self.interaction_style.challenge_tolerance.is_some()
            || self.interaction_style.format.is_some()
            || !self.interaction_style.language.is_empty();
        let has_content = !active_domains.is_empty()
            || has_style
            || !self.corrections.is_empty()
            || !self.workspace.frequent_paths.is_empty()
            || !self.sessions.is_empty();

        if !has_content {
            return None;
        }

        let mut parts: Vec<String> =
            vec!["# User Model (accumulated across sessions)".to_string()];

        // ## You — knowledge domains
        if !active_domains.is_empty() || has_style {
            let mut you_section = String::from("## You");

            if !active_domains.is_empty() {
                you_section.push_str("\n- Knowledge: ");
                let domain_strs: Vec<String> = active_domains
                    .iter()
                    .map(|d| format_domain_entry(d))
                    .collect();
                you_section.push_str(&domain_strs.join("; "));
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
                    you_section.push_str(&format!("\n- Style: {}", style_parts.join(", ")));
                }
                if !self.interaction_style.language.is_empty() {
                    let lang_parts: Vec<String> = self
                        .interaction_style
                        .language
                        .iter()
                        .map(|(k, v)| format!("{k} in {v}"))
                        .collect();
                    you_section.push_str(&format!("\n- Language: {}", lang_parts.join(", ")));
                }
            }

            parts.push(you_section);
        }

        // ## Workspace
        if !self.workspace.frequent_paths.is_empty() || !self.workspace.language_distribution.is_empty()
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
            parts.push(ws);
        }

        // ## Rules
        if !self.corrections.is_empty() {
            let mut rules = String::from("## Rules");
            for c in &self.corrections {
                rules.push_str(&format!("\n- {} — {}", c.rule, c.reason));
            }
            parts.push(rules);
        }

        // ## Recent
        if !self.sessions.is_empty() {
            let mut recent = String::from("## Recent");
            let start = self.sessions.len().saturating_sub(INJECTION_RECENT_SESSIONS);
            for s in &self.sessions[start..] {
                let date_short = s.date.get(..10).unwrap_or(&s.date);
                let follow = s
                    .follow_up
                    .as_deref()
                    .map(|f| format!(" → next: {f}"))
                    .unwrap_or_default();
                recent.push_str(&format!("\n- [{}] {}{}", date_short, s.summary, follow));
            }
            parts.push(recent);
        }

        parts.push(
            "Adapt your responses to this user's knowledge level and style.\n\
             Do NOT mention this model unless asked.\n\
             Current request overrides any remembered preference."
                .to_string(),
        );

        Some(parts.join("\n\n"))
    }
}

fn format_domain_entry(d: &KnowledgeDomain) -> String {
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

// ── Trigger detection ──

pub fn contains_trigger(message: &str) -> bool {
    let lower = message.to_lowercase();
    let zh_triggers = ["记住", "忘记", "以后都", "以后每次", "不要再"];
    let en_triggers = [
        "remember ",
        "forget ",
        "from now on",
        "always ",
        "never ",
    ];
    zh_triggers.iter().any(|t| lower.contains(t))
        || en_triggers.iter().any(|t| lower.contains(t))
}

// ── Merge algorithm (Spec 3) ──

fn depth_rank(depth: &str) -> u32 {
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
                    (existing.confidence + (1.0 - existing.confidence) * 0.2).min(0.95);

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

            if self.interaction_style.pending.get(&key) == Some(&value) {
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
                self.interaction_style.pending.insert(key, value);
            }
        }
    }

    fn merge_corrections(
        &mut self,
        new_corrections: Vec<NewCorrection>,
        remove_corrections: Vec<String>,
    ) {
        for rule_text in &remove_corrections {
            self.corrections.retain(|c| c.rule != *rule_text);
        }

        let today = Utc::now().format("%Y-%m-%d").to_string();

        for nc in new_corrections {
            if let Some(existing) = self.corrections.iter_mut().find(|c| c.rule == nc.rule) {
                existing.date = today.clone();
                existing.reason = nc.reason;
            } else {
                self.corrections.push(MemoryCorrection {
                    rule: nc.rule,
                    reason: nc.reason,
                    date: today.clone(),
                    source: "explicit".to_string(),
                });
            }
        }

        if self.corrections.len() > MAX_CORRECTIONS {
            self.corrections.sort_by(|a, b| a.date.cmp(&b.date));
            let excess = self.corrections.len() - MAX_CORRECTIONS;
            self.corrections.drain(0..excess);
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

// ── MemoryManager (Spec 4) ──

const MIN_USER_MESSAGES_FOR_SESSION: usize = 2;
const MAX_EXTRACTION_MESSAGES: usize = 30;

const REFLECTION_PROMPT: &str = r#"You are a memory curator for KnowForge, a personal knowledge management tool.
Your task: compare the EXISTING memory with a NEW extraction from this session,
and produce a list of proposals to keep the memory accurate, non-redundant, and up-to-date.

## Existing memory
{current_memory_json}

## New extraction from this session
{new_update_json}

## What to check

### 1. Conflicts
Does the new extraction contradict anything in existing memory?
(e.g. user was "learning Rust" but now clearly "expert" level)
→ Propose UPDATE or MERGE.

### 2. Staleness
Are there domains/corrections that haven't been referenced in a long time
and the new session shows the user has moved on?
→ Propose ARCHIVE (soft-delete, recoverable).

### 3. Redundancy
Does the new extraction duplicate an existing entry?
→ Propose SKIP (do not add) or MERGE (combine into one).

### 4. Additions
Genuinely new information not already captured?
→ Propose ADD.

## Rules
- Every proposal must have a clear reason.
- Prefer MERGE over ARCHIVE when two entries overlap.
- A single reflection may archive at most 3 domains — be conservative.
- If nothing needs changing, return an empty array [].
- Return ONLY a JSON array of proposals, no markdown fences.

## Proposal schema
[{
  "action": "add" | "update" | "archive" | "merge" | "skip",
  "category": "knowledge_domain" | "correction" | "interaction_style",
  "target": "name of existing entry to modify (null for add)",
  "content": { ... fields for the new/updated entry ... },
  "reason": "why this change"
}]"#;

const SESSION_EXTRACTION_PROMPT: &str = r#"You are a user modeling agent for KnowForge, a personal knowledge management tool.
Your task: analyze the conversation and update the user model to help future sessions
understand this user better.

## Current user model
{current_memory_json}

## What to extract

### 1. Knowledge domains
Identify domains the user engaged with. For each domain:
- "domain": the knowledge area (e.g. "distributed systems", "machine learning")
- "depth": assess from conversation evidence:
    - "curious": user asked basic questions, requested explanations
    - "learning": user followed discussion but needed guidance
    - "practitioner": user showed hands-on experience, discussed trade-offs
    - "expert": user corrected the assistant, discussed cutting-edge topics
- "current_focus": specific sub-topic if identifiable (nullable)
- "motivation": ONLY if the user EXPLICITLY stated why they're interested (nullable)
  Do NOT infer motivation.
- "confidence": signal strength — explicit self-description = 0.8, demonstrated expertise = 0.7, asked questions = 0.5

### 2. Interaction style (only if clearly demonstrated)
Only include fields with CLEAR evidence from this conversation.
Do NOT extrapolate from a single instance.
Possible keys: "detail_preference", "explanation_style", "challenge_tolerance", "format".

### 3. Corrections
User explicitly told the assistant to do/not do something.
Include the user's stated reason.

### 4. Session summary
One concise sentence: what the user wanted and what was done.

## Rules
- One conversation = one evidence point.
- For interaction style: require >= 2 instances in this conversation.
- For motivation: ONLY extract from explicit statements. NEVER infer.
- When uncertain, OMIT the field entirely.
- Do NOT duplicate information already in the current model unless updating it.
- Return ONLY a JSON object, no markdown fences.

Return a JSON object with these optional fields:
knowledge_domains, interaction_style_updates, new_corrections,
remove_corrections, session_summary, session_domains_touched, follow_up."#;

const EXPLICIT_EXTRACTION_PROMPT: &str = r#"The user just gave an explicit memory instruction in a KnowForge session.
Extract what they want remembered or forgotten.

## Current user model
{current_memory_json}

## Context messages
{context_messages}

For "forget" instructions, put the matching rule text in remove_corrections.
For "remember" instructions, add to new_corrections (with reason from context),
knowledge_domains, or interaction_style_updates as appropriate.

Return ONLY a JSON object with these optional fields:
knowledge_domains, interaction_style_updates, new_corrections,
remove_corrections, session_summary, session_domains_touched, follow_up."#;

pub struct MemoryManager {
    pub memory: AgentMemory,
    cloud: Option<Arc<dyn LlmProvider>>,
    workspace_root: PathBuf,
    dirty: bool,
}

impl MemoryManager {
    pub fn new(workspace_root: PathBuf, cloud: Option<Arc<dyn LlmProvider>>) -> Self {
        let mut memory = AgentMemory::load(&workspace_root);
        memory.apply_confidence_decay();

        if is_workspace_stale(&memory.workspace.updated_at) {
            let note_paths = scan_md_paths(&workspace_root);
            memory.workspace = observe_workspace(&note_paths, None);
            if let Err(e) = memory.save(&workspace_root) {
                eprintln!("[memory] Failed to save after workspace observation: {e}");
            }
        }

        Self {
            memory,
            cloud,
            workspace_root,
            dirty: false,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn reset_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn format_for_injection(&self) -> Option<String> {
        self.memory.format_for_injection()
    }

    pub async fn extract_explicit(&mut self, context_messages: &[LlmChatMessage]) {
        let cloud = match &self.cloud {
            Some(c) => c.clone(),
            None => {
                eprintln!("[memory] extract_explicit: no provider available, skipping");
                return;
            }
        };
        eprintln!("[memory] extract_explicit: starting with {} context messages", context_messages.len());

        let prompt = build_explicit_extraction_prompt(&self.memory, context_messages);
        let messages = vec![LlmChatMessage {
            role: "user".to_string(),
            content: prompt,
            ..Default::default()
        }];

        let overrides = CompletionOverrides {
            json_mode: true,
            temperature: Some(0.1),
            ..Default::default()
        };

        match cloud.chat_completion(&messages, Some(&overrides)).await {
            Ok(response) => {
                eprintln!("[memory] extract_explicit raw response: {response}");
                match serde_json::from_str::<UserModelUpdate>(&response) {
                    Ok(update) => {
                        self.memory.merge_user_model(update);
                        if let Err(e) = self.memory.save(&self.workspace_root) {
                            eprintln!("[memory] Failed to save after explicit extraction: {e}");
                        }
                        self.dirty = true;
                    }
                    Err(e) => eprintln!("[memory] Failed to parse extraction response: {e}"),
                }
            }
            Err(e) => eprintln!("[memory] Explicit extraction failed: {e}"),
        }
    }

    pub async fn extract_session_update(
        &self,
        messages: &[LlmChatMessage],
    ) -> Option<UserModelUpdate> {
        let cloud = match &self.cloud {
            Some(c) => c.clone(),
            None => return None,
        };

        let user_msg_count = messages.iter().filter(|m| m.role == "user").count();
        if user_msg_count < MIN_USER_MESSAGES_FOR_SESSION {
            return None;
        }

        let trimmed = trim_messages_for_extraction(messages);
        let prompt = build_session_extraction_prompt(&self.memory, &trimmed);

        let extraction_messages = vec![LlmChatMessage {
            role: "user".to_string(),
            content: prompt,
            ..Default::default()
        }];

        let overrides = CompletionOverrides {
            json_mode: true,
            temperature: Some(0.1),
            ..Default::default()
        };

        match cloud
            .chat_completion(&extraction_messages, Some(&overrides))
            .await
        {
            Ok(response) => {
                eprintln!("[memory] extract_session_update raw response: {response}");
                match serde_json::from_str::<UserModelUpdate>(&response) {
                    Ok(update) => Some(update),
                    Err(e) => {
                        eprintln!("[memory] Failed to parse session extraction: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("[memory] Session extraction failed: {e}");
                None
            }
        }
    }

    pub async fn extract_session_end(&mut self, messages: &[LlmChatMessage]) {
        if let Some(update) = self.extract_session_update(messages).await {
            self.memory.merge_user_model(update);
            if let Err(e) = self.memory.save(&self.workspace_root) {
                eprintln!("[memory] Failed to save after session extraction: {e}");
            }
        }
    }

    pub async fn reflect_on_memory(&self, update: &UserModelUpdate) -> Vec<MemoryProposal> {
        let cloud = match &self.cloud {
            Some(c) => c.clone(),
            None => return Vec::new(),
        };

        let prompt = build_reflection_prompt(&self.memory, update);
        let messages = vec![LlmChatMessage {
            role: "user".to_string(),
            content: prompt,
            ..Default::default()
        }];

        let overrides = CompletionOverrides {
            json_mode: true,
            temperature: Some(0.1),
            ..Default::default()
        };

        match cloud.chat_completion(&messages, Some(&overrides)).await {
            Ok(response) => {
                eprintln!("[memory] reflect_on_memory raw response: {response}");
                match serde_json::from_str::<Vec<MemoryProposal>>(&response) {
                    Ok(mut proposals) => {
                        for p in &mut proposals {
                            if p.id.is_empty() {
                                p.id = generate_proposal_id();
                            }
                        }
                        let mut archive_count = 0;
                        proposals.retain(|p| {
                            if matches!(p.action, ProposalAction::Archive) {
                                archive_count += 1;
                                archive_count <= MAX_ARCHIVES_PER_REFLECTION
                            } else {
                                true
                            }
                        });
                        proposals
                    }
                    Err(e) => {
                        eprintln!("[memory] Failed to parse reflection: {e}");
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                eprintln!("[memory] Reflection LLM call failed: {e}");
                Vec::new()
            }
        }
    }

    // ── Snapshot & pending proposals (Spec 9) ──

    pub fn create_snapshot(&self) -> Result<(), String> {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(SNAPSHOT_FILE);
        let content = serde_json::to_string_pretty(&self.memory)
            .map_err(|e| format!("Snapshot serialization failed: {e}"))?;
        std::fs::create_dir_all(self.workspace_root.join(KNOWFORGE_DIR))
            .map_err(|e| format!("Failed to create .knowforge dir: {e}"))?;
        std::fs::write(&path, format!("{content}\n"))
            .map_err(|e| format!("Snapshot write failed: {e}"))?;
        Ok(())
    }

    pub fn rollback_from_snapshot(&mut self) -> Result<(), String> {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(SNAPSHOT_FILE);
        if !path.exists() {
            return Err("No snapshot to rollback to".to_string());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Snapshot read failed: {e}"))?;
        self.memory = serde_json::from_str(&content)
            .map_err(|e| format!("Snapshot parse failed: {e}"))?;
        self.memory.save(&self.workspace_root)?;
        self.delete_snapshot();
        Ok(())
    }

    pub fn delete_snapshot(&self) {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(SNAPSHOT_FILE);
        let _ = std::fs::remove_file(&path);
    }

    pub fn save_pending_proposals(&self, batch: &MemoryProposalBatch) -> Result<(), String> {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(PENDING_FILE);
        std::fs::create_dir_all(self.workspace_root.join(KNOWFORGE_DIR))
            .map_err(|e| format!("Failed to create .knowforge dir: {e}"))?;
        let content = serde_json::to_string_pretty(batch)
            .map_err(|e| format!("Pending serialization failed: {e}"))?;
        std::fs::write(&path, format!("{content}\n"))
            .map_err(|e| format!("Pending write failed: {e}"))?;
        Ok(())
    }

    pub fn load_pending_proposals(&self) -> Option<MemoryProposalBatch> {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(PENDING_FILE);
        let content = std::fs::read_to_string(&path).ok()?;
        let batch: MemoryProposalBatch = serde_json::from_str(&content).ok()?;

        if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&batch.created_at) {
            let age_days = (Utc::now() - created.with_timezone(&Utc)).num_days();
            if age_days > PROPOSAL_EXPIRY_DAYS {
                let _ = std::fs::remove_file(&path);
                return None;
            }
        }

        Some(batch)
    }

    pub fn has_pending_proposals(&self) -> bool {
        self.workspace_root
            .join(KNOWFORGE_DIR)
            .join(PENDING_FILE)
            .exists()
    }

    pub fn delete_pending_proposals(&self) {
        let path = self.workspace_root.join(KNOWFORGE_DIR).join(PENDING_FILE);
        let _ = std::fs::remove_file(&path);
    }
}

fn trim_messages_for_extraction(messages: &[LlmChatMessage]) -> Vec<LlmChatMessage> {
    if messages.len() <= MAX_EXTRACTION_MESSAGES {
        return messages.to_vec();
    }
    let mut result = Vec::with_capacity(MAX_EXTRACTION_MESSAGES);
    result.extend_from_slice(&messages[..2]);
    result.extend_from_slice(&messages[messages.len() - (MAX_EXTRACTION_MESSAGES - 2)..]);
    result
}

fn build_session_extraction_prompt(memory: &AgentMemory, messages: &[LlmChatMessage]) -> String {
    let memory_json = serde_json::to_string_pretty(memory).unwrap_or_default();
    let conversation = messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| format!("[{}]: {}", m.role, truncate_message(&m.content, 500)))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = SESSION_EXTRACTION_PROMPT.replace("{current_memory_json}", &memory_json);
    format!("{prompt}\n\n## Conversation\n{conversation}")
}

fn build_explicit_extraction_prompt(
    memory: &AgentMemory,
    context_messages: &[LlmChatMessage],
) -> String {
    let memory_json = serde_json::to_string_pretty(memory).unwrap_or_default();
    let context = context_messages
        .iter()
        .map(|m| format!("[{}]: {}", m.role, truncate_message(&m.content, 300)))
        .collect::<Vec<_>>()
        .join("\n\n");

    EXPLICIT_EXTRACTION_PROMPT
        .replace("{current_memory_json}", &memory_json)
        .replace("{context_messages}", &context)
}

fn build_reflection_prompt(memory: &AgentMemory, update: &UserModelUpdate) -> String {
    let memory_json = serde_json::to_string_pretty(memory).unwrap_or_default();
    let update_json = serde_json::to_string_pretty(update).unwrap_or_default();
    REFLECTION_PROMPT
        .replace("{current_memory_json}", &memory_json)
        .replace("{new_update_json}", &update_json)
}

fn truncate_message(content: &str, max_chars: usize) -> &str {
    if content.len() <= max_chars {
        return content;
    }
    match content.char_indices().nth(max_chars) {
        Some((idx, _)) => &content[..idx],
        None => content,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- Load / Save --

    #[test]
    fn default_memory_has_version_2() {
        let m = AgentMemory::default();
        assert_eq!(m.version, 2);
        assert!(m.knowledge_domains.is_empty());
        assert!(m.corrections.is_empty());
        assert!(m.sessions.is_empty());
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let m = AgentMemory::load(tmp.path());
        assert_eq!(m.version, 2);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut m = AgentMemory::default();
        m.corrections.push(MemoryCorrection {
            rule: "use Chinese titles".to_string(),
            reason: "user preference".to_string(),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
        });
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: Some("async".to_string()),
            motivation: None,
            confidence: 0.7,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 2,
            archived: false,
        });
        m.save(tmp.path()).unwrap();
        let loaded = AgentMemory::load(tmp.path());
        assert_eq!(loaded.corrections.len(), 1);
        assert_eq!(loaded.corrections[0].rule, "use Chinese titles");
        assert_eq!(loaded.knowledge_domains.len(), 1);
        assert_eq!(loaded.knowledge_domains[0].domain, "Rust");
        assert!((loaded.knowledge_domains[0].confidence - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn load_corrupt_json_returns_default() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(KNOWFORGE_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MEMORY_FILE), "not json{{{").unwrap();
        let m = AgentMemory::load(tmp.path());
        assert_eq!(m.version, 2);
    }

    #[test]
    fn save_creates_knowforge_dir() {
        let tmp = TempDir::new().unwrap();
        let m = AgentMemory::default();
        m.save(tmp.path()).unwrap();
        assert!(tmp.path().join(KNOWFORGE_DIR).join(MEMORY_FILE).exists());
    }

    // -- Observe workspace --

    #[test]
    fn observe_empty_paths() {
        let ws = observe_workspace(&[], None);
        assert!(ws.frequent_paths.is_empty());
        assert!(ws.language_distribution.is_empty());
        assert!(ws.updated_at.is_some());
    }

    #[test]
    fn observe_language_distribution() {
        let paths = vec![
            "notes/中文笔记.md".to_string(),
            "notes/另一个中文笔记标题.md".to_string(),
            "notes/english.md".to_string(),
        ];
        let ws = observe_workspace(&paths, None);
        let zh = ws.language_distribution.get("zh").copied().unwrap_or(0.0);
        let en = ws.language_distribution.get("en").copied().unwrap_or(0.0);
        assert!(zh > 0.0, "should detect CJK characters");
        assert!(en > 0.0, "should detect ASCII characters");
        assert!(zh > en, "CJK should dominate with these inputs");
    }

    #[test]
    fn observe_frequent_paths() {
        let paths = vec![
            "reading-notes/a.md".to_string(),
            "reading-notes/b.md".to_string(),
            "reading-notes/c.md".to_string(),
            "daily/d.md".to_string(),
        ];
        let ws = observe_workspace(&paths, None);
        assert!(!ws.frequent_paths.is_empty());
        assert_eq!(ws.frequent_paths[0].path, "reading-notes/");
        assert!(ws.frequent_paths[0].description.contains("3"));
    }

    #[test]
    fn observe_frequent_paths_truncated() {
        let mut paths = Vec::new();
        for i in 0..20 {
            paths.push(format!("dir{i}/note.md"));
        }
        let ws = observe_workspace(&paths, None);
        assert!(ws.frequent_paths.len() <= MAX_FREQUENT_PATHS);
    }

    // -- Confidence decay --

    #[test]
    fn decay_within_30_days_no_change() {
        let mut m = AgentMemory::default();
        let today = Utc::now().format("%Y-%m-%d").to_string();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
            last_evidence: today,
            evidence_count: 1,
            archived: false,
        });
        m.apply_confidence_decay();
        assert!((m.knowledge_domains[0].confidence - 0.7).abs() < f64::EPSILON);
        assert!(!m.knowledge_domains[0].archived);
    }

    #[test]
    fn decay_after_60_days() {
        let mut m = AgentMemory::default();
        let old_date = (Utc::now().date_naive() - chrono::Duration::days(61))
            .format("%Y-%m-%d")
            .to_string();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
            last_evidence: old_date,
            evidence_count: 1,
            archived: false,
        });
        m.apply_confidence_decay();
        // 61 days = 2 periods of 30 days → -0.2
        assert!((m.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn decay_archives_below_threshold() {
        let mut m = AgentMemory::default();
        let old_date = (Utc::now().date_naive() - chrono::Duration::days(150))
            .format("%Y-%m-%d")
            .to_string();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: old_date,
            evidence_count: 1,
            archived: false,
        });
        m.apply_confidence_decay();
        assert!(m.knowledge_domains[0].archived);
        assert!(m.knowledge_domains[0].confidence < 0.3);
    }

    #[test]
    fn decay_skips_archived() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "old".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.1,
            last_evidence: "2020-01-01".to_string(),
            evidence_count: 1,
            archived: true,
        });
        m.apply_confidence_decay();
        assert!((m.knowledge_domains[0].confidence - 0.1).abs() < f64::EPSILON);
    }

    // -- Format for injection --

    #[test]
    fn injection_empty_memory_returns_none() {
        let m = AgentMemory::default();
        assert!(m.format_for_injection().is_none());
    }

    #[test]
    fn injection_includes_high_confidence_directly() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "distributed systems".to_string(),
            depth: "practitioner".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.9,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 5,
            archived: false,
        });
        let text = m.format_for_injection().unwrap();
        assert!(text.contains("distributed systems (practitioner"));
        assert!(!text.contains("likely"));
        assert!(!text.contains("possibly"));
    }

    #[test]
    fn injection_medium_confidence_uses_likely() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.6,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let text = m.format_for_injection().unwrap();
        assert!(text.contains("likely learning"));
    }

    #[test]
    fn injection_low_confidence_uses_possibly() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "quantum".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.35,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let text = m.format_for_injection().unwrap();
        assert!(text.contains("possibly curious"));
    }

    #[test]
    fn injection_below_threshold_not_injected() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "invisible".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.2,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
        assert!(m.format_for_injection().is_none());
    }

    #[test]
    fn injection_archived_not_injected() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "archived".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.8,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 3,
            archived: true,
        });
        assert!(m.format_for_injection().is_none());
    }

    #[test]
    fn injection_corrections_format() {
        let mut m = AgentMemory::default();
        m.corrections.push(MemoryCorrection {
            rule: "use Chinese titles".to_string(),
            reason: "user prefers zh".to_string(),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
        });
        let text = m.format_for_injection().unwrap();
        assert!(text.contains("use Chinese titles"));
        assert!(text.contains("user prefers zh"));
    }

    #[test]
    fn injection_sessions_shows_recent_3() {
        let mut m = AgentMemory::default();
        for i in 0..5 {
            m.sessions.push(MemorySession {
                date: format!("2026-06-{:02}T00:00:00Z", 10 + i),
                summary: format!("session {i}"),
                domains_touched: Vec::new(),
                follow_up: None,
            });
        }
        let text = m.format_for_injection().unwrap();
        assert!(!text.contains("session 0"));
        assert!(!text.contains("session 1"));
        assert!(text.contains("session 2"));
        assert!(text.contains("session 3"));
        assert!(text.contains("session 4"));
    }

    // -- Trigger detection --

    #[test]
    fn trigger_zh_keywords() {
        assert!(contains_trigger("记住我喜欢用中文标题"));
        assert!(contains_trigger("忘记这个偏好"));
        assert!(contains_trigger("以后都用简洁模式"));
        assert!(contains_trigger("以后每次都这样"));
        assert!(contains_trigger("不要再用英文标题"));
    }

    #[test]
    fn trigger_en_keywords() {
        assert!(contains_trigger("remember I like bullet points"));
        assert!(contains_trigger("forget that preference"));
        assert!(contains_trigger("from now on use concise mode"));
        assert!(contains_trigger("always use Chinese"));
        assert!(contains_trigger("never use English titles"));
    }

    #[test]
    fn trigger_case_insensitive() {
        assert!(contains_trigger("Remember this please"));
        assert!(contains_trigger("ALWAYS use this format"));
    }

    #[test]
    fn trigger_normal_message_returns_false() {
        assert!(!contains_trigger("hello, how are you?"));
        assert!(!contains_trigger("help me write a note"));
        assert!(!contains_trigger("搜索一下 Rust async"));
    }

    // -- Merge: knowledge domains --

    #[test]
    fn merge_new_domain() {
        let mut m = AgentMemory::default();
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "learning".to_string(),
                current_focus: Some("async".to_string()),
                motivation: None,
                confidence: 0.5,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.knowledge_domains.len(), 1);
        assert_eq!(m.knowledge_domains[0].evidence_count, 1);
        assert!((m.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_existing_domain_accumulates() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "learning".to_string(),
                current_focus: Some("async".to_string()),
                motivation: None,
                confidence: 0.7,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.knowledge_domains.len(), 1);
        assert_eq!(m.knowledge_domains[0].evidence_count, 2);
        // confidence: 0.5 + (1.0 - 0.5) * 0.2 = 0.6
        assert!((m.knowledge_domains[0].confidence - 0.6).abs() < f64::EPSILON);
        assert_eq!(m.knowledge_domains[0].depth, "learning");
        assert_eq!(
            m.knowledge_domains[0].current_focus.as_deref(),
            Some("async")
        );
    }

    #[test]
    fn merge_confidence_progressive() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 1,
            archived: false,
        });
        // Merge 4 times
        for _ in 0..4 {
            let update = UserModelUpdate {
                knowledge_domains: vec![DomainUpdate {
                    domain: "Rust".to_string(),
                    depth: "learning".to_string(),
                    current_focus: None,
                    motivation: None,
                    confidence: 0.5,
                }],
                ..Default::default()
            };
            m.merge_user_model(update);
        }
        // 0.5 → 0.6 → 0.68 → 0.744 → 0.7952
        assert!(m.knowledge_domains[0].confidence > 0.79);
        assert!(m.knowledge_domains[0].confidence <= 0.95);
    }

    #[test]
    fn merge_confidence_capped_at_095() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "expert".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.94,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 10,
            archived: false,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "expert".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.9,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.knowledge_domains[0].confidence <= 0.95);
    }

    #[test]
    fn merge_depth_only_upgrades() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "practitioner".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.8,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 3,
            archived: false,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "curious".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.knowledge_domains[0].depth, "practitioner");
    }

    #[test]
    fn merge_depth_upgrade_requires_confidence() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "expert".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5, // below 0.7 threshold
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.knowledge_domains[0].depth, "learning");
    }

    #[test]
    fn merge_domain_case_insensitive() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: "2026-06-14".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "rust".to_string(),
                depth: "learning".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.knowledge_domains.len(), 1);
        assert_eq!(m.knowledge_domains[0].evidence_count, 2);
    }

    #[test]
    fn merge_reactivates_archived() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.2,
            last_evidence: "2025-01-01".to_string(),
            evidence_count: 1,
            archived: true,
        });
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "learning".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.7,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(!m.knowledge_domains[0].archived);
    }

    #[test]
    fn merge_domains_capacity_overflow() {
        let mut m = AgentMemory::default();
        for i in 0..16 {
            m.knowledge_domains.push(KnowledgeDomain {
                domain: format!("domain-{i}"),
                depth: "curious".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5 + (i as f64 * 0.02),
                last_evidence: "2026-06-15".to_string(),
                evidence_count: 1,
                archived: false,
            });
        }
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "new-domain".to_string(),
                depth: "curious".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.3,
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.knowledge_domains.len() <= MAX_KNOWLEDGE_DOMAINS);
    }

    // -- Merge: interaction style pending --

    #[test]
    fn merge_style_first_observation_goes_to_pending() {
        let mut m = AgentMemory::default();
        let mut updates = HashMap::new();
        updates.insert(
            "detail_preference".to_string(),
            Some("concise".to_string()),
        );
        let update = UserModelUpdate {
            interaction_style_updates: updates,
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.interaction_style.detail_preference.is_none());
        assert_eq!(
            m.interaction_style.pending.get("detail_preference"),
            Some(&"concise".to_string())
        );
    }

    #[test]
    fn merge_style_second_consistent_promotes() {
        let mut m = AgentMemory::default();
        m.interaction_style
            .pending
            .insert("detail_preference".to_string(), "concise".to_string());
        let mut updates = HashMap::new();
        updates.insert(
            "detail_preference".to_string(),
            Some("concise".to_string()),
        );
        let update = UserModelUpdate {
            interaction_style_updates: updates,
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(
            m.interaction_style.detail_preference.as_deref(),
            Some("concise")
        );
        assert!(!m.interaction_style.pending.contains_key("detail_preference"));
    }

    #[test]
    fn merge_style_different_value_replaces_pending() {
        let mut m = AgentMemory::default();
        m.interaction_style
            .pending
            .insert("detail_preference".to_string(), "concise".to_string());
        let mut updates = HashMap::new();
        updates.insert(
            "detail_preference".to_string(),
            Some("detailed".to_string()),
        );
        let update = UserModelUpdate {
            interaction_style_updates: updates,
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.interaction_style.detail_preference.is_none());
        assert_eq!(
            m.interaction_style.pending.get("detail_preference"),
            Some(&"detailed".to_string())
        );
    }

    #[test]
    fn merge_style_skips_when_already_set() {
        let mut m = AgentMemory::default();
        m.interaction_style.detail_preference = Some("concise".to_string());
        let mut updates = HashMap::new();
        updates.insert(
            "detail_preference".to_string(),
            Some("concise".to_string()),
        );
        let update = UserModelUpdate {
            interaction_style_updates: updates,
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.interaction_style.pending.is_empty());
    }

    // -- Merge: corrections --

    #[test]
    fn merge_corrections_remove_then_add() {
        let mut m = AgentMemory::default();
        m.corrections.push(MemoryCorrection {
            rule: "old rule".to_string(),
            reason: "old".to_string(),
            date: "2026-06-14".to_string(),
            source: "explicit".to_string(),
        });
        let update = UserModelUpdate {
            remove_corrections: vec!["old rule".to_string()],
            new_corrections: vec![NewCorrection {
                rule: "new rule".to_string(),
                reason: "new reason".to_string(),
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.corrections.len(), 1);
        assert_eq!(m.corrections[0].rule, "new rule");
    }

    #[test]
    fn merge_corrections_dedup() {
        let mut m = AgentMemory::default();
        m.corrections.push(MemoryCorrection {
            rule: "existing rule".to_string(),
            reason: "old reason".to_string(),
            date: "2026-06-14".to_string(),
            source: "explicit".to_string(),
        });
        let update = UserModelUpdate {
            new_corrections: vec![NewCorrection {
                rule: "existing rule".to_string(),
                reason: "updated reason".to_string(),
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.corrections.len(), 1);
        assert_eq!(m.corrections[0].reason, "updated reason");
    }

    #[test]
    fn merge_corrections_capacity() {
        let mut m = AgentMemory::default();
        for i in 0..21 {
            m.corrections.push(MemoryCorrection {
                rule: format!("rule-{i}"),
                reason: "reason".to_string(),
                date: format!("2026-06-{:02}", (i % 28) + 1),
                source: "explicit".to_string(),
            });
        }
        let update = UserModelUpdate {
            new_corrections: vec![NewCorrection {
                rule: "new rule".to_string(),
                reason: "reason".to_string(),
            }],
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.corrections.len() <= MAX_CORRECTIONS);
    }

    // -- Merge: sessions --

    #[test]
    fn merge_session_appends() {
        let mut m = AgentMemory::default();
        let update = UserModelUpdate {
            session_summary: Some("did stuff".to_string()),
            session_domains_touched: vec!["Rust".to_string()],
            follow_up: Some("continue".to_string()),
            ..Default::default()
        };
        m.merge_user_model(update);
        assert_eq!(m.sessions.len(), 1);
        assert_eq!(m.sessions[0].summary, "did stuff");
        assert_eq!(m.sessions[0].follow_up.as_deref(), Some("continue"));
    }

    #[test]
    fn merge_session_no_summary_skips() {
        let mut m = AgentMemory::default();
        let update = UserModelUpdate::default();
        m.merge_user_model(update);
        assert!(m.sessions.is_empty());
    }

    #[test]
    fn merge_session_capacity() {
        let mut m = AgentMemory::default();
        for i in 0..11 {
            m.sessions.push(MemorySession {
                date: format!("2026-06-{:02}T00:00:00Z", (i % 28) + 1),
                summary: format!("session {i}"),
                domains_touched: Vec::new(),
                follow_up: None,
            });
        }
        let update = UserModelUpdate {
            session_summary: Some("new session".to_string()),
            ..Default::default()
        };
        m.merge_user_model(update);
        assert!(m.sessions.len() <= MAX_SESSIONS);
        assert_eq!(m.sessions.last().unwrap().summary, "new session");
    }

    // -- Reflection: should_reflect --

    #[test]
    fn should_reflect_empty_memory_returns_false() {
        let memory = AgentMemory::default();
        let messages: Vec<LlmChatMessage> = (0..5)
            .map(|i| LlmChatMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("msg {i}"),
                ..Default::default()
            })
            .collect();
        assert!(!should_reflect(&messages, &memory));
    }

    #[test]
    fn should_reflect_short_conversation_returns_false() {
        let mut memory = AgentMemory::default();
        memory.corrections.push(MemoryCorrection {
            rule: "test".to_string(),
            reason: "test".to_string(),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
        });
        let messages = vec![
            LlmChatMessage { role: "user".to_string(), content: "hi".to_string(), ..Default::default() },
            LlmChatMessage { role: "assistant".to_string(), content: "hello".to_string(), ..Default::default() },
            LlmChatMessage { role: "user".to_string(), content: "bye".to_string(), ..Default::default() },
        ];
        assert!(!should_reflect(&messages, &memory));
    }

    #[test]
    fn should_reflect_sufficient_returns_true() {
        let mut memory = AgentMemory::default();
        memory.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let messages: Vec<LlmChatMessage> = (0..6)
            .map(|i| LlmChatMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("msg {i}"),
                ..Default::default()
            })
            .collect();
        assert!(should_reflect(&messages, &memory));
    }

    // -- Reflection: apply_single_proposal --

    #[test]
    fn proposal_add_knowledge_domain() {
        let mut m = AgentMemory::default();
        let proposal = MemoryProposal {
            id: "mp-1".to_string(),
            action: ProposalAction::Add,
            category: "knowledge_domain".to_string(),
            target: None,
            content: serde_json::json!({
                "domain": "Python",
                "depth": "practitioner",
                "confidence": 0.8
            }),
            reason: "user discussed Python".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        assert_eq!(m.knowledge_domains.len(), 1);
        assert_eq!(m.knowledge_domains[0].domain, "Python");
    }

    #[test]
    fn proposal_add_correction() {
        let mut m = AgentMemory::default();
        let proposal = MemoryProposal {
            id: "mp-2".to_string(),
            action: ProposalAction::Add,
            category: "correction".to_string(),
            target: None,
            content: serde_json::json!({
                "rule": "always use Chinese",
                "reason": "user preference"
            }),
            reason: "explicit instruction".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        assert_eq!(m.corrections.len(), 1);
        assert_eq!(m.corrections[0].rule, "always use Chinese");
    }

    #[test]
    fn proposal_update_domain() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: Some("async".to_string()),
            motivation: None,
            confidence: 0.7,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 2,
            archived: false,
        });
        let proposal = MemoryProposal {
            id: "mp-3".to_string(),
            action: ProposalAction::Update,
            category: "knowledge_domain".to_string(),
            target: Some("Rust".to_string()),
            content: serde_json::json!({
                "depth": "practitioner",
                "current_focus": "macros"
            }),
            reason: "user showed deeper expertise".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        assert_eq!(m.knowledge_domains[0].depth, "practitioner");
        assert_eq!(m.knowledge_domains[0].current_focus.as_deref(), Some("macros"));
    }

    #[test]
    fn proposal_archive_domain() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "React".to_string(),
            depth: "curious".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.5,
            last_evidence: "2026-01-01".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let proposal = MemoryProposal {
            id: "mp-4".to_string(),
            action: ProposalAction::Archive,
            category: "knowledge_domain".to_string(),
            target: Some("React".to_string()),
            content: serde_json::json!({}),
            reason: "user moved to Vue".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        assert!(m.knowledge_domains[0].archived);
    }

    #[test]
    fn proposal_merge_archives_old_and_adds_new() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "frontend".to_string(),
            depth: "learning".to_string(),
            current_focus: Some("React".to_string()),
            motivation: None,
            confidence: 0.6,
            last_evidence: "2026-03-01".to_string(),
            evidence_count: 2,
            archived: false,
        });
        let proposal = MemoryProposal {
            id: "mp-5".to_string(),
            action: ProposalAction::Merge,
            category: "knowledge_domain".to_string(),
            target: Some("frontend".to_string()),
            content: serde_json::json!({
                "domain": "frontend",
                "depth": "practitioner",
                "current_focus": "Vue",
                "confidence": 0.7
            }),
            reason: "merge React and Vue into single frontend entry".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        // merge_user_model matches by domain name, so the archived entry
        // gets reactivated and updated in-place (1 entry, not 2).
        assert_eq!(m.knowledge_domains.len(), 1);
        let entry = &m.knowledge_domains[0];
        assert!(!entry.archived);
        assert_eq!(entry.current_focus.as_deref(), Some("Vue"));
    }

    #[test]
    fn proposal_skip_no_change() {
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
            last_evidence: "2026-06-15".to_string(),
            evidence_count: 1,
            archived: false,
        });
        let before = m.knowledge_domains.clone();
        let proposal = MemoryProposal {
            id: "mp-6".to_string(),
            action: ProposalAction::Skip,
            category: "knowledge_domain".to_string(),
            target: Some("Rust".to_string()),
            content: serde_json::json!({}),
            reason: "already captured".to_string(),
        };
        apply_single_proposal(&mut m, &proposal).unwrap();
        assert_eq!(m.knowledge_domains.len(), before.len());
        assert_eq!(m.knowledge_domains[0].depth, before[0].depth);
    }

    #[test]
    fn reflection_prompt_contains_both_inputs() {
        let m = AgentMemory::default();
        let update = UserModelUpdate {
            knowledge_domains: vec![DomainUpdate {
                domain: "Rust".to_string(),
                depth: "learning".to_string(),
                current_focus: None,
                motivation: None,
                confidence: 0.5,
            }],
            ..Default::default()
        };
        let prompt = build_reflection_prompt(&m, &update);
        assert!(prompt.contains("Existing memory"));
        assert!(prompt.contains("New extraction"));
        assert!(prompt.contains("Rust"));
    }

    // -- Snapshot & pending proposals --

    #[test]
    fn snapshot_create_and_rollback() {
        let tmp = TempDir::new().unwrap();
        let mut m = AgentMemory::default();
        m.corrections.push(MemoryCorrection {
            rule: "original".to_string(),
            reason: "test".to_string(),
            date: "2026-06-15".to_string(),
            source: "explicit".to_string(),
        });
        m.save(tmp.path()).unwrap();

        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        assert_eq!(mgr.memory.corrections.len(), 1);

        mgr.create_snapshot().unwrap();
        assert!(tmp.path().join(KNOWFORGE_DIR).join(SNAPSHOT_FILE).exists());

        // Mutate memory after snapshot
        mgr.memory.corrections.push(MemoryCorrection {
            rule: "added after snapshot".to_string(),
            reason: "test".to_string(),
            date: "2026-06-16".to_string(),
            source: "explicit".to_string(),
        });
        mgr.memory.save(tmp.path()).unwrap();
        assert_eq!(mgr.memory.corrections.len(), 2);

        // Rollback restores to snapshot state
        mgr.rollback_from_snapshot().unwrap();
        assert_eq!(mgr.memory.corrections.len(), 1);
        assert_eq!(mgr.memory.corrections[0].rule, "original");

        // Snapshot file is deleted after rollback
        assert!(!tmp.path().join(KNOWFORGE_DIR).join(SNAPSHOT_FILE).exists());
    }

    #[test]
    fn snapshot_rollback_no_snapshot_returns_err() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        let result = mgr.rollback_from_snapshot();
        assert!(result.is_err());
    }

    #[test]
    fn snapshot_delete_nonexistent_no_panic() {
        let tmp = TempDir::new().unwrap();
        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        mgr.delete_snapshot(); // should not panic
    }

    #[test]
    fn pending_proposals_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);

        let batch = MemoryProposalBatch {
            session_id: "test-session".to_string(),
            proposals: vec![MemoryProposal {
                id: "mp-1".to_string(),
                action: ProposalAction::Add,
                category: "knowledge_domain".to_string(),
                target: None,
                content: serde_json::json!({"domain": "Rust", "depth": "learning", "confidence": 0.5}),
                reason: "test".to_string(),
            }],
            created_at: Utc::now().to_rfc3339(),
        };

        mgr.save_pending_proposals(&batch).unwrap();
        assert!(mgr.has_pending_proposals());

        let loaded = mgr.load_pending_proposals().unwrap();
        assert_eq!(loaded.session_id, "test-session");
        assert_eq!(loaded.proposals.len(), 1);
        assert_eq!(loaded.proposals[0].id, "mp-1");
    }

    #[test]
    fn pending_proposals_expired_returns_none() {
        let tmp = TempDir::new().unwrap();
        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);

        let old_date = (Utc::now() - chrono::Duration::days(8)).to_rfc3339();
        let batch = MemoryProposalBatch {
            session_id: "old-session".to_string(),
            proposals: vec![],
            created_at: old_date,
        };

        mgr.save_pending_proposals(&batch).unwrap();
        assert!(mgr.has_pending_proposals());

        let loaded = mgr.load_pending_proposals();
        assert!(loaded.is_none());
        // File should be deleted after expiry check
        assert!(!mgr.has_pending_proposals());
    }

    #[test]
    fn pending_proposals_has_and_delete() {
        let tmp = TempDir::new().unwrap();
        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        assert!(!mgr.has_pending_proposals());

        let batch = MemoryProposalBatch {
            session_id: "s".to_string(),
            proposals: vec![],
            created_at: Utc::now().to_rfc3339(),
        };
        mgr.save_pending_proposals(&batch).unwrap();
        assert!(mgr.has_pending_proposals());

        mgr.delete_pending_proposals();
        assert!(!mgr.has_pending_proposals());
    }

    // -- MemoryManager --

    #[test]
    fn manager_new_loads_and_decays() {
        let tmp = TempDir::new().unwrap();
        let old_date = (Utc::now().date_naive() - chrono::Duration::days(61))
            .format("%Y-%m-%d")
            .to_string();
        let mut m = AgentMemory::default();
        m.knowledge_domains.push(KnowledgeDomain {
            domain: "Rust".to_string(),
            depth: "learning".to_string(),
            current_focus: None,
            motivation: None,
            confidence: 0.7,
            last_evidence: old_date,
            evidence_count: 1,
            archived: false,
        });
        m.save(tmp.path()).unwrap();

        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        assert!((mgr.memory.knowledge_domains[0].confidence - 0.5).abs() < f64::EPSILON);
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn manager_new_missing_file() {
        let tmp = TempDir::new().unwrap();
        let mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        assert_eq!(mgr.memory.version, 2);
        assert!(mgr.memory.knowledge_domains.is_empty());
        assert!(!mgr.is_dirty());
    }

    #[test]
    fn manager_dirty_flag() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        assert!(!mgr.is_dirty());
        mgr.dirty = true;
        assert!(mgr.is_dirty());
        mgr.reset_dirty();
        assert!(!mgr.is_dirty());
    }

    #[tokio::test]
    async fn extract_explicit_no_cloud_skips() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        let messages = vec![LlmChatMessage {
            role: "user".to_string(),
            content: "remember I like bullet points".to_string(),
            ..Default::default()
        }];
        mgr.extract_explicit(&messages).await;
        assert!(!mgr.is_dirty());
        assert!(mgr.memory.corrections.is_empty());
    }

    #[tokio::test]
    async fn extract_session_end_short_conversation_skips() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        let messages = vec![LlmChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            ..Default::default()
        }];
        mgr.extract_session_end(&messages).await;
        assert!(mgr.memory.sessions.is_empty());
    }

    #[tokio::test]
    async fn extract_session_end_no_cloud_skips() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(tmp.path().to_path_buf(), None);
        let messages = vec![
            LlmChatMessage {
                role: "user".to_string(),
                content: "first".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "assistant".to_string(),
                content: "reply".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "user".to_string(),
                content: "second".to_string(),
                ..Default::default()
            },
        ];
        mgr.extract_session_end(&messages).await;
        assert!(mgr.memory.sessions.is_empty());
    }

    // -- trim_messages_for_extraction --

    #[test]
    fn trim_within_limit() {
        let msgs: Vec<LlmChatMessage> = (0..10)
            .map(|i| LlmChatMessage {
                role: "user".to_string(),
                content: format!("msg {i}"),
                ..Default::default()
            })
            .collect();
        let trimmed = trim_messages_for_extraction(&msgs);
        assert_eq!(trimmed.len(), 10);
    }

    #[test]
    fn trim_over_limit() {
        let msgs: Vec<LlmChatMessage> = (0..40)
            .map(|i| LlmChatMessage {
                role: "user".to_string(),
                content: format!("msg {i}"),
                ..Default::default()
            })
            .collect();
        let trimmed = trim_messages_for_extraction(&msgs);
        assert_eq!(trimmed.len(), MAX_EXTRACTION_MESSAGES);
        assert_eq!(trimmed[0].content, "msg 0");
        assert_eq!(trimmed[1].content, "msg 1");
        assert_eq!(trimmed[2].content, "msg 12");
        assert_eq!(trimmed.last().unwrap().content, "msg 39");
    }

    // -- Prompt builders --

    #[test]
    fn session_prompt_contains_memory_and_conversation() {
        let m = AgentMemory::default();
        let msgs = vec![
            LlmChatMessage {
                role: "user".to_string(),
                content: "tell me about Rust".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "assistant".to_string(),
                content: "Rust is a systems language".to_string(),
                ..Default::default()
            },
        ];
        let prompt = build_session_extraction_prompt(&m, &msgs);
        assert!(prompt.contains("\"version\": 2"));
        assert!(prompt.contains("[user]: tell me about Rust"));
        assert!(prompt.contains("[assistant]: Rust is a systems language"));
        assert!(prompt.contains("knowledge_domains"));
    }

    #[test]
    fn session_prompt_filters_tool_messages() {
        let m = AgentMemory::default();
        let msgs = vec![
            LlmChatMessage {
                role: "user".to_string(),
                content: "search something".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "tool".to_string(),
                content: "tool result".to_string(),
                ..Default::default()
            },
            LlmChatMessage {
                role: "assistant".to_string(),
                content: "here's what I found".to_string(),
                ..Default::default()
            },
        ];
        let prompt = build_session_extraction_prompt(&m, &msgs);
        assert!(!prompt.contains("[tool]"));
        assert!(prompt.contains("[user]"));
        assert!(prompt.contains("[assistant]"));
    }

    #[test]
    fn explicit_prompt_contains_context() {
        let m = AgentMemory::default();
        let msgs = vec![LlmChatMessage {
            role: "user".to_string(),
            content: "remember I prefer concise answers".to_string(),
            ..Default::default()
        }];
        let prompt = build_explicit_extraction_prompt(&m, &msgs);
        assert!(prompt.contains("remember I prefer concise answers"));
        assert!(prompt.contains("remove_corrections"));
    }

    // -- truncate_message --

    #[test]
    fn truncate_short_message() {
        assert_eq!(truncate_message("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_message() {
        let long = "a".repeat(600);
        let truncated = truncate_message(&long, 500);
        assert_eq!(truncated.len(), 500);
    }

    #[test]
    fn truncate_multibyte() {
        let msg = "你好世界这是一个测试";
        let truncated = truncate_message(msg, 3);
        assert_eq!(truncated, "你好世");
    }
}
