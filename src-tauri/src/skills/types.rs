use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillUiEntry {
    ConversationMode,
    EditorPanel,
    Standalone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub system_prompt_template: String,
    pub allowed_tools: Vec<String>,
    pub max_tool_calls: u16,
    pub timeout_secs: u64,
    pub ui_entry: SkillUiEntry,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Iter 5 #4 (Stage 1): allow LLM to invoke this skill as a tool.
    /// When true, a `skill-<id>` tool wrapper is registered automatically
    /// so the model can choose to call the skill mid-conversation.
    #[serde(default)]
    pub auto_invocable: bool,
    /// Short hint shown in the skill list injected into the chat system prompt.
    /// Helps the LLM decide when calling `skill-<id>` is appropriate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    #[serde(default = "default_max_tool_result_chars")]
    pub max_tool_result_chars: u32,
}

fn default_max_tool_result_chars() -> u32 {
    8000
}

impl SkillManifest {
    /// Render the system prompt template with workspace variables.
    /// Supported placeholders: {{workspace_name}}, {{workspace_root}}.
    /// Unknown placeholders are left as-is.
    pub fn render_system_prompt(&self, workspace_name: &str, workspace_root: &str) -> String {
        self.system_prompt_template
            .replace("{{workspace_name}}", workspace_name)
            .replace("{{workspace_root}}", workspace_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SkillManifest {
        SkillManifest {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            version: "0.1.0".to_string(),
            description: "demo skill".to_string(),
            system_prompt_template: "You are in {{workspace_name}} at {{workspace_root}}.".to_string(),
            allowed_tools: vec!["time-now".to_string()],
            max_tool_calls: 4,
            timeout_secs: 30,
            ui_entry: SkillUiEntry::Standalone,
            tags: vec!["test".to_string()],
            auto_invocable: false,
            when_to_use: None,
            max_tool_result_chars: 8000,
        }
    }

    #[test]
    fn renders_placeholders() {
        let m = sample();
        let s = m.render_system_prompt("vault-x", "/tmp/vault-x");
        assert_eq!(s, "You are in vault-x at /tmp/vault-x.");
    }

    #[test]
    fn leaves_unknown_placeholder() {
        let mut m = sample();
        m.system_prompt_template = "hi {{unknown}}".to_string();
        assert_eq!(m.render_system_prompt("a", "b"), "hi {{unknown}}");
    }

    #[test]
    fn serializes_camel_case() {
        let m = sample();
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("systemPromptTemplate").is_some());
        assert!(v.get("allowedTools").is_some());
        assert!(v.get("uiEntry").is_some());
        assert_eq!(v.get("uiEntry").unwrap().as_str(), Some("standalone"));
    }
}
