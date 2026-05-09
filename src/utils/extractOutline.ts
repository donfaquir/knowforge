import GithubSlugger from "github-slugger";
import type { Heading, Root } from "mdast";
import { toString } from "mdast-util-to-string";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { visit } from "unist-util-visit";

export type OutlineItem = {
  level: number;
  /** 纯文本标题（与 Preview 中用于生成 id 的文案一致） */
  text: string;
  slug: string;
};

/** 大纲提取缓存：同一正文在文档切换往返时复用解析结果，降低主线程 AST 开销 */
const OUTLINE_CACHE_MAX_ENTRIES = 32;
/** 仅缓存中小文档，避免超大正文占用过多内存 */
const OUTLINE_CACHE_MAX_SOURCE_LENGTH = 300_000;
const outlineCache = new Map<string, OutlineItem[]>();

function readOutlineCache(markdown: string): OutlineItem[] | null {
  if (markdown.length > OUTLINE_CACHE_MAX_SOURCE_LENGTH) {
    return null;
  }
  const cached = outlineCache.get(markdown);
  if (!cached) {
    return null;
  }
  // LRU：命中后刷新到末尾
  outlineCache.delete(markdown);
  outlineCache.set(markdown, cached);
  return cached;
}

function writeOutlineCache(markdown: string, outline: OutlineItem[]): void {
  if (markdown.length > OUTLINE_CACHE_MAX_SOURCE_LENGTH) {
    return;
  }
  if (outlineCache.has(markdown)) {
    outlineCache.delete(markdown);
  }
  outlineCache.set(markdown, outline);
  if (outlineCache.size > OUTLINE_CACHE_MAX_ENTRIES) {
    const oldestKey = outlineCache.keys().next().value;
    if (typeof oldestKey === "string") {
      outlineCache.delete(oldestKey);
    }
  }
}

/**
 * 从 Markdown 源码提取标题；slug 用于大纲项标识，跳转时按序号对应 WYSIWYG 中的标题节点。
 */
export function extractOutline(markdown: string): OutlineItem[] {
  const cached = readOutlineCache(markdown);
  if (cached) {
    return cached;
  }
  const tree = unified().use(remarkParse).use(remarkGfm).parse(markdown) as Root;
  const slugger = new GithubSlugger();
  const items: OutlineItem[] = [];

  visit(tree, "heading", (node: Heading) => {
    const text = toString(node);
    items.push({
      level: node.depth,
      text,
      slug: slugger.slug(text),
    });
  });

  writeOutlineCache(markdown, items);
  return items;
}
