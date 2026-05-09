import type { Node } from "@milkdown/prose/model";
import { splitFrontmatterMarkdown } from "./kfPrivateFrontmatterCore";
import { getMarkdownBodyForEditor } from "./kfPrivateFrontmatterEdit";

/**
 * 闭合 frontmatter 之后、正文首字符在全文中的字节偏移（非 closed 返回 null）
 */
export function bodyStartByteOffsetInFull(fullMarkdown: string): number | null {
  const sp = splitFrontmatterMarkdown(fullMarkdown);
  if (sp.kind !== "closed") {
    return null;
  }
  const leadLen = sp.leading.length;
  const trimmed = fullMarkdown.slice(leadLen);
  const lines = trimmed.split("\n");
  for (let i = 1; i < lines.length; i++) {
    if ((lines[i] ?? "").trim().replace(/\r$/, "") === "---") {
      const throughClosing = lines.slice(0, i + 1).join("\n");
      return leadLen + throughClosing.length + 1;
    }
  }
  return null;
}

function line1BasedAtByteOffset(fullMarkdown: string, byteOffset: number): number {
  const prefix = fullMarkdown.slice(0, Math.min(byteOffset, fullMarkdown.length));
  return prefix.split(/\r?\n/).length;
}

/** 去掉行首常见 Markdown 语法，便于在 WYSIWYG 纯文本中匹配 */
function stripCommonMarkdownLinePrefix(line: string): string {
  let s = line.trim();
  s = s.replace(/^#{1,6}\s+/, "");
  s = s.replace(/^\[[ xX]\]\s+/, "");
  s = s.replace(/^\s*[-*+]\s+(\[[ xX]\]\s+)?/, "");
  s = s.replace(/^\s*\d+[.)]\s+/, "");
  s = s.replace(/^>\s+/, "");
  s = s.replace(/^`{3,}\S*/, "");
  return s.trim();
}

/**
 * 根据全文 1-based 行号构造若干候选子串，用于在 ProseMirror 文档中定位滚动目标
 */
export function buildPreviewScrollNeedles(fullMarkdown: string, hitLine1Based: number): string[] {
  const lines = fullMarkdown.split(/\r?\n/);
  const idx = hitLine1Based - 1;
  const sp = splitFrontmatterMarkdown(fullMarkdown);

  let primaryLine = (lines[idx] ?? "").replace(/\r$/, "");

  if (sp.kind === "closed") {
    const bodyOff = bodyStartByteOffsetInFull(fullMarkdown);
    if (bodyOff != null) {
      const firstBodyLine1 = line1BasedAtByteOffset(fullMarkdown, bodyOff);
      if (hitLine1Based < firstBodyLine1) {
        const bodyText = getMarkdownBodyForEditor(fullMarkdown);
        const bodyLines = bodyText.split(/\r?\n/);
        primaryLine = bodyLines.find((l) => l.replace(/\r$/, "").trim().length > 0) ?? "";
      }
    }
  }

  const trimmed = primaryLine.trim();
  const stripped = stripCommonMarkdownLinePrefix(primaryLine);
  const candidates = [stripped, trimmed];
  if (stripped.length > 24) {
    candidates.push(stripped.slice(0, 48));
  }
  if (trimmed.length > 24) {
    candidates.push(trimmed.slice(0, 48));
  }
  const out: string[] = [];
  const seen = new Set<string>();
  for (const c of candidates) {
    const n = c.trim();
    if (n.length < 2) {
      continue;
    }
    if (seen.has(n)) {
      continue;
    }
    seen.add(n);
    out.push(n);
  }
  return out;
}

/** 在文档文本节点中按候选串依次查找首次出现位置（不区分大小写） */
export function findFirstDocMatchForNeedles(doc: Node, needles: string[]): number | null {
  for (const needle of needles) {
    const n = needle.trim();
    if (n.length < 2) {
      continue;
    }
    const lower = n.toLowerCase();
    let found: number | null = null;
    doc.descendants((node, pos) => {
      if (found != null || !node.isText || !node.text) {
        return;
      }
      const idx = node.text.toLowerCase().indexOf(lower);
      if (idx >= 0) {
        found = pos + idx;
      }
    });
    if (found != null) {
      return found;
    }
  }
  return null;
}
