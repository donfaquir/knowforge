import { parse, stringify } from "yaml";
import { splitFrontmatterMarkdown } from "./kfPrivateFrontmatterCore";

function isAtxHeadingLine(line: string): boolean {
  return /^#{1,6}(?:\s|$)/.test(line.trim());
}

/** 将闭合块内的 YAML 文本解析为 mapping；空、非 mapping、解析失败返回 null */
function tryParseYamlRecord(yamlText: string): Record<string, unknown> | null {
  const t = yamlText.trim();
  if (t === "") {
    return {};
  }
  try {
    const v = parse(yamlText) as unknown;
    if (v === null || typeof v !== "object" || Array.isArray(v)) {
      return null;
    }
    return { ...(v as Record<string, unknown>) };
  } catch {
    return null;
  }
}

function fenceFromRecord(rec: Record<string, unknown>): string {
  const y = stringify(rec, { lineWidth: 120 }).replace(/\s+$/, "");
  if (y === "" || y === "null" || y === "{}") {
    return "---\n---\n";
  }
  return `---\n${y}\n---\n`;
}

/**
 * 未闭合 frontmatter 内：第一个 ATX 标题行起视为正文；否则整段 inner 当作正文保留（避免误删仅有 YAML 行且无标题的草稿）。
 */
function malformedParts(innerLines: string[]): { yamlPart: string; bodyPart: string } {
  const h = innerLines.findIndex((l) => isAtxHeadingLine(l));
  if (h < 0) {
    return { yamlPart: innerLines.join("\n"), bodyPart: "" };
  }
  return {
    yamlPart: innerLines.slice(0, h).join("\n"),
    bodyPart: innerLines.slice(h).join("\n"),
  };
}

/** 修复此前 Crepe 丢 frontmatter 时多次点击「私密」堆叠的相同文首块，仅折叠与本工具插入格式一致的重复段 */
function collapseDuplicateKfPrivateIntroBlocks(markdown: string): string {
  const m = markdown.match(/^([\s\uFEFF]*)/);
  const lead = m?.[1] ?? "";
  let rest = markdown.slice(lead.length);
  const unit = "---\nkf-private: true\n---\n\n";
  let n = 0;
  while (rest.startsWith(unit)) {
    rest = rest.slice(unit.length);
    n += 1;
  }
  if (n <= 1) {
    return markdown;
  }
  return lead + unit + rest;
}

/**
 * 在 Markdown 全文上设置或清除 `kf-private`（标准 YAML frontmatter）。
 * 与出站隐私判定对齐：解析失败或非 mapping 时「去私密」会去掉整块 frontmatter，仅保留正文侧内容。
 */
export function setMarkdownKfPrivate(markdown: string, wantPrivate: boolean): string {
  const md = collapseDuplicateKfPrivateIntroBlocks(markdown);
  const sp = splitFrontmatterMarkdown(md);

  if (sp.kind === "none") {
    if (!wantPrivate) {
      return md;
    }
    return `${sp.leading}---\nkf-private: true\n---\n\n${sp.rest}`;
  }

  if (sp.kind === "malformed") {
    const { yamlPart, bodyPart } = malformedParts(sp.innerLines);
    if (wantPrivate) {
      const base = tryParseYamlRecord(yamlPart);
      if (base) {
        base["kf-private"] = true;
        const tail = bodyPart === "" ? "" : bodyPart;
        return `${sp.leading}${fenceFromRecord(base)}${tail}`;
      }
      // inner 无法解析为 mapping：在文首插入标准块，原 inner 全文保留在正文侧（避免丢稿）
      return `${sp.leading}---\nkf-private: true\n---\n\n${sp.innerLines.join("\n")}`;
    }
    // 去私密：丢弃无法闭合的元数据区，保留推断出的正文
    if (bodyPart !== "") {
      return `${sp.leading}${bodyPart}`;
    }
    return `${sp.leading}${yamlPart}`;
  }

  const yamlText = sp.yamlLines.join("\n");
  const rec = tryParseYamlRecord(yamlText);
  let body = sp.body;

  if (!wantPrivate) {
    if (!rec) {
      return `${sp.leading}${body}`;
    }
    const next = { ...rec };
    delete next["kf-private"];
    if (Object.keys(next).length === 0) {
      return `${sp.leading}${body}`;
    }
    return `${sp.leading}${fenceFromRecord(next)}${body}`;
  }

  if (!rec) {
    return `${sp.leading}---\nkf-private: true\n---\n\n${body}`;
  }
  const next = { ...rec, "kf-private": true };
  return `${sp.leading}${fenceFromRecord(next)}${body}`;
}

/**
 * 首段已闭合的 YAML frontmatter（含 `kf-private: true` 等）不在 Crepe 所见即所得中展示；缓冲区仍存完整全文。
 * 未闭合 frontmatter 原样返回，避免误藏正文。
 */
export function getMarkdownBodyForEditor(fullMarkdown: string): string {
  const sp = splitFrontmatterMarkdown(fullMarkdown);
  if (sp.kind === "closed") {
    return `${sp.leading}${sp.body}`;
  }
  if (sp.kind === "none") {
    return `${sp.leading}${sp.rest}`;
  }
  return fullMarkdown;
}

/**
 * Crepe/Milkdown 的 getMarkdown 通常不包含 YAML frontmatter。
 * 用缓冲区上一版全文里已闭合的那段 frontmatter + 本轮编辑器产出拼回，避免 kf-private 等键被 baseline 静默抹掉。
 */
export function mergeEditorMarkdownWithStoredFrontmatter(
  prevFullMarkdown: string,
  editorMarkdown: string,
): string {
  const sp = splitFrontmatterMarkdown(prevFullMarkdown);
  if (sp.kind !== "closed") {
    return editorMarkdown;
  }
  const yamlInner = sp.yamlLines.join("\n");
  return `${sp.leading}---\n${yamlInner}\n---\n${editorMarkdown}`;
}
