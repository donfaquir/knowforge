/**
 * Thought（理解区块）前端操作工具——封装 IPC 调用 + 客户端侧 frontmatter 合并。
 * 与 Rust thought_parser.rs 的 camelCase JSON 协议对齐。
 */

import { invoke } from "@tauri-apps/api/core";
import { parse, stringify } from "yaml";
import { splitFrontmatterMarkdown } from "./kfPrivateFrontmatterCore";
import type {
  InsertThoughtArgs,
  InsertThoughtResponse,
  KfThoughtMeta,
  ParseNoteThoughtsResponse,
} from "../types/cognitiveTypes";

// --- IPC wrappers ---

/** 解析笔记中的 kf-thoughts 元数据 + 侧车正文摘录（磁盘 + `.knowforge/thoughts/index.sqlite`）。 */
export async function parseNoteThoughts(
  relPath: string,
): Promise<ParseNoteThoughtsResponse> {
  return invoke<ParseNoteThoughtsResponse>("parse_note_thoughts", {
    relPath,
  });
}

/** 在笔记中插入新想法：更新 YAML 与侧车 SQLite，不写正文 callout（磁盘写入）。 */
export async function insertThoughtToNote(
  args: InsertThoughtArgs,
): Promise<InsertThoughtResponse> {
  return invoke<InsertThoughtResponse>("insert_thought_to_note", { args });
}

// --- 客户端侧 frontmatter 合并（编辑器内存缓冲区场景） ---

type YamlRecord = Record<string, unknown>;

function tryParseYamlRecord(yamlText: string): YamlRecord | null {
  const t = yamlText.trim();
  if (t === "") return {};
  try {
    const v = parse(yamlText) as unknown;
    if (v === null || typeof v !== "object" || Array.isArray(v)) return null;
    return { ...(v as YamlRecord) };
  } catch {
    return null;
  }
}

/**
 * 将一条 KfThoughtMeta 合并到 Markdown 全文的 frontmatter 中（按 id 去重）。
 * 用于编辑器缓冲区原地更新，无需磁盘 round-trip。
 */
export function mergeThoughtMetaIntoMarkdown(
  markdown: string,
  meta: KfThoughtMeta,
): string {
  const sp = splitFrontmatterMarkdown(markdown);

  if (sp.kind === "none") {
    // 无 frontmatter，创建新的
    const yaml = stringify({ "kf-thoughts": [meta] }, { lineWidth: 120 }).replace(/\s+$/, "");
    return `${sp.leading}---\n${yaml}\n---\n${sp.rest}`;
  }

  if (sp.kind === "malformed") {
    // 未闭合 frontmatter，不冒险修改
    return markdown;
  }

  // closed frontmatter
  const yamlText = sp.yamlLines.join("\n");
  const rec = tryParseYamlRecord(yamlText);
  if (!rec) return markdown;

  const existing = Array.isArray(rec["kf-thoughts"])
    ? (rec["kf-thoughts"] as KfThoughtMeta[])
    : [];

  const idx = existing.findIndex((t) => t.id === meta.id);
  if (idx >= 0) {
    existing[idx] = meta;
  } else {
    existing.push(meta);
  }
  rec["kf-thoughts"] = existing;

  const newYaml = stringify(rec, { lineWidth: 120 }).replace(/\s+$/, "");
  return `${sp.leading}---\n${newYaml}\n---\n${sp.body}`;
}
