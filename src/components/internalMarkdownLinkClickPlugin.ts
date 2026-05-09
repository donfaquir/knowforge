/**
 * 正文内 Vault 链接：标准 `[text](path)` 与保留原文的 `[[wikilink]]` 左键走应用内打开。
 */
import type { Node as ProseNode } from "@milkdown/prose/model";
import type { EditorView } from "@milkdown/prose/view";
import { Plugin, PluginKey } from "@milkdown/prose/state";
import { $prose } from "@milkdown/utils";
import { resolveWikiHref } from "../utils/wikiLinkResolve";
import { wikiLinkContextRef } from "./wikiLinkContext";

/** 内链打开：可选 headingFragment 为 # 后片段（已 URI 解码） */
export type InternalMarkdownLinkOpenMeta = {
  headingFragment?: string | null;
};

/** 由 CrepeMarkdownEditor 在 layout/effect 中同步，供插件在点击时读取最新回调 */
export const internalMarkdownLinkOpenRef = {
  current: null as null | ((relPath: string, meta?: InternalMarkdownLinkOpenMeta) => void),
};

function isExternalHref(href: string): boolean {
  const h = href.trim().toLowerCase();
  if (!h || h === "#" || h.startsWith("#")) {
    return true;
  }
  return /^(https?:|mailto:|javascript:|data:)/i.test(h) || h.startsWith("//");
}

/** 反斜杠转正斜杠、合并连续 `/`、去掉前缀 `/`，与仓库内其它 Vault 相对路径处理一致 */
function normalizeVaultRelPathSeparators(rel: string): string {
  return rel.replace(/\\/g, "/").replace(/\/+/g, "/").replace(/^\/+/, "");
}

function isSafeVaultRelPath(rel: string): boolean {
  const norm = normalizeVaultRelPathSeparators(rel);
  if (!norm || norm === ".") {
    return false;
  }
  const parts = norm.split("/");
  return !parts.some((p) => p === ".." || p === "");
}

/** 将 href 还原为 Vault 相对路径与可选标题片段；非 Vault 内 Markdown 目标返回 null */
export function parseVaultInternalMarkdownHref(href: string): {
  relPath: string;
  headingFragment: string | null;
} | null {
  const raw = href.trim();
  if (!raw || isExternalHref(raw)) {
    return null;
  }
  if (raw.includes("://")) {
    return null;
  }
  let decoded: string;
  try {
    decoded = raw.split("/").map((s) => decodeURIComponent(s)).join("/");
  } catch {
    return null;
  }
  const unixFull = normalizeVaultRelPathSeparators(decoded);
  const hashIdx = unixFull.indexOf("#");
  const pathPart = hashIdx >= 0 ? unixFull.slice(0, hashIdx) : unixFull;
  let headingFragment: string | null = null;
  if (hashIdx >= 0) {
    const fragRaw = unixFull.slice(hashIdx + 1);
    try {
      headingFragment = decodeURIComponent(fragRaw).trim() || null;
    } catch {
      headingFragment = fragRaw.trim() || null;
    }
  }
  if (!isSafeVaultRelPath(pathPart)) {
    return null;
  }
  const base = pathPart.split("/").pop() ?? "";
  if (!/\.md$/i.test(base)) {
    return null;
  }
  return { relPath: pathPart, headingFragment };
}

/** 与理解网络、remark 旧逻辑一致：跳过 `![[`、含 `|` 的别名仍解析目标（Obsidian 可点别名） */
const WIKI_LINK_RE = /\[\[([^\]|]+?)(?:\|([^\]]+?))?\]\]/g;

/**
 * 点击是否落在 wikilink 装饰 DOM 上（含其内部文本）；
 * 用 DOM 判定避免仅靠 posAtCoords 把邻近像素吸附进 [[...]]；不用「仅 Text」判断，避免部分环境下 elementFromPoint 命中 span 导致永远不跳转。
 */
function isClickOnWikilinkDecoration(event: MouseEvent, view: EditorView): boolean {
  const root = view.dom;
  const owner = root.ownerDocument ?? document;

  const chainContainsWikilink = (node: Node | null): boolean => {
    const start = node instanceof Element ? node : node instanceof Text ? node.parentElement : null;
    const span = start?.closest(".kf-wikilink");
    return !!(span && root.contains(span));
  };

  const top = owner.elementFromPoint(event.clientX, event.clientY);
  if (top && chainContainsWikilink(top)) {
    return true;
  }
  const t = event.target;
  if (t instanceof Node && chainContainsWikilink(t)) {
    return true;
  }
  return false;
}

function findInlineBlockBounds(doc: ProseNode, clickPos: number): { start: number; end: number } | null {
  const $pos = doc.resolve(clickPos);
  for (let d = $pos.depth; d > 0; d--) {
    if ($pos.node(d).inlineContent) {
      return { start: $pos.start(d), end: $pos.end(d) };
    }
  }
  return null;
}

/** 点击位置在块内扁平纯文本中的字符下标；不在任何 text 叶子上时为 -1 */
function clickOffsetInFlattenedInline(doc: ProseNode, blockStart: number, blockEnd: number, clickPos: number): number {
  let acc = 0;
  let result = -1;
  doc.nodesBetween(blockStart, blockEnd, (node, pos) => {
    if (!node.isText) {
      return;
    }
    const text = node.text ?? "";
    const first = pos + 1;
    const lastExclusive = pos + 1 + text.length;
    if (result < 0 && clickPos >= first && clickPos < lastExclusive) {
      result = acc + (clickPos - first);
    }
    acc += text.length;
  });
  return result;
}

function flattenInlineText(doc: ProseNode, blockStart: number, blockEnd: number): string {
  let s = "";
  doc.nodesBetween(blockStart, blockEnd, (node) => {
    if (node.isText) {
      s += node.text ?? "";
    }
  });
  return s;
}

function tryOpenWikilinkAtClick(view: EditorView, event: MouseEvent): boolean {
  if (!isClickOnWikilinkDecoration(event, view)) {
    return false;
  }
  const coords = view.posAtCoords({ left: event.clientX, top: event.clientY });
  if (!coords) {
    return false;
  }
  const clickPos = coords.pos;
  const bounds = findInlineBlockBounds(view.state.doc, clickPos);
  if (!bounds) {
    return false;
  }
  const off = clickOffsetInFlattenedInline(view.state.doc, bounds.start, bounds.end, clickPos);
  if (off < 0) {
    return false;
  }
  const text = flattenInlineText(view.state.doc, bounds.start, bounds.end);
  if (!text.includes("[[")) {
    return false;
  }
  WIKI_LINK_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = WIKI_LINK_RE.exec(text)) !== null) {
    if (m.index > 0 && text[m.index - 1] === "!") {
      continue;
    }
    const start = m.index;
    const end = start + m[0].length;
    if (off < start || off >= end) {
      continue;
    }
    const target = (m[1] ?? "").trim();
    if (!target) {
      return false;
    }
    const base = wikiLinkContextRef.currentRelPath;
    const href = resolveWikiHref(base, target);
    if (!href) {
      return false;
    }
    const parsed = parseVaultInternalMarkdownHref(href);
    if (!parsed) {
      return false;
    }
    const fn = internalMarkdownLinkOpenRef.current;
    if (!fn) {
      return false;
    }
    event.preventDefault();
    event.stopPropagation();
    fn(parsed.relPath, { headingFragment: parsed.headingFragment });
    return true;
  }
  return false;
}

const pluginKey = new PluginKey("kf-internal-md-link-click");

export const internalMarkdownLinkClickPlugin = $prose(() => {
  return new Plugin({
    key: pluginKey,
    props: {
      handleDOMEvents: {
        click(view, event) {
          const target = event.target;
          if (!(target instanceof HTMLElement)) {
            return false;
          }
          const e = event as MouseEvent;
          if (e.button !== 0) {
            return false;
          }
          if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) {
            return false;
          }
          if (e.defaultPrevented) {
            return false;
          }
          const fn = internalMarkdownLinkOpenRef.current;
          if (!fn) {
            return false;
          }
          const el = target.closest("a");
          if (el instanceof HTMLAnchorElement) {
            const href = el.getAttribute("href") ?? "";
            const parsed = parseVaultInternalMarkdownHref(href);
            if (!parsed) {
              return false;
            }
            e.preventDefault();
            e.stopPropagation();
            fn(parsed.relPath, {
              headingFragment: parsed.headingFragment,
            });
            return true;
          }
          return tryOpenWikilinkAtClick(view, e);
        },
      },
    },
  });
});
