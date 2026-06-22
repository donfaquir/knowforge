use chrono::Utc;

use crate::llm::provider::CompletionOverrides;
use crate::llm::LlmChatMessage;

use super::merge::{depth_rank, MAX_ARCHIVES_PER_REFLECTION};
use super::types::*;

pub(super) const MIN_USER_MESSAGES_FOR_SESSION: usize = 2;
pub(super) const MAX_EXTRACTION_MESSAGES: usize = 30;
pub(super) const PROPOSAL_EXPIRY_DAYS: i64 = 7;

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

// ── Proposal helpers ──

fn generate_proposal_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("mp-{ts:x}-{seq:04x}")
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
                                if depth_rank(dep) > depth_rank(&d.depth) {
                                    d.depth = dep.to_string();
                                }
                            }
                            d.last_evidence = Utc::now().format("%Y-%m-%d").to_string();
                            d.evidence_count += 1;
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

// ── Extraction helpers ──

pub(super) fn trim_messages_for_extraction(messages: &[LlmChatMessage]) -> Vec<LlmChatMessage> {
    if messages.len() <= MAX_EXTRACTION_MESSAGES {
        return messages.to_vec();
    }
    let mut head_end = 0;
    for (i, m) in messages.iter().enumerate() {
        head_end = i + 1;
        if m.role != "system" {
            break;
        }
    }
    head_end = head_end.min(MAX_EXTRACTION_MESSAGES / 2);
    let tail_count = MAX_EXTRACTION_MESSAGES - head_end;
    let tail_start = messages.len().saturating_sub(tail_count);

    let mut result = Vec::with_capacity(MAX_EXTRACTION_MESSAGES);
    result.extend_from_slice(&messages[..head_end]);
    if tail_start > head_end {
        result.extend_from_slice(&messages[tail_start..]);
    }
    result
}

pub(super) fn build_memory_summary_for_extraction(memory: &AgentMemory) -> String {
    let mut parts = Vec::new();

    let domains: Vec<String> = memory
        .knowledge_domains
        .iter()
        .filter(|d| !d.archived)
        .map(|d| {
            let mut line = format!("- {} ({})", d.domain, d.depth);
            if let Some(ref focus) = d.current_focus {
                line.push_str(&format!(", focus: {focus}"));
            }
            line
        })
        .collect();
    if !domains.is_empty() {
        parts.push(format!("Known domains:\n{}", domains.join("\n")));
    }

    let active_rules: Vec<String> = memory
        .corrections
        .iter()
        .filter(|c| c.is_active())
        .map(|c| format!("- {}", c.rule))
        .collect();
    if !active_rules.is_empty() {
        parts.push(format!("Existing rules:\n{}", active_rules.join("\n")));
    }

    let style = &memory.interaction_style;
    let mut style_items = Vec::new();
    if let Some(ref v) = style.detail_preference {
        style_items.push(format!("detail_preference={v}"));
    }
    if let Some(ref v) = style.explanation_style {
        style_items.push(format!("explanation_style={v}"));
    }
    if let Some(ref v) = style.challenge_tolerance {
        style_items.push(format!("challenge_tolerance={v}"));
    }
    if let Some(ref v) = style.format {
        style_items.push(format!("format={v}"));
    }
    if !style_items.is_empty() {
        parts.push(format!("Interaction style: {}", style_items.join(", ")));
    }

    if parts.is_empty() {
        "No existing memory.".to_string()
    } else {
        parts.join("\n\n")
    }
}

pub(super) fn build_session_extraction_prompt(memory: &AgentMemory, messages: &[LlmChatMessage]) -> String {
    let memory_json = build_memory_summary_for_extraction(memory);
    let conversation = messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .map(|m| format!("[{}]: {}", m.role, truncate_message(&m.content, 500)))
        .collect::<Vec<_>>()
        .join("\n\n");

    let prompt = SESSION_EXTRACTION_PROMPT.replace("{current_memory_json}", &memory_json);
    format!("{prompt}\n\n## Conversation\n{conversation}")
}

pub(super) fn build_reflection_prompt(memory: &AgentMemory, update: &UserModelUpdate) -> String {
    let memory_json = build_memory_summary_for_extraction(memory);
    let update_json = serde_json::to_string_pretty(update).unwrap_or_default();
    REFLECTION_PROMPT
        .replace("{current_memory_json}", &memory_json)
        .replace("{new_update_json}", &update_json)
}

pub(super) fn truncate_message(content: &str, max_chars: usize) -> &str {
    if content.len() <= max_chars {
        return content;
    }
    match content.char_indices().nth(max_chars) {
        Some((idx, _)) => &content[..idx],
        None => content,
    }
}

// ── MemoryManager extraction methods ──

impl super::MemoryManager {
    pub async fn extract_session_update(
        &self,
        messages: &[LlmChatMessage],
    ) -> Result<Option<UserModelUpdate>, String> {
        let cloud = match &self.cloud {
            Some(c) => c.clone(),
            None => return Ok(None),
        };

        let user_msg_count = messages.iter().filter(|m| m.role == "user").count();
        if user_msg_count < MIN_USER_MESSAGES_FOR_SESSION {
            return Ok(None);
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
                    Ok(update) => Ok(Some(update)),
                    Err(e) => {
                        eprintln!("[memory] Failed to parse session extraction: {e}");
                        Err(format!("Failed to parse session extraction: {e}"))
                    }
                }
            }
            Err(e) => {
                eprintln!("[memory] Session extraction failed: {e}");
                Err(format!("Session extraction LLM call failed: {e}"))
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
}
