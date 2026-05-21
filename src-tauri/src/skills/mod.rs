//! Iter 4 Skill framework: Layer 3 above Layer 2 Tools.
//!
//! A Skill is a manifest-defined sub-workflow that runs an isolated agent_loop
//! sub-turn with its own system prompt, tool whitelist, and limits.

pub mod commands;
pub mod registry;
pub mod runtime;
pub mod types;

pub use registry::SkillRegistry;

use crate::tools::registry::ToolRegistry;

/// Built-in skill registration entry point. Empty in Iter 4; Iter 5 will register
/// `writing_coach` and `challenge_review` here.
pub fn register_builtin_skills(
    _skills: &SkillRegistry,
    _tools: &ToolRegistry,
) -> Result<(), registry::SkillRegistryError> {
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
        assert_eq!(skills.list().len(), 0);
    }
}
