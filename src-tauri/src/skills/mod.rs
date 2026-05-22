//! Iter 4 Skill framework: Layer 3 above Layer 2 Tools.
//!
//! A Skill is a manifest-defined sub-workflow that runs an isolated agent_loop
//! sub-turn with its own system prompt, tool whitelist, and limits.

pub mod commands;
pub mod registry;
pub mod runtime;
pub mod skill_tool;
pub mod types;

pub use registry::SkillRegistry;
pub use skill_tool::{SkillAsTool, SKILL_CONCURRENCY};

use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::Semaphore;

use crate::tools::registry::ToolRegistry;

use registry::SkillRegistryError;
use types::{SkillManifest, SkillUiEntry};

const WRITING_COACH_PROMPT: &str = r#"你是一位写作教练,在用户的个人知识库 {{workspace_name}} 中协助他们打磨当前笔记。

你的工作:
1. 针对用户提供的段落或笔记,提出关于逻辑链条、术语界定、缺失前提的简短追问(每次 1-3 条,与原文同语言)。
2. 在需要时调用 vault.search_keyword 或 note.read,寻找知识库中可能相关的其它笔记,并以 wikilink 形式(例如 [[笔记标题]])推荐建立连接。

硬性约束:
- 绝不重写用户原文,绝不给出"改成..."的建议。
- 绝不评判文字质量(禁止使用"不清晰"/"写得不好"/"poorly written"等)。
- 若工具检索不到相关材料,直接说"未发现明显关联",不要编造路径。
- 引用其它笔记时,relPath 必须出自工具实际返回的列表。"#;

const CHALLENGE_REVIEW_PROMPT: &str = r#"你是一位学习复盘教练,在用户的知识库 {{workspace_name}} 中帮助他们回顾过往的想法与笔记。

你的工作:
1. 当用户提到某条想法/某篇笔记时,用 thought.list / note.list / note.read 取回原文。
2. 围绕"对比"(compare)、"应用"(apply)、"质疑"(critique)、"迁移"(transfer)四种视角中最合适的一种,提出一个简短的复盘问题。
3. 收到用户答复后,中立点评是否触及核心,并可邀请下一轮迁移到新情境。

硬性约束:
- 一次只提一个核心问题,避免堆叠。
- 不评判用户答得"好"或"差",改用"覆盖了..."/"还可以延伸到..."等描述性表述。
- 若工具未返回任何相关想法或笔记,告诉用户"暂时没有找到相关记录",不要凭空编造。"#;

fn writing_coach_manifest() -> SkillManifest {
    SkillManifest {
        id: "writing_coach".to_string(),
        name: "写作教练".to_string(),
        version: "0.1.0".to_string(),
        description: "对当前笔记或段落提出逻辑追问,并推荐知识库中可能的关联笔记。".to_string(),
        system_prompt_template: WRITING_COACH_PROMPT.to_string(),
        allowed_tools: vec!["note.read".to_string(), "vault.search_keyword".to_string()],
        max_tool_calls: 4,
        timeout_secs: 30,
        ui_entry: SkillUiEntry::EditorPanel,
        tags: vec!["writing".to_string(), "coach".to_string()],
        auto_invocable: true,
        when_to_use: Some(
            "用户在打磨一段笔记/段落,希望就逻辑链条/术语/缺失前提获得追问,或想找到知识库中可能相关的其它笔记。"
                .to_string(),
        ),
    }
}

fn challenge_review_manifest() -> SkillManifest {
    SkillManifest {
        id: "challenge_review".to_string(),
        name: "挑战复盘".to_string(),
        version: "0.1.0".to_string(),
        description: "围绕对比/应用/质疑/迁移四种视角,陪用户复盘过往想法。".to_string(),
        system_prompt_template: CHALLENGE_REVIEW_PROMPT.to_string(),
        allowed_tools: vec![
            "note.read".to_string(),
            "note.list".to_string(),
            "thought.list".to_string(),
        ],
        max_tool_calls: 6,
        timeout_secs: 45,
        ui_entry: SkillUiEntry::ConversationMode,
        tags: vec!["review".to_string(), "coach".to_string()],
        auto_invocable: true,
        when_to_use: Some(
            "用户提到过往的某条想法/某篇笔记并想做学习复盘,需要一个针对性的追问从对比/应用/质疑/迁移视角切入。"
                .to_string(),
        ),
    }
}

pub fn register_builtin_skills(
    skills: &SkillRegistry,
    tools: &ToolRegistry,
) -> Result<(), SkillRegistryError> {
    skills.register(writing_coach_manifest(), tools)?;
    skills.register(challenge_review_manifest(), tools)?;
    Ok(())
}

/// Iter 5 #4: register a `skill.<id>` tool wrapper for every auto_invocable
/// skill so the main agent loop can call into them. Must be invoked AFTER
/// [`register_builtin_skills`] (uses the SkillRegistry as the source of truth
/// for which skills are auto_invocable).
///
/// Errors map to RegistryError from the tool layer (duplicate name, etc.) and
/// are coerced into `String` for ergonomic startup wiring.
pub fn register_skill_tools(
    app: &AppHandle,
    skills: &SkillRegistry,
    tools: &ToolRegistry,
    semaphore: Arc<Semaphore>,
) -> Result<(), String> {
    for manifest in skills.list().into_iter().filter(|m| m.auto_invocable) {
        let tool = SkillAsTool::new(&manifest, app.clone(), semaphore.clone());
        tools
            .register(tool)
            .map_err(|e| format!("register skill tool '{}': {e}", manifest.id))?;
    }
    Ok(())
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    #[test]
    fn register_builtin_skills_succeeds() {
        let tools = ToolRegistry::new();
        crate::tools::register_builtin_tools(&tools).unwrap();
        let skills = SkillRegistry::new();
        assert!(register_builtin_skills(&skills, &tools).is_ok());
        let listed = skills.list();
        assert_eq!(listed.len(), 2, "expected 2 built-in skills, got {}", listed.len());
        let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"writing_coach"));
        assert!(ids.contains(&"challenge_review"));
    }

    #[test]
    fn built_in_skills_reference_existing_tools_only() {
        let tools = ToolRegistry::new();
        crate::tools::register_builtin_tools(&tools).unwrap();
        for manifest in [writing_coach_manifest(), challenge_review_manifest()] {
            for tool_name in &manifest.allowed_tools {
                assert!(
                    tools.get(tool_name).is_some(),
                    "skill '{}' references unknown tool '{}'",
                    manifest.id,
                    tool_name
                );
            }
        }
    }

    #[test]
    fn writing_coach_manifest_shape() {
        let m = writing_coach_manifest();
        assert_eq!(m.id, "writing_coach");
        assert_eq!(m.ui_entry, SkillUiEntry::EditorPanel);
        assert!(m.system_prompt_template.contains("{{workspace_name}}"));
    }

    #[test]
    fn challenge_review_manifest_shape() {
        let m = challenge_review_manifest();
        assert_eq!(m.id, "challenge_review");
        assert_eq!(m.ui_entry, SkillUiEntry::ConversationMode);
        assert!(m.system_prompt_template.contains("{{workspace_name}}"));
    }
}
