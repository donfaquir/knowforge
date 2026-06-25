//! Iter 4 Skill framework: Layer 3 above Layer 2 Tools.
//!
//! A Skill is a manifest-defined sub-workflow that runs an isolated agent_loop
//! sub-turn with its own system prompt, tool whitelist, and limits.

pub mod commands;
pub mod loader;
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

const WRITING_COACH_PROMPT: &str = r#"You are a writing coach helping the user refine notes in their personal knowledge base {{workspace_name}}.

Your job:
1. For a given paragraph or note, raise short follow-up questions about logical chains, terminology definitions, and missing premises (1-3 questions per turn, in the same language as the original text).
2. When needed, call vault.search_keyword or note.read to find potentially related notes in the vault and suggest connections using wikilink syntax (e.g. [[Note Title]]).

Hard constraints:
- Never rewrite the user's original text or suggest specific rewrites.
- Never judge writing quality (do not use phrases like "unclear", "poorly written", etc.).
- If tool searches return no related material, say so honestly; do not fabricate paths.
- When referencing other notes, the relPath must come from the tool's actual response."#;

const CHALLENGE_REVIEW_PROMPT: &str = r#"You are a learning review coach helping the user revisit past thoughts and notes in their knowledge base {{workspace_name}}.

Your job:
1. When the user mentions a thought or note, use thought.list / note.list / note.read to retrieve the original content.
2. Choose the most fitting perspective among compare, apply, critique, and transfer, then pose one short review question.
3. After the user responds, give neutral feedback on whether the core was addressed, and optionally invite a next round by transferring to a new context.

Hard constraints:
- Ask only one core question at a time; do not stack multiple questions.
- Do not judge answers as "good" or "bad"; use descriptive phrases like "this covers..." or "this could extend to..." instead.
- If no related thoughts or notes are found by the tools, say so honestly; do not fabricate content."#;

const WEB_RESEARCH_PROMPT: &str = r#"You are a research assistant helping the user conduct web research in their knowledge base {{workspace_name}}.

Your workflow:
1. Analyze the user's research request and break it into 1-3 search keywords (prefer English keywords for broader results).
2. Call web.search to execute the search and obtain a result list.
3. Select 2-4 of the most relevant pages from the results and call web.read_page to read their content in detail.
4. Optionally call vault.search_keyword to check whether related notes already exist in the vault, avoiding duplicates and establishing connections.
5. Synthesize all information into a structured research report.
6. Call note.create to save the report to the vault. Save path format: research/{topic-keyword}.md

Report format requirements:
- Frontmatter tags must include "research".
- Include these sections: Overview, Key Findings (bulleted), Sources.
- Annotate each finding with its source URL.
- End with a `## Sources` section listing all referenced URLs with titles.
- If related notes exist in the vault, link them using [[wikilink]] syntax.

Hard constraints:
- All information must come from actual tool results; never fabricate URLs or facts.
- If a search returns no results or a page read fails, report it honestly; do not fake information.
- Every key finding must have at least one supporting source.
- Write the report in the same language the user used."#;

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
        max_tool_result_chars: 8000,
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
        max_tool_result_chars: 8000,
    }
}

fn web_research_manifest() -> SkillManifest {
    SkillManifest {
        id: "web_research".to_string(),
        name: "网络调研".to_string(),
        version: "0.1.0".to_string(),
        description: "搜索网络信息,精读关键页面,生成调研报告并归档到知识库。".to_string(),
        system_prompt_template: WEB_RESEARCH_PROMPT.to_string(),
        allowed_tools: vec![
            "web.search".to_string(),
            "web.read_page".to_string(),
            "note.create".to_string(),
            "note.append".to_string(),
            "vault.search_keyword".to_string(),
        ],
        max_tool_calls: 15,
        timeout_secs: 120,
        ui_entry: SkillUiEntry::ConversationMode,
        tags: vec!["research".to_string(), "web".to_string()],
        auto_invocable: true,
        when_to_use: Some(
            "用户提出调研任务、要求搜索网络信息、或要求就某个主题生成调研报告时".to_string(),
        ),
        max_tool_result_chars: 20000,
    }
}

pub fn register_builtin_skills(
    skills: &SkillRegistry,
    tools: &ToolRegistry,
) -> Result<(), SkillRegistryError> {
    skills.register_builtin(writing_coach_manifest(), tools)?;
    skills.register_builtin(challenge_review_manifest(), tools)?;
    skills.register_builtin(web_research_manifest(), tools)?;
    Ok(())
}

/// 自定义 Skill 加载结果
pub enum SkillLoadResult {
    /// 成功加载的 Skill ID
    Loaded(String),
    /// 加载失败
    Failed { file: String, error: String },
}

/// 从工作区 `.knowforge/skills/` 加载自定义 Skill
pub fn load_custom_skills(
    skills_dir: &std::path::Path,
    skill_registry: &SkillRegistry,
    tool_registry: &ToolRegistry,
) -> Vec<SkillLoadResult> {
    if !skills_dir.exists() {
        return vec![];
    }

    let file_results = loader::load_skills_from_dir(skills_dir);
    let mut outcomes = vec![];

    for (filename, result) in file_results {
        match result {
            Ok(manifest) => {
                let id = manifest.id.clone();
                match skill_registry.register(manifest, tool_registry) {
                    Ok(()) => outcomes.push(SkillLoadResult::Loaded(id)),
                    Err(e) => outcomes.push(SkillLoadResult::Failed {
                        file: filename,
                        error: format!("skill '{}': {}", id, e),
                    }),
                }
            }
            Err(e) => outcomes.push(SkillLoadResult::Failed {
                file: filename,
                error: e.to_string(),
            }),
        }
    }

    outcomes
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
        crate::tools::register_builtin_tools(&tools, None).unwrap();
        let skills = SkillRegistry::new();
        assert!(register_builtin_skills(&skills, &tools).is_ok());
        let listed = skills.list();
        assert_eq!(listed.len(), 3, "expected 3 built-in skills, got {}", listed.len());
        let ids: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"writing_coach"));
        assert!(ids.contains(&"challenge_review"));
        assert!(ids.contains(&"web_research"));
    }

    #[test]
    fn built_in_skills_reference_existing_tools_only() {
        let tools = ToolRegistry::new();
        crate::tools::register_builtin_tools(&tools, None).unwrap();
        for manifest in [writing_coach_manifest(), challenge_review_manifest(), web_research_manifest()] {
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

    #[test]
    fn web_research_manifest_shape() {
        let m = web_research_manifest();
        assert_eq!(m.id, "web_research");
        assert_eq!(m.ui_entry, SkillUiEntry::ConversationMode);
        assert!(m.system_prompt_template.contains("{{workspace_name}}"));
        assert!(m.allowed_tools.contains(&"web.search".to_string()));
        assert!(m.allowed_tools.contains(&"web.read_page".to_string()));
        assert!(m.allowed_tools.contains(&"note.create".to_string()));
        assert_eq!(m.max_tool_calls, 15);
        assert_eq!(m.timeout_secs, 120);
        assert_eq!(m.max_tool_result_chars, 20000);
        assert!(m.auto_invocable);
    }
}
