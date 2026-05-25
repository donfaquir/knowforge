// Tauri invoke wrappers for Iter 4 Skill commands.
// Iter 5 Stage 1 extended with custom-skill CRUD + tool listing.

import { invoke } from "@tauri-apps/api/core";
import type {
  InvokeSkillResponseJson,
  ListSkillsResponseJson,
  ReloadSkillsResponse,
  SkillListItemJson,
  SkillManifestJson,
  ToolSummary,
} from "../types/skillTypes";

/**
 * List all registered Skills (legacy shape, manifests only).
 *
 * NOTE: backend now flattens `SkillListItem` (manifest + isBuiltin). Because
 * `SkillListItemJson` extends `SkillManifestJson`, returning the array still
 * satisfies callers that consume the original `SkillManifestJson[]` contract.
 */
export async function listSkills(): Promise<SkillManifestJson[]> {
  const res = await invoke<ListSkillsResponseJson>("list_skills");
  return res.skills;
}

/**
 * List all registered Skills with the `isBuiltin` flag — used by the Skill
 * Management panel to discriminate built-in vs custom entries.
 */
export async function listSkillItems(): Promise<SkillListItemJson[]> {
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

/** Persist a brand-new custom skill. Backend writes `.knowforge/skills/<id>.md`. */
export async function createCustomSkill(manifest: SkillManifestJson): Promise<void> {
  await invoke("create_custom_skill", { args: { manifest } });
}

/** Overwrite an existing custom skill on disk + in-memory registry. */
export async function updateCustomSkill(manifest: SkillManifestJson): Promise<void> {
  await invoke("update_custom_skill", { args: { manifest } });
}

/** Delete a custom skill (built-ins reject; backend returns Err). */
export async function deleteCustomSkill(skillId: string): Promise<void> {
  await invoke("delete_custom_skill", { args: { skillId } });
}

/** Re-scan `.knowforge/skills/` from disk and reload all custom skills. */
export async function reloadCustomSkills(): Promise<ReloadSkillsResponse> {
  return invoke<ReloadSkillsResponse>("reload_custom_skills");
}

/** Enumerate available tool names + descriptions for the allowed-tools picker. */
export async function listAvailableTools(): Promise<ToolSummary[]> {
  return invoke<ToolSummary[]>("list_available_tools");
}
