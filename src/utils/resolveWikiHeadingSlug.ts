import GithubSlugger from "github-slugger";
import { extractOutline, type OutlineItem } from "./extractOutline";

/** 在已解析的大纲上尝试三种匹配（不处理 `##` 嵌套）；供外层循环复用同一 outline */
function lookupSlugInOutline(outline: OutlineItem[], raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) {
    return null;
  }
  const lower = trimmed.toLowerCase();
  const solo = new GithubSlugger().slug(trimmed);

  let byText: string | null = null;
  let bySlug: string | null = null;
  let bySolo: string | null = null;
  for (const item of outline) {
    if (byText === null && item.text.trim().toLowerCase() === lower) {
      byText = item.slug;
    }
    if (bySlug === null && item.slug.toLowerCase() === lower) {
      bySlug = item.slug;
    }
    if (bySolo === null && item.slug === solo) {
      bySolo = item.slug;
    }
    if (byText !== null) {
      break;
    }
  }
  return byText ?? bySlug ?? bySolo;
}

/**
 * 将 wikilink / 内链里 # 后的片段解析为与 extractOutline、正文 DOM 一致的 GitHub 风格 slug，
 * 对齐 Obsidian 常见用法：可写标题原文、可写 slug（含重复标题的 -1、-2 后缀）。
 *
 * @param markdownBody 与编辑器一致的 Markdown 正文（不含 frontmatter 时与 getMarkdownBodyForEditor 一致）
 * @param fragment # 之后、decodeURIComponent 后的用户片段
 */
export function resolveWikiHeadingSlug(markdownBody: string, fragment: string): string | null {
  const raw = fragment.trim();
  if (!raw) {
    return null;
  }
  const outline = extractOutline(markdownBody);
  if (outline.length === 0) {
    return null;
  }

  // Obsidian `父##子` 链式剥离上限，避免恶意超长串与无界循环
  const MAX_NEST_SEGMENTS = 64;
  let current = raw;
  for (let i = 0; i < MAX_NEST_SEGMENTS; i += 1) {
    const hit = lookupSlugInOutline(outline, current);
    if (hit !== null) {
      return hit;
    }
    if (!current.includes("##")) {
      return null;
    }
    const tail = current.split("##").pop()?.trim() ?? "";
    if (!tail || tail === current) {
      return null;
    }
    current = tail;
  }
  return null;
}
