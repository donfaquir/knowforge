/**
 * 为正文中的 `[[wikilink]]` / `![[embed]]` 加 ProseMirror inline decoration，便于用 CSS 做成「类链接」样式。
 */
import type { Node as PmNode, ResolvedPos } from "@milkdown/prose/model";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { Decoration, DecorationSet } from "@milkdown/prose/view";
import { $prose } from "@milkdown/utils";
import { isCodeBlockContainerNodeName } from "../utils/milkdownCodeBlockNodeName";
import { WIKI_LINK_BODY } from "../utils/wikiLinkBodyRegex";

export const wikiLinkDecoratePluginKey = new PluginKey("kf-wikilink-deco");

/** 模块内单例，避免每个文本节点 `new RegExp`；每次 `exec` 循环前须 `lastIndex = 0` */
const WIKI_LINK_G = new RegExp(WIKI_LINK_BODY.source, "g");

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

function collectWikilinkDecorations(doc: PmNode): Decoration[] {
  const out: Decoration[] = [];
  doc.nodesBetween(0, doc.content.size, (node, pos) => {
    if (!node.isText) {
      return;
    }
    const text = node.text;
    if (!text || !text.includes("[[")) {
      return;
    }
    if (node.marks.some((m) => m.type.name === "link")) {
      return;
    }
    if (node.marks.some((m) => m.type.name === "code_inline")) {
      return;
    }
    const base = pos + 1;
    WIKI_LINK_G.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = WIKI_LINK_G.exec(text)) !== null) {
      const isEmbed = m.index > 0 && text[m.index - 1] === "!";
      const startOff = isEmbed ? m.index - 1 : m.index;
      const endOff = m.index + m[0].length;
      const from = base + startOff;
      const to = base + endOff;
      if (from >= to || to > doc.content.size) {
        continue;
      }
      try {
        const $f = doc.resolve(from);
        if (isUnderCodeOrLiteral($f)) {
          continue;
        }
      } catch {
        continue;
      }
      out.push(Decoration.inline(from, to, { class: "kf-wikilink" }));
    }
  });
  return out;
}

function buildDecoSet(doc: PmNode): DecorationSet {
  const list = collectWikilinkDecorations(doc);
  if (list.length === 0) {
    return DecorationSet.empty;
  }
  return DecorationSet.create(doc, list);
}

export const wikiLinkDecoratePlugin = $prose(() => {
  return new Plugin({
    key: wikiLinkDecoratePluginKey,
    state: {
      init: (_, { doc }) => buildDecoSet(doc),
      apply(tr, deco, _old, newState) {
        if (!tr.docChanged) {
          return deco;
        }
        return buildDecoSet(newState.doc);
      },
    },
    props: {
      decorations(state) {
        return wikiLinkDecoratePluginKey.getState(state) ?? null;
      },
    },
  });
});
