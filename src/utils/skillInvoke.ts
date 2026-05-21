// Tauri invoke wrappers for Iter 4 Skill commands.

import { invoke } from "@tauri-apps/api/core";
import type {
  InvokeSkillResponseJson,
  ListSkillsResponseJson,
  SkillManifestJson,
} from "../types/skillTypes";

/**
 * List all registered Skills.
 */
export async function listSkills(): Promise<SkillManifestJson[]> {
  const res = await invoke<ListSkillsResponseJson>("list_skills");
  return res.skills;
}

/**
 * Invoke a Skill. Returns the spawned session id; results stream through the same
 * `llm:stream-chunk` / `llm:tool-*` / `llm:agent-done` events as a normal chat turn.
 *
 * @param skillId the manifest id, e.g. "writing_coach"
 * @param input user input passed to the skill as the initial user message
 * @param conversationId optional parent conversation id; scopes approval cache and audit
 * @param model optional model override
 */
export async function invokeSkill(
  skillId: string,
  input: string,
  conversationId?: string,
  model?: string,
): Promise<InvokeSkillResponseJson> {
  return invoke<InvokeSkillResponseJson>("invoke_skill", {
    args: {
      skillId,
      input,
      conversationId: conversationId ?? null,
      model: model ?? null,
    },
  });
}
