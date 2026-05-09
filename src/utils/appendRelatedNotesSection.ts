/** 与迭代 6.3 文档一致：文末「相关笔记」二级标题（兼容英文标题） */

const SECTION_HEADINGS = ["## 相关笔记", "## Related notes"] as const;

/** 判断是否为「独占一行的二级标题」（`## xxx`，排除 `###`） */
function isH2HeadingLine(line: string): boolean {
  return /^##\s+\S/.test(line);
}

/**
 * 在「相关笔记」小节末尾追加一条 `- [[wikiInner]]`；无小节则创建。
 * @returns 新全文；若已存在相同 `[[wikiInner]]` 则返回 `null`（幂等跳过）
 */
export function appendRelatedNotesWikiLinkToMarkdown(markdown: string, wikiInner: string): string | null {
  const inner = wikiInner.trim().replace(/\\/g, "/");
  if (!inner) {
    return null;
  }
  const token = `[[${inner}]]`;
  if (markdown.includes(token)) {
    return null;
  }
  const bullet = `- ${token}`;
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");

  let sectionStart = -1;
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if ((SECTION_HEADINGS as readonly string[]).includes(t)) {
      sectionStart = i;
      break;
    }
  }

  if (sectionStart === -1) {
    const base = markdown.trimEnd();
    const sep = base.length > 0 ? "\n\n" : "";
    return `${base}${sep}${SECTION_HEADINGS[0]}\n${bullet}\n`;
  }

  let endExclusive = lines.length;
  for (let j = sectionStart + 1; j < lines.length; j++) {
    const line = lines[j];
    if (isH2HeadingLine(line)) {
      const tt = line.trim();
      if (!(SECTION_HEADINGS as readonly string[]).includes(tt)) {
        endExclusive = j;
        break;
      }
    }
  }

  const upper = lines.slice(0, endExclusive).join("\n").trimEnd();
  const lower = lines.slice(endExclusive).join("\n");
  if (!lower.trim()) {
    return `${upper}\n${bullet}\n`;
  }
  return `${upper}\n${bullet}\n${lower}`;
}
