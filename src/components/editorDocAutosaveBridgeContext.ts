/**
 * Crepe 内 ProseMirror 文档变更桥接：供 autosave bridge 插件读取当前 getMarkdown 与抑制标志（避免循环依赖）。
 */
export type EditorAutosaveBridgeContext = {
  getMarkdownBody: () => string;
  getDocKey: () => string;
  isSuppressBaseline: () => boolean;
  onDocBodyChange: (relPath: string, markdown: string) => void;
};

export const editorAutosaveBridgeRef: { current: EditorAutosaveBridgeContext | null } = {
  current: null,
};
