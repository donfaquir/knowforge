use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

use super::types::{Effect, Risk, Tool, ToolManifest};

// ─── RegistryError ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RegistryError {
    DuplicateName(String),
    InvalidName(String),
    /// effects 含 Read/Write 但 privacy_aware=false
    PrivacyAwareViolation(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName(n) => write!(f, "duplicate tool name: {n}"),
            Self::InvalidName(n) => write!(f, "invalid tool name: {n}"),
            Self::PrivacyAwareViolation(n) => {
                write!(f, "tool '{n}' has Read/Write effects but privacy_aware=false")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

// ─── ListFilter ────────────────────────────────────────────────────────────────

pub struct ListFilter {
    pub effects: Option<Vec<Effect>>,
    pub risk: Option<Vec<Risk>>,
    pub tags: Option<Vec<String>>,
}

// ─── ToolScope ─────────────────────────────────────────────────────────────────

pub enum ToolScope {
    Global,
    Conversation(String),
}

// ─── name regex ────────────────────────────────────────────────────────────────
// ^[a-z][a-z0-9]*(\.[a-z][a-z0-9_]*)+$

fn is_valid_tool_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            return false;
        }
        let bytes = part.as_bytes();
        // first char must be [a-z]
        if !bytes[0].is_ascii_lowercase() {
            return false;
        }
        for &b in &bytes[1..] {
            if i == 0 {
                // first segment: [a-z0-9]
                if !b.is_ascii_lowercase() && !b.is_ascii_digit() {
                    return false;
                }
            } else {
                // subsequent segments: [a-z0-9_]
                if !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'_' {
                    return false;
                }
            }
        }
    }
    true
}

// ─── ToolRegistry ──────────────────────────────────────────────────────────────

pub struct ToolRegistry {
    tools: DashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }

    pub fn register(&self, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        let manifest = tool.manifest();

        // 1. name 格式校验
        if !is_valid_tool_name(&manifest.name) {
            return Err(RegistryError::InvalidName(manifest.name.clone()));
        }

        // 2. semver 格式校验
        if semver::Version::parse(&manifest.version).is_err() {
            return Err(RegistryError::InvalidName(format!(
                "{}: invalid semver version '{}'",
                manifest.name, manifest.version
            )));
        }

        // 3. privacy_aware 一致性
        let has_read_write = manifest
            .effects
            .iter()
            .any(|e| *e == Effect::Read || *e == Effect::Write);
        if has_read_write && !manifest.privacy_aware {
            return Err(RegistryError::PrivacyAwareViolation(manifest.name.clone()));
        }

        // 4. 重复检测
        if self.tools.contains_key(&manifest.name) {
            return Err(RegistryError::DuplicateName(manifest.name.clone()));
        }

        self.tools.insert(manifest.name.clone(), tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).map(|r| r.value().clone())
    }

    pub fn list(&self, filter: ListFilter) -> Vec<ToolManifest> {
        self.tools
            .iter()
            .filter(|entry| {
                let m = entry.value().manifest();
                if let Some(ref effects) = filter.effects {
                    if !effects.iter().any(|e| m.effects.contains(e)) {
                        return false;
                    }
                }
                if let Some(ref risks) = filter.risk {
                    if !risks.contains(&m.risk) {
                        return false;
                    }
                }
                if let Some(ref tags) = filter.tags {
                    if !tags.iter().any(|t| m.tags.contains(t)) {
                        return false;
                    }
                }
                true
            })
            .map(|entry| entry.value().manifest().clone())
            .collect()
    }

    /// 精简 manifest，只含 name/description/input_schema/examples，用于 LLM tool 目录
    pub fn list_for_llm(&self, _scope: ToolScope) -> Vec<Value> {
        self.tools
            .iter()
            .map(|entry| {
                let m = entry.value().manifest();
                serde_json::json!({
                    "name": m.name,
                    "description": m.description,
                    "input_schema": m.input_schema,
                    "examples": m.examples,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names() {
        assert!(is_valid_tool_name("time.now"));
        assert!(is_valid_tool_name("note.read_content"));
        assert!(is_valid_tool_name("ab1.cd2_ef3"));
    }

    #[test]
    fn invalid_names() {
        assert!(!is_valid_tool_name("time"));       // no dot
        assert!(!is_valid_tool_name(".time"));       // starts with dot
        assert!(!is_valid_tool_name("Time.now"));    // uppercase
        assert!(!is_valid_tool_name("1time.now"));   // starts with digit
        assert!(!is_valid_tool_name("time."));       // trailing dot
        assert!(!is_valid_tool_name(""));            // empty
    }
}
