/**
 * 在 ProseMirror 文档变更时短防抖回调宿主（与 markdownUpdated 互补，减少漏报导致永不自动保存）。
 */
import { $prose } from "@milkdown/utils";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { editorAutosaveBridgeRef } from "./editorDocAutosaveBridgeContext";

const KEY = new PluginKey("kf-editor-doc-autosave-bridge");
/** 略长于 listener 内 debounce，减少双通道重复；仍保持近实时 */
const BRIDGE_DEBOUNCE_MS = 260;

export const editorDocAutosaveBridgePlugin = $prose(() => {
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  return new Plugin({
    key: KEY,
    view: () => ({
      update(_view, prevState) {
        const ctx = editorAutosaveBridgeRef.current;
        if (!ctx || ctx.isSuppressBaseline()) {
          return;
        }
        const v = _view.state;
        if (prevState.doc.eq(v.doc)) {
          return;
        }
        if (debounceTimer !== null) {
          clearTimeout(debounceTimer);
        }
        debounceTimer = window.setTimeout(() => {
          debounceTimer = null;
          const c = editorAutosaveBridgeRef.current;
          if (!c || c.isSuppressBaseline()) {
            return;
          }
          let md: string;
          try {
            md = c.getMarkdownBody();
          } catch {
            return;
          }
          c.onDocBodyChange(c.getDocKey(), md);
        }, BRIDGE_DEBOUNCE_MS);
      },
      destroy() {
        if (debounceTimer !== null) {
          clearTimeout(debounceTimer);
          debounceTimer = null;
        }
      },
    }),
  });
});
