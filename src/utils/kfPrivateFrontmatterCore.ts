/**
 * 首段 YAML frontmatter 切分（与 `note_privacy.rs` / 原 `kfPrivateMarkdown` 一致：先 `trimStart` 再解析）。
 * 供 UI 隐私判定、Crepe merge、编辑器正文剥离、kf-private 编辑共用，避免两套行扫描漂移。
 */

export type FrontmatterSplitResult =
  | { kind: "none"; leading: string; rest: string }
  | { kind: "closed"; leading: string; yamlLines: string[]; body: string }
  | { kind: "malformed"; leading: string; innerLines: string[] };

/**
 * 切分首段 frontmatter：`leading` 为全文相对 `trimStart` 之前的字符；`rest`/`body` 均在去掉该前缀后的串上定义。
 * malformed：有起始 `---` 但全文无闭合 `---`。
 */
export function splitFrontmatterMarkdown(src: string): FrontmatterSplitResult {
  const trimmedFull = src.trimStart();
  const leading = src.slice(0, src.length - trimmedFull.length);
  const s = trimmedFull;
  if (!s.startsWith("---")) {
    return { kind: "none", leading, rest: s };
  }
  const lines = s.split("\n");
  const first = (lines[0] ?? "").trim().replace(/\r$/, "");
  if (first !== "---") {
    return { kind: "none", leading, rest: s };
  }
  const yamlLines: string[] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i] ?? "";
    const t = line.trim().replace(/\r$/, "");
    if (t === "---") {
      const body = lines.slice(i + 1).join("\n");
      return { kind: "closed", leading, yamlLines, body };
    }
    yamlLines.push(line);
  }
  return { kind: "malformed", leading, innerLines: yamlLines };
}
