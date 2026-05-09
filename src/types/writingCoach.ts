/** IPC `analyze_writing_coach` 响应（camelCase，与 Rust serde 对齐） */

/** 候选链接类型；Rust `filter_response` 仅放行 `note` / `thought`（小写） */
export type WritingCoachLinkKind = "note" | "thought";

export type WritingCoachLinkItem = {
  title: string;
  relPath: string;
  kind: WritingCoachLinkKind;
  thoughtId?: string;
  excerpt?: string;
};

export type AnalyzeWritingCoachResponse = {
  reasoningQuestions: string[];
  links: WritingCoachLinkItem[];
  knowledgeModuleSkipped: boolean;
};
