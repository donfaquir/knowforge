/**
 * Milkdown 插件：为 `> [!thought]` callout 区块添加 CSS 装饰类名。
 * 基于 ProseMirror Decoration API，不修改 schema / markdown AST。
 */

import { $prose } from "@milkdown/utils";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { Decoration, DecorationSet } from "@milkdown/prose/view";
import type { Node as PmNode } from "@milkdown/prose/model";
import { THOUGHT_CALLOUT_LINE_PREFIX } from "../utils/thoughtCalloutConstants";

// --- 与 Markdown / CSS / 后端 thought 解析对齐的常量 ---

const PLUGIN_DECORATION_KEY = "thought-callout-decorations";

/** ProseMirror blockquote 节点类型名 */
const PM_NODE_BLOCKQUOTE = "blockquote";

/** `[` 转义为 `\[` 时 PM 文本以 `\[!thought` 开头（与后端 thought_parser 对齐） */
const THOUGHT_CALLOUT_ESCAPED_PREFIX = "\\[!thought";

function thoughtTypeSuffixBoundaryOk(tail: string): boolean {
  if (tail.length === 0) return true;
  const c = tail[0];
  return c === "]" || c === "|" || c === " " || c === "\t";
}

/** 成熟态：callout 标题行中的 growing 线索（emoji） */
const MATURITY_GROWING_EMOJI = "🌿";
const MATURITY_MATURE_EMOJI = "🌳";

const MATURITY_SEEDLING = "seedling";
const MATURITY_GROWING = "growing";
const MATURITY_MATURE = "mature";

const CSS_THOUGHT_CALLOUT = "thought-callout";

const thoughtCalloutPluginKey = new PluginKey(PLUGIN_DECORATION_KEY);

type ThoughtMaturityClassSuffix =
  | typeof MATURITY_SEEDLING
  | typeof MATURITY_GROWING
  | typeof MATURITY_MATURE;

/**
 * 判断 blockquote 节点首段文本是否以 thought callout 标记开头（不区分大小写）。
 * 返回 maturity，用于 `thought-callout--*` 类名后缀。
 */
function detectThoughtCallout(
  blockquoteNode: { firstChild: { textContent?: string } | null },
): ThoughtMaturityClassSuffix | null {
  const firstChild = blockquoteNode.firstChild;
  if (!firstChild) return null;
  const text = (firstChild.textContent ?? "").trimStart().toLowerCase();
  let tail: string | null = null;
  if (text.startsWith(THOUGHT_CALLOUT_LINE_PREFIX)) {
    tail = text.slice(THOUGHT_CALLOUT_LINE_PREFIX.length);
  } else if (text.startsWith(THOUGHT_CALLOUT_ESCAPED_PREFIX)) {
    tail = text.slice(THOUGHT_CALLOUT_ESCAPED_PREFIX.length);
  }
  if (tail === null || !thoughtTypeSuffixBoundaryOk(tail)) return null;
  if (text.includes(MATURITY_MATURE_EMOJI) || text.includes(MATURITY_MATURE)) {
    return MATURITY_MATURE;
  }
  if (text.includes(MATURITY_GROWING_EMOJI) || text.includes(MATURITY_GROWING)) {
    return MATURITY_GROWING;
  }
  return MATURITY_SEEDLING;
}

/** Milkdown $prose 插件，可直接 `crepe.editor.use(thoughtCalloutPlugin)` */
export const thoughtCalloutPlugin = $prose(() => {
  return new Plugin({
    key: thoughtCalloutPluginKey,
    state: {
      init(_, state) {
        return buildDecorations(state.doc);
      },
      apply(tr, oldDecoSet) {
        if (!tr.docChanged) return oldDecoSet;
        return buildDecorations(tr.doc);
      },
    },
    props: {
      decorations(state) {
        return this.getState(state);
      },
    },
  });
});

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- PmNode used below
function buildDecorations(doc: PmNode): DecorationSet {
  const decorations: Decoration[] = [];

  doc.descendants((node: PmNode, pos: number) => {
    if (node.type.name !== PM_NODE_BLOCKQUOTE) return;

    const maturity = detectThoughtCallout(node);
    if (!maturity) return;

    decorations.push(
      Decoration.node(pos, pos + node.nodeSize, {
        class: `${CSS_THOUGHT_CALLOUT} ${CSS_THOUGHT_CALLOUT}--${maturity}`,
      }),
    );
  });

  return DecorationSet.create(doc, decorations);
}
