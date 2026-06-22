use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default, deserialize_with = "deserialize_pending_map")]
    pub pending: HashMap<String, PendingStyleEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PendingStyleEntry {
    pub value: String,
    pub observed_at: String,
}

impl PendingStyleEntry {
    pub fn new(value: String) -> Self {
        Self {
            value,
            observed_at: Utc::now().to_rfc3339(),
        }
    }
}

impl<'de> serde::Deserialize<'de> for PendingStyleEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};

        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = PendingStyleEntry;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or {value, observed_at} object")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<PendingStyleEntry, E> {
                Ok(PendingStyleEntry {
                    value: v.to_string(),
                    observed_at: Utc::now().to_rfc3339(),
                })
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<PendingStyleEntry, M::Error> {
                let mut value = None;
                let mut observed_at = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "value" => value = Some(map.next_value()?),
                        "observed_at" => observed_at = Some(map.next_value()?),
                        _ => { let _ = map.next_value::<serde::de::IgnoredAny>()?; }
                    }
                }
                Ok(PendingStyleEntry {
                    value: value.ok_or_else(|| de::Error::missing_field("value"))?,
                    observed_at: observed_at.unwrap_or_else(|| Utc::now().to_rfc3339()),
                })
            }
        }

        deserializer.deserialize_any(EntryVisitor)
    }
}

fn deserialize_pending_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, PendingStyleEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};

    struct PendingMapVisitor;

    impl<'de> Visitor<'de> for PendingMapVisitor {
        type Value = HashMap<String, PendingStyleEntry>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a map of pending style entries")
        }

        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
            let mut result = HashMap::new();
            while let Some(key) = map.next_key::<String>()? {
                let entry: PendingStyleEntry = map.next_value()?;
                result.insert(key, entry);
            }
            Ok(result)
        }
    }

    deserializer.deserialize_map(PendingMapVisitor)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCorrection {
    pub rule: String,
    pub reason: String,
    pub date: String,
    #[serde(default = "default_explicit")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<String>,
}

impl MemoryCorrection {
    pub fn is_active(&self) -> bool {
        self.superseded_by.is_none()
    }
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
    #[serde(default, alias = "corrections")]
    pub new_corrections: Vec<NewCorrection>,
    #[serde(default, alias = "forget_corrections")]
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
