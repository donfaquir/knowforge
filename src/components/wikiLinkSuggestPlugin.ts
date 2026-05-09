/**
 * 检测未完成的 `[[...`（非 `![[`），同步 wikiLinkSuggestStore 供 WikiLinkSuggestPopover 展示。
 */
import type { ResolvedPos } from "@milkdown/prose/model";
import type { EditorState } from "@milkdown/prose/state";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { TextSelection } from "@milkdown/prose/state";
import { $prose } from "@milkdown/utils";
import { isCodeBlockContainerNodeName } from "../utils/milkdownCodeBlockNodeName";
import {
  clearWikiSuggestDismiss,
  clearWikiSuggestDismissIfStale,
  dismissWikiSuggestAtAnchor,
  isWikiSuggestDismissedForAnchor,
  setWikiSuggestSnapshot,
  type WikiSuggestSnapshot,
} from "./wikiLinkSuggestStore";

export const wikiLinkSuggestPluginKey = new PluginKey("kf-wikilink-suggest");

function isUnderCodeOrLiteral($pos: ResolvedPos): boolean {
  for (let d = $pos.depth; d > 0; d--) {
    const t = $pos.node(d).type;
    if (t.spec.code) {
      return true;
    }
    if (isCodeBlockContainerNodeName(t.name)) {
      return true;
    }
  }
  return false;
}

function hasCodeInlineMark($pos: ResolvedPos): boolean {
  return $pos.marks().some((m) => m.type.name === "code_inline");
}

/** 从光标位置解析未闭合的 wikilink 起点与过滤串 */
function deriveIncompleteWikiLink(state: EditorState): WikiSuggestSnapshot {
  const sel = state.selection;
  if (!(sel instanceof TextSelection) || !sel.empty) {
    return { open: false };
  }
  const pos = sel.from;
  const $pos = state.doc.resolve(pos);
  if (isUnderCodeOrLiteral($pos) || hasCodeInlineMark($pos)) {
    return { open: false };
  }
  if (!$pos.parent.isTextblock) {
    return { open: false };
  }
  const blockStart = $pos.start();
  const text = state.doc.textBetween(blockStart, pos);
  const openIdx = text.lastIndexOf("[[");
  if (openIdx < 0) {
    return { open: false };
  }
  if (openIdx >= 1 && text.slice(openIdx - 1, openIdx + 2) === "![[") {
    return { open: false };
  }
  const after = text.slice(openIdx + 2);
  if (after.includes("]]")) {
    return { open: false };
  }
  const anchor = blockStart + openIdx;
  return { open: true, anchor, head: pos, filter: after };
}

function syncStoreFromState(state: EditorState): void {
  let next = deriveIncompleteWikiLink(state);
  if (next.open) {
    clearWikiSuggestDismissIfStale(next.anchor);
    if (isWikiSuggestDismissedForAnchor(next.anchor)) {
      next = { open: false };
    }
  } else {
    clearWikiSuggestDismissIfStale(null);
  }
  setWikiSuggestSnapshot(next);
}

export const wikiLinkSuggestPlugin = $prose(() => {
  return new Plugin({
    key: wikiLinkSuggestPluginKey,
    view: () => ({
      update(view) {
        syncStoreFromState(view.state);
      },
      destroy() {
        setWikiSuggestSnapshot({ open: false });
        clearWikiSuggestDismiss();
      },
    }),
    props: {
      handleTextInput(view, from, _to, text) {
        if (text !== "[") {
          return false;
        }
        const left = view.state.doc.textBetween(Math.max(0, from - 1), from);
        if (left !== "[") {
          return false;
        }
        queueMicrotask(() => {
          syncStoreFromState(view.state);
        });
        return false;
      },
      handleKeyDown(view, event) {
        if (event.key === "Escape") {
          const snap = deriveIncompleteWikiLink(view.state);
          if (snap.open) {
            dismissWikiSuggestAtAnchor(snap.anchor);
            event.preventDefault();
            return true;
          }
        }
        return false;
      },
    },
  });
});
