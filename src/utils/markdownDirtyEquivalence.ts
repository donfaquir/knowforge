import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified } from "unified";

/**
 * 脏标记 /「是否与磁盘一致」的语义等价判定（方案说明）：
 * - 纯字符串 === 无法兼容所见即所得编辑器的合法往返差异（列表符、<br />、空行等）。
 * - 用与 Crepe 相近的 remark 管线解析为 mdast，去掉 position 后深比较；
 *   语义相同则视为「无未保存实质变更」，适配多数 Markdown 笔记场景。
 * - 未引入 remark-frontmatter：带复杂 YAML 的极少数边界可能与 Milkdown 不完全一致；
 *   解析失败时保守地视为「不等价」，避免漏报未保存。
 */

const comparePipeline = unified().use(remarkParse).use(remarkGfm).use(remarkBreaks);

function normalizeMarkdownSource(s: string): string {
  let t = s;
  if (t.startsWith("\uFEFF")) {
    t = t.slice(1);
  }
  return t.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

function stripPositions(node: unknown): unknown {
  if (node === null || typeof node !== "object") {
    return node;
  }
  if (Array.isArray(node)) {
    return node.map(stripPositions);
  }
  const o = node as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(o).sort()) {
    if (key === "position") {
      continue;
    }
    out[key] = stripPositions(o[key]);
  }
  return out;
}

/** 缓冲区与「已保存快照」是否语义等价（等价则不应显示未保存） */
export function markdownEquivalentForDirty(a: string, b: string): boolean {
  if (a === b) {
    return true;
  }
  const na = normalizeMarkdownSource(a);
  const nb = normalizeMarkdownSource(b);
  if (na === nb) {
    return true;
  }
  try {
    const ta = stripPositions(comparePipeline.parse(na));
    const tb = stripPositions(comparePipeline.parse(nb));
    return JSON.stringify(ta) === JSON.stringify(tb);
  } catch {
    return false;
  }
}
