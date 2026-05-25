use std::path::Path;

use serde::Deserialize;

use crate::skills::types::{SkillManifest, SkillUiEntry};

/// 从 .md 文件内容解析出 SkillManifest
pub fn parse_skill_markdown(content: &str) -> Result<SkillManifest, SkillLoadError> {
    // 1. 查找 YAML frontmatter：以 "---\n" 开头，找到第二个 "---\n"
    let content = content.trim_start();
    if !content.starts_with("---") {
        return Err(SkillLoadError::MissingFrontmatter);
    }

    // 跳过第一个 "---"
    let after_first = &content[3..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    // 找到第二个 "---"
    let end_pos = after_first
        .find("\n---")
        .ok_or(SkillLoadError::MissingFrontmatter)?;

    let yaml_str = &after_first[..end_pos];
    let rest = &after_first[end_pos + 4..]; // skip "\n---"
    let rest = rest.strip_prefix('\n').unwrap_or(rest);

    // 2. 解析 YAML frontmatter
    // 注意：SkillManifest 已有 #[derive(Deserialize)] 和 #[serde(rename_all = "camelCase")]
    // 但 YAML frontmatter 使用 snake_case，所以需要一个中间结构体
    #[derive(Deserialize)]
    struct SkillFrontmatter {
        id: String,
        name: String,
        version: String,
        description: String,
        allowed_tools: Vec<String>,
        max_tool_calls: u8,
        timeout_secs: u64,
        ui_entry: SkillUiEntry,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        auto_invocable: bool,
        #[serde(default)]
        when_to_use: Option<String>,
    }

    let fm: SkillFrontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| SkillLoadError::InvalidYaml(e.to_string()))?;

    // 3. 提取系统提示词（Markdown 正文）
    let system_prompt = rest.trim().to_string();
    if system_prompt.is_empty() {
        return Err(SkillLoadError::EmptyPrompt);
    }

    // 4. 组装 SkillManifest
    Ok(SkillManifest {
        id: fm.id,
        name: fm.name,
        version: fm.version,
        description: fm.description,
        system_prompt_template: system_prompt,
        allowed_tools: fm.allowed_tools,
        max_tool_calls: fm.max_tool_calls,
        timeout_secs: fm.timeout_secs,
        ui_entry: fm.ui_entry,
        tags: fm.tags,
        auto_invocable: fm.auto_invocable,
        when_to_use: fm.when_to_use,
    })
}

/// 序列化 SkillManifest 为 Markdown 格式
pub fn serialize_skill_markdown(manifest: &SkillManifest) -> String {
    // 构建 YAML frontmatter（手动构建以保持 snake_case 格式）
    let mut yaml = String::new();
    yaml.push_str("---\n");
    yaml.push_str(&format!("id: {}\n", manifest.id));
    yaml.push_str(&format!("name: {}\n", manifest.name));
    yaml.push_str(&format!("version: {}\n", manifest.version));
    yaml.push_str(&format!("description: {}\n", manifest.description));
    yaml.push_str("allowed_tools:\n");
    for tool in &manifest.allowed_tools {
        yaml.push_str(&format!("  - {}\n", tool));
    }
    yaml.push_str(&format!("max_tool_calls: {}\n", manifest.max_tool_calls));
    yaml.push_str(&format!("timeout_secs: {}\n", manifest.timeout_secs));
    let ui_entry_str = match manifest.ui_entry {
        SkillUiEntry::ConversationMode => "conversation_mode",
        SkillUiEntry::EditorPanel => "editor_panel",
        SkillUiEntry::Standalone => "standalone",
    };
    yaml.push_str(&format!("ui_entry: {}\n", ui_entry_str));
    if !manifest.tags.is_empty() {
        yaml.push_str("tags:\n");
        for tag in &manifest.tags {
            yaml.push_str(&format!("  - {}\n", tag));
        }
    }
    if manifest.auto_invocable {
        yaml.push_str("auto_invocable: true\n");
    }
    if let Some(ref when) = manifest.when_to_use {
        yaml.push_str(&format!("when_to_use: {}\n", when));
    }
    yaml.push_str("---\n\n");
    yaml.push_str(&manifest.system_prompt_template);
    yaml.push('\n');
    yaml
}

/// 扫描目录加载所有 .md Skill 文件
pub fn load_skills_from_dir(dir: &Path) -> Vec<(String, Result<SkillManifest, SkillLoadError>)> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return vec![("".to_string(), Err(SkillLoadError::Io(e)))],
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                results.push(("".to_string(), Err(SkillLoadError::Io(e))));
                continue;
            }
        };

        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    results.push((filename, parse_skill_markdown(&content)));
                }
                Err(e) => {
                    results.push((filename, Err(SkillLoadError::Io(e))));
                }
            }
        }
    }

    results
}

#[derive(Debug)]
pub enum SkillLoadError {
    MissingFrontmatter,
    InvalidYaml(String),
    EmptyPrompt,
    Io(std::io::Error),
}

impl std::fmt::Display for SkillLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFrontmatter => write!(f, "missing YAML frontmatter (---...---)"),
            Self::InvalidYaml(msg) => write!(f, "invalid YAML: {}", msg),
            Self::EmptyPrompt => write!(f, "empty system prompt (no content after frontmatter)"),
            Self::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for SkillLoadError {}
