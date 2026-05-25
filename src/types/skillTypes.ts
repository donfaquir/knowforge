// Iter 4 Skill framework types — mirrors src-tauri/src/skills/types.rs.
// Iter 5 Stage 1 extended for custom-skill CRUD (mirrors src-tauri/src/skills/commands.rs).

export type SkillUiEntry = "conversation_mode" | "editor_panel" | "standalone";

export interface SkillManifestJson {
  id: string;
  name: string;
  version: string;
  description: string;
  systemPromptTemplate: string;
  allowedTools: string[];
  maxToolCalls: number;
  timeoutSecs: number;
  uiEntry: SkillUiEntry;
  tags: string[];
  /** Iter 5 #4: when true, exposed as a `skill.<id>` tool to the LLM. */
  autoInvocable?: boolean;
  /** Optional hint shown alongside the skill in chat-system tool listings. */
  whenToUse?: string | null;
}

/**
 * Backend `SkillListItem` flattens `manifest` into the same level and adds `isBuiltin`.
 * (Rust: `#[serde(flatten)] manifest`, plus `is_builtin`.)
 */
export interface SkillListItemJson extends SkillManifestJson {
  isBuiltin: boolean;
}

export interface ListSkillsResponseJson {
  skills: SkillListItemJson[];
}

export interface InvokeSkillResponseJson {
  sessionId: string;
  skillId: string;
}

// ── CRUD args (camelCase wraps `manifest` / `skillId`) ────────────────────────
export interface CreateSkillArgs {
  manifest: SkillManifestJson;
}

export interface UpdateSkillArgs {
  manifest: SkillManifestJson;
}

export interface DeleteSkillArgs {
  skillId: string;
}

// ── Reload result ─────────────────────────────────────────────────────────────
export interface SkillLoadFailure {
  file: string;
  error: string;
}

export interface ReloadSkillsResponse {
  loaded: string[];
  failed: SkillLoadFailure[];
}

// ── Available tools (for the allowed-tools picker) ────────────────────────────
export interface ToolSummary {
  name: string;
  description: string;
}
