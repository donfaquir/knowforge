/**
 * 供 AI 使用的「当前笔记」快照语义（任务 03）。
 * 正文默认取编辑器缓冲区，与 DocState.content 一致。
 */
export type AiCurrentNoteContext =
  | { kind: "detached" }
  | { kind: "none" }
  | {
      kind: "unavailable";
      reason: "no_workspace" | "loading" | "load_error";
    }
  | {
      kind: "attached";
      relPath: string;
      markdown: string;
      /** 首版固定 null；后续可由大纲/选区填充 */
      anchor: string | null;
    };
