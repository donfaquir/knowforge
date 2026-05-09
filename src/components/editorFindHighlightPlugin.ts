/**
 * Milkdown：篇内查找高亮与当前匹配（ProseMirror Decoration）。
 */

import { $prose } from "@milkdown/utils";
import type { Node as PmNode } from "@milkdown/prose/model";
import { Plugin, PluginKey, TextSelection } from "@milkdown/prose/state";
import { Decoration, DecorationSet } from "@milkdown/prose/view";

const FIND_PLUGIN_KEY = new PluginKey("knowforge-editor-find");

const CSS_FIND_MATCH = "editor-find-match";
const CSS_FIND_CURRENT = "editor-find-match--current";

/** 高亮与 ranges 上限，避免超大文档 + 短关键词时主线程与 Decoration 过载 */
const FIND_HIGHLIGHT_MAX_MATCHES = 3000;

export const editorFindHighlightPluginKey = FIND_PLUGIN_KEY;

type FindMeta =
  | { type: "set"; query: string; caseSensitive: boolean }
  | { type: "next" }
  | { type: "prev" }
  | { type: "clear" };

type FindPluginState = {
  query: string;
  caseSensitive: boolean;
  ranges: { from: number; to: number }[];
  /** 命中数是否因 FIND_HIGHLIGHT_MAX_MATCHES 截断 */
  rangesTruncated: boolean;
  index: number;
  deco: DecorationSet;
};

function collectRangesInDoc(
  doc: PmNode,
  query: string,
  caseSensitive: boolean,
): { ranges: { from: number; to: number }[]; truncated: boolean } {
  const q = query.trim();
  if (!q) {
    return { ranges: [], truncated: false };
  }
  const out: { from: number; to: number }[] = [];
  const needle = caseSensitive ? q : q.toLowerCase();
  const step = Math.max(needle.length, 1);
  let truncated = false;

  // 与 descendants 等价遍历；回调 return false 可立即结束整文档 walk，便于达上限后早停
  doc.nodesBetween(0, doc.content.size, (node, pos) => {
    if (out.length >= FIND_HIGHLIGHT_MAX_MATCHES) {
      truncated = true;
      return false;
    }
    if (!node.isText || !node.text) {
      return;
    }
    const text = node.text;
    const hay = caseSensitive ? text : text.toLowerCase();
    let from = 0;
    while (from < hay.length) {
      const idx = hay.indexOf(needle, from);
      if (idx < 0) {
        break;
      }
      const start = pos + idx;
      const end = start + q.length;
      out.push({ from: start, to: end });
      if (out.length >= FIND_HIGHLIGHT_MAX_MATCHES) {
        truncated = true;
        return false;
      }
      from = idx + step;
    }
  });

  return { ranges: out, truncated };
}

function buildDeco(doc: PmNode, ranges: { from: number; to: number }[], index: number): DecorationSet {
  if (ranges.length === 0) {
    return DecorationSet.empty;
  }
  const decos: Decoration[] = [];
  for (let i = 0; i < ranges.length; i++) {
    const { from, to } = ranges[i];
    if (from >= to || to > doc.content.size) {
      continue;
    }
    const cls = i === index ? `${CSS_FIND_MATCH} ${CSS_FIND_CURRENT}` : CSS_FIND_MATCH;
    decos.push(Decoration.inline(from, to, { class: cls }));
  }
  return DecorationSet.create(doc, decos);
}

function clampIndex(len: number, i: number): number {
  if (len === 0) {
    return 0;
  }
  const m = ((i % len) + len) % len;
  return m;
}

export const editorFindHighlightPlugin = $prose(() => {
  const initState: FindPluginState = {
    query: "",
    caseSensitive: false,
    ranges: [],
    rangesTruncated: false,
    index: 0,
    deco: DecorationSet.empty,
  };

  return new Plugin({
    key: FIND_PLUGIN_KEY,
    state: {
      init(_, state) {
        return { ...initState, deco: DecorationSet.create(state.doc, []) };
      },
      apply(tr, prev) {
        const meta = tr.getMeta(FIND_PLUGIN_KEY) as FindMeta | undefined;
        const doc = tr.doc;
        if (meta?.type === "clear") {
          return {
            query: "",
            caseSensitive: prev.caseSensitive,
            ranges: [],
            rangesTruncated: false,
            index: 0,
            deco: DecorationSet.empty,
          };
        }
        if (meta?.type === "set") {
          const { ranges, truncated } = collectRangesInDoc(doc, meta.query, meta.caseSensitive);
          const index = 0;
          return {
            query: meta.query,
            caseSensitive: meta.caseSensitive,
            ranges,
            rangesTruncated: truncated,
            index,
            deco: buildDeco(doc, ranges, index),
          };
        }
        if (meta?.type === "next" && prev.ranges.length > 0) {
          const index = clampIndex(prev.ranges.length, prev.index + 1);
          return {
            ...prev,
            index,
            deco: buildDeco(doc, prev.ranges, index),
          };
        }
        if (meta?.type === "prev" && prev.ranges.length > 0) {
          const index = clampIndex(prev.ranges.length, prev.index - 1);
          return {
            ...prev,
            index,
            deco: buildDeco(doc, prev.ranges, index),
          };
        }
        if (tr.docChanged && prev.query.trim()) {
          const { ranges, truncated } = collectRangesInDoc(doc, prev.query, prev.caseSensitive);
          const index = clampIndex(ranges.length, Math.min(prev.index, ranges.length - 1));
          return {
            ...prev,
            ranges,
            rangesTruncated: truncated,
            index,
            deco: buildDeco(doc, ranges, index),
          };
        }
        return prev;
      },
    },
    props: {
      decorations(state) {
        return FIND_PLUGIN_KEY.getState(state)?.deco ?? null;
      },
    },
  });
});

export function dispatchFindMeta(view: import("@milkdown/prose/view").EditorView, meta: FindMeta): void {
  view.dispatch(view.state.tr.setMeta(FIND_PLUGIN_KEY, meta));
}

/** 供查找条展示：current 为 1-based；无活动查找词时返回 null */
export function getFindMatchSummary(
  view: import("@milkdown/prose/view").EditorView,
): { total: number; current: number; truncated?: boolean } | null {
  const st = FIND_PLUGIN_KEY.getState(view.state) as FindPluginState | undefined;
  if (!st?.query?.trim()) {
    return null;
  }
  const total = st.ranges.length;
  if (total === 0) {
    return { total: 0, current: 0 };
  }
  return { total, current: st.index + 1, truncated: st.rangesTruncated || undefined };
}

/**
 * Crepe 实际纵向滚动在 [data-milkdown-root]；单处匹配按 Enter 时选区不变，PM 会丢弃仅 scrollIntoView 的事务
 */
function scrollFindMatchIntoMilkdownRoot(view: import("@milkdown/prose/view").EditorView, from: number, to: number): void {
  try {
    const doc = view.state.doc;
    const size = doc.content.size;
    const a = Math.max(1, Math.min(from, size));
    const b = Math.max(1, Math.min(to, size));
    const mid = Math.min(Math.floor((a + b) / 2), size);
    const coords = view.coordsAtPos(mid);
    const root = view.dom.closest("[data-milkdown-root]") as HTMLElement | null;
    if (!root) {
      return;
    }
    const rect = root.getBoundingClientRect();
    const centerY = (coords.top + coords.bottom) / 2;
    const nextTop = centerY - rect.top + root.scrollTop - root.clientHeight / 2;
    const maxTop = Math.max(0, root.scrollHeight - root.clientHeight);
    root.scrollTop = Math.max(0, Math.min(nextTop, maxTop));
  } catch {
    // 复杂节点上 coords 可能不可用
  }
}

export function findScrollToCurrent(view: import("@milkdown/prose/view").EditorView): void {
  const st = FIND_PLUGIN_KEY.getState(view.state) as FindPluginState | undefined;
  if (!st?.ranges.length) {
    return;
  }
  const { from, to } = st.ranges[st.index];
  if (from >= to) {
    return;
  }
  const sel = TextSelection.create(view.state.doc, from, to);
  const tr = view.state.tr.setSelection(sel).scrollIntoView();
  if (!sel.eq(view.state.selection)) {
    view.dispatch(tr);
  }
  // 不调用 view.focus()：篇内查找条需保持焦点在输入框
  requestAnimationFrame(() => {
    scrollFindMatchIntoMilkdownRoot(view, from, to);
  });
}
