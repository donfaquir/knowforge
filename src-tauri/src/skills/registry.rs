use dashmap::DashMap;
use std::sync::Arc;

use crate::tools::registry::ToolRegistry;

use super::types::SkillManifest;

#[derive(Debug)]
pub enum SkillRegistryError {
    DuplicateId(String),
    InvalidId(String),
    EmptyAllowedTools(String),
    UnknownTool { skill_id: String, tool: String },
    InvalidVersion(String),
}

impl std::fmt::Display for SkillRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "duplicate skill id: {id}"),
            Self::InvalidId(id) => write!(f, "invalid skill id: {id}"),
            Self::EmptyAllowedTools(id) => write!(f, "skill '{id}' has empty allowed_tools"),
            Self::UnknownTool { skill_id, tool } => {
                write!(f, "skill '{skill_id}' references unknown tool '{tool}'")
            }
            Self::InvalidVersion(id) => write!(f, "skill '{id}' has invalid semver version"),
        }
    }
}

impl std::error::Error for SkillRegistryError {}

fn is_valid_skill_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    let bytes = id.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    for &b in &bytes[1..] {
        if !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'_' {
            return false;
        }
    }
    true
}

pub struct SkillRegistry {
    skills: DashMap<String, Arc<SkillManifest>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: DashMap::new(),
        }
    }

    /// Register a skill manifest. Validates id format, semver, non-empty allowed_tools,
    /// and that every allowed tool exists in `tool_registry`.
    pub fn register(
        &self,
        manifest: SkillManifest,
        tool_registry: &ToolRegistry,
    ) -> Result<(), SkillRegistryError> {
        if !is_valid_skill_id(&manifest.id) {
            return Err(SkillRegistryError::InvalidId(manifest.id.clone()));
        }
        if semver::Version::parse(&manifest.version).is_err() {
            return Err(SkillRegistryError::InvalidVersion(manifest.id.clone()));
        }
        if manifest.allowed_tools.is_empty() {
            return Err(SkillRegistryError::EmptyAllowedTools(manifest.id.clone()));
        }
        for tool_name in &manifest.allowed_tools {
            if tool_registry.get(tool_name).is_none() {
                return Err(SkillRegistryError::UnknownTool {
                    skill_id: manifest.id.clone(),
                    tool: tool_name.clone(),
                });
            }
        }
        if self.skills.contains_key(&manifest.id) {
            return Err(SkillRegistryError::DuplicateId(manifest.id.clone()));
        }
        self.skills.insert(manifest.id.clone(), Arc::new(manifest));
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<SkillManifest>> {
        self.skills.get(id).map(|r| r.value().clone())
    }

    pub fn list(&self) -> Vec<SkillManifest> {
        self.skills
            .iter()
            .map(|entry| (**entry.value()).clone())
            .collect()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::SkillUiEntry;

    fn make_tool_registry_with_time_now() -> ToolRegistry {
        let r = ToolRegistry::new();
        crate::tools::register_builtin_tools(&r).unwrap();
        r
    }

    fn manifest(id: &str, tools: Vec<&str>) -> SkillManifest {
        SkillManifest {
            id: id.to_string(),
            name: id.to_string(),
            version: "0.1.0".to_string(),
            description: "x".to_string(),
            system_prompt_template: "p".to_string(),
            allowed_tools: tools.into_iter().map(String::from).collect(),
            max_tool_calls: 4,
            timeout_secs: 30,
            ui_entry: SkillUiEntry::Standalone,
            tags: vec![],
            auto_invocable: false,
            when_to_use: None,
        }
    }

    #[test]
    fn registers_valid_skill() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        assert!(skills.register(manifest("demo", vec!["time.now"]), &tools).is_ok());
        assert_eq!(skills.list().len(), 1);
        assert!(skills.get("demo").is_some());
    }

    #[test]
    fn rejects_invalid_id() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        assert!(matches!(
            skills.register(manifest("Bad-Id", vec!["time.now"]), &tools),
            Err(SkillRegistryError::InvalidId(_))
        ));
    }

    #[test]
    fn rejects_unknown_tool() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        assert!(matches!(
            skills.register(manifest("demo", vec!["nonexistent.tool"]), &tools),
            Err(SkillRegistryError::UnknownTool { .. })
        ));
    }

    #[test]
    fn rejects_empty_allowed_tools() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        assert!(matches!(
            skills.register(manifest("demo", vec![]), &tools),
            Err(SkillRegistryError::EmptyAllowedTools(_))
        ));
    }

    #[test]
    fn rejects_duplicate() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        skills.register(manifest("demo", vec!["time.now"]), &tools).unwrap();
        assert!(matches!(
            skills.register(manifest("demo", vec!["time.now"]), &tools),
            Err(SkillRegistryError::DuplicateId(_))
        ));
    }

    #[test]
    fn rejects_invalid_version() {
        let tools = make_tool_registry_with_time_now();
        let skills = SkillRegistry::new();
        let mut m = manifest("demo", vec!["time.now"]);
        m.version = "not-semver".to_string();
        assert!(matches!(
            skills.register(m, &tools),
            Err(SkillRegistryError::InvalidVersion(_))
        ));
    }
}
