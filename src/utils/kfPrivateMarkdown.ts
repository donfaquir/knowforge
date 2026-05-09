import { parse } from "yaml";
import { splitFrontmatterMarkdown } from "./kfPrivateFrontmatterCore";

/** Frontmatter 内 YAML 段 UTF-8 字节上限；超出则不解析，避免恶意超大输入拖垮主线程 */
const MAX_FRONTMATTER_YAML_UTF8_BYTES = 256 * 1024;

/** 解析选项：收紧别名展开，与库默认相比略保守（正常 frontmatter 不用别名） */
const YAML_PARSE_OPTIONS = { maxAliasCount: 32 } as const;

/**
 * 统计 UTF-8 字节数，超过 `limit` 即返回 true；不整段 TextEncoder，便于早停。
 */
function utf8ExceedsByteLimit(s: string, limit: number): boolean {
  let n = 0;
  for (let i = 0; i < s.length; ) {
    const cp = s.codePointAt(i)!;
    if (cp <= 0x7f) {
      n += 1;
    } else if (cp <= 0x7ff) {
      n += 2;
    } else if (cp <= 0xffff) {
      n += 3;
    } else {
      n += 4;
    }
    if (n > limit) {
      return true;
    }
    i += cp > 0xffff ? 2 : 1;
  }
  return false;
}

/**
 * 与 `src-tauri/src/note_privacy.rs` 中 frontmatter 抽取及 `kf-private` 判定尽量一致；
 * 仅供 UI 提示，**出站裁决以 Rust 为准**。
 */
function splitFrontmatterYamlOwned(src: string): { ok: true; yaml: string | null } | { ok: false } {
  const sp = splitFrontmatterMarkdown(src);
  if (sp.kind === "malformed") {
    return { ok: false };
  }
  if (sp.kind === "none") {
    return { ok: true, yaml: null };
  }
  return { ok: true, yaml: sp.yamlLines.join("\n") };
}

/** 与 Rust `markdown_treat_as_kf_private` 对齐，用于发送前 UI 提示。 */
export function markdownTreatAsKfPrivateForUi(markdown: string): boolean {
  const split = splitFrontmatterYamlOwned(markdown);
  if (!split.ok) {
    return true;
  }
  if (split.yaml === null) {
    return false;
  }
  const trimmed = split.yaml.trim();
  if (trimmed === "") {
    return false;
  }
  if (utf8ExceedsByteLimit(trimmed, MAX_FRONTMATTER_YAML_UTF8_BYTES)) {
    // 与解析失败相同：无法确认非私密，UI 侧保守提示
    return true;
  }
  try {
    const v = parse(trimmed, YAML_PARSE_OPTIONS) as unknown;
    if (v === null || typeof v !== "object" || Array.isArray(v)) {
      return true;
    }
    const rec = v as Record<string, unknown>;
    if (!Object.prototype.hasOwnProperty.call(rec, "kf-private")) {
      return false;
    }
    const flag = rec["kf-private"];
    if (flag === true) {
      return true;
    }
    if (flag === false) {
      return false;
    }
    return true;
  } catch {
    return true;
  }
}
