// Iter 4 Skill framework types — mirrors src-tauri/src/skills/types.rs.

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
}

export interface ListSkillsResponseJson {
  skills: SkillManifestJson[];
}

export interface InvokeSkillResponseJson {
  sessionId: string;
  skillId: string;
}
