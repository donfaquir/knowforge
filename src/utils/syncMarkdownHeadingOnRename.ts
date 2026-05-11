import { splitFrontmatterMarkdown } from "./kfPrivateFrontmatterCore";
import { displayHeadingStemFromBasename } from "./newUntitledMarkdownPath";

function isAtxHeadingLine(line: string): boolean {
  return /^#{1,6}(?:\s|$)/.test(line.trim());
}

/** 用于与 displayHeadingStemFromBasename 可比：正文里 # 后标题的规范化 */
function sanitizeHeadingFromMarkdownLine(textAfterHash: string): string {
  return textAfterHash
    .replace(/\r\n|\n|\r|\t/g, " ")
    .replace(/ +/g, " ")
    .trim();
}

/** 将 body 首行 ATX H1（仅 `#`，非 `##`）在标题与旧 stem 一致时替换为新 stem；否则原样返回 */
function replaceFirstH1IfMatchesOldStem(
  body: string,
  oldBasename: string,
  newBasename: string,
): string {
  const oldStem = displayHeadingStemFromBasename(oldBasename);
  const newStem = displayHeadingStemFromBasename(newBasename);
  const lines = body.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? "";
    const m = line.match(/^#\s+(.*)$/);
    if (!m) {
      continue;
    }
    const candidate = sanitizeHeadingFromMarkdownLine(m[1] ?? "");
    if (candidate !== oldStem) {
      return body;
    }
    lines[i] = `# ${newStem}`;
    return lines.join("\n");
  }
  return body;
}

/**
 * 切出「frontmatter 前缀 + 正文起始」后，仅在首行 H1 与旧文件名 stem 一致时替换为新 stem。
 * 与 `initialMarkdownFromBasename` 使用同一套 stem 规则，避免误改用户自定义标题。
 */
function splitPrefixBodyForHeadingEdit(markdown: string): { prefix: string; body: string } | null {
  const sp = splitFrontmatterMarkdown(markdown);
  const trimmedFull = markdown.trimStart();
  const leading = markdown.slice(0, markdown.length - trimmedFull.length);

  if (sp.kind === "closed") {
    const { body } = sp;
    const headInTrimmed = trimmedFull.slice(0, trimmedFull.length - body.length);
    return { prefix: leading + headInTrimmed, body };
  }
  if (sp.kind === "none") {
    return { prefix: leading, body: sp.rest };
  }
  const inner = sp.innerLines;
  const h = inner.findIndex((l) => isAtxHeadingLine(l));
  if (h < 0) {
    return null;
  }
  const beforeHeading = inner.slice(0, h).join("\n");
  const body = inner.slice(h).join("\n");
  const prefix =
    leading +
    "---\n" +
    (beforeHeading.length > 0 ? `${beforeHeading}\n` : "");
  return { prefix, body };
}

/** 相对路径是否为 Markdown 文件（仅扩展名） */
export function isMarkdownRelPath(relPath: string): boolean {
  const lower = relPath.toLowerCase();
  return lower.endsWith(".md") || lower.endsWith(".markdown");
}

/**
 * 重命名后同步首行 H1：仅当正文首个 `# ` 标题与旧基础名 stem 一致时替换为新 stem。
 */
export function syncMarkdownHeadingAfterRename(
  markdown: string,
  oldBasename: string,
  newBasename: string,
): string {
  const split = splitPrefixBodyForHeadingEdit(markdown);
  if (!split) {
    return markdown;
  }
  const nextBody = replaceFirstH1IfMatchesOldStem(split.body, oldBasename, newBasename);
  if (nextBody === split.body) {
    return markdown;
  }
  return split.prefix + nextBody;
}
