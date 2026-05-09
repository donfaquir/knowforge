/**
 * Obsidian 风格 wikilink：`[[title]]` 正文（内层至少一个非 `]` 字符）。
 * 供装饰插件与 remark-stringify 等复用；循环匹配请使用 `new RegExp(WIKI_LINK_BODY.source, "g")`，避免共享带 `g` 实例的 `lastIndex`。
 */
export const WIKI_LINK_BODY = /\[\[([^\]]+)\]\]/;
