/**
 * 当前编辑笔记相对路径，供正文内 `[[wikilink]]` 点击解析（resolveWikiHref）使用。
 * 在注入 Markdown 前由 CrepeMarkdownEditor 同步更新。
 */
export const wikiLinkContextRef = { currentRelPath: "" as string };
