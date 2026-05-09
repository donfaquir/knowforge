/**
 * 将 wikilink 目标解析为 Vault 内相对 Markdown 路径（href）。
 * 含 `/` 视为从 Vault 根起的路径；否则视为当前文件同目录下的笔记名。
 * 支持 `[[笔记#标题]]`、`[[#仅标题]]`：按首个 `#` 拆路径与标题，标题写入 URL fragment。
 *
 * 不包含 `[[target|label]]` 中 `|` 之后别名的处理：调用方须先去掉 `|…` 再传入 `rawTarget`（本模块与 `splitWikiLinkTarget` 均不按 `|` 拆分）。
 */

const MD_EXT = ".md";

/** 去掉末尾扩展名（.md 大小写不敏感），不改动 basename 大小写；输出路径一律再接小写 `.md` */
function stripMd(s: string): string {
  return s.replace(/\.md$/i, "");
}

/** 将 Vault 内笔记相对路径规范为带小写 `.md` 尾缀（与 pathPart 非空时的解析一致） */
function normalizeMarkdownRelPath(rel: string): string {
  const n = rel.replace(/\\/g, "/").replace(/\/+$/, "");
  if (!n) {
    return "";
  }
  const dir = dirnameRel(n);
  const base = dir ? n.slice(dir.length + 1) : n;
  const file = `${stripMd(base)}${MD_EXT}`;
  return dir ? `${dir}/${file}` : file;
}

/** 对 Vault 相对路径各段编码，保留斜杠 */
function encodeRelPath(rel: string): string {
  if (!rel) {
    return "";
  }
  return rel
    .split("/")
    .map((seg) => encodeURIComponent(seg))
    .join("/");
}

/** 取 Unix 风格目录前缀（不含末尾斜杠），空串表示根 */
export function dirnameRel(relPath: string): string {
  const n = relPath.replace(/\\/g, "/").replace(/\/+$/, "");
  const i = n.lastIndexOf("/");
  return i <= 0 ? "" : n.slice(0, i);
}

/** 按首个 `#` 拆成笔记路径段与标题片段（路径中不写 `#`）；不处理 `|` 别名 */
export function splitWikiLinkTarget(rawTarget: string): {
  pathPart: string;
  headingFragment: string | null;
} {
  const t = rawTarget.trim().replace(/\\/g, "/");
  const idx = t.indexOf("#");
  if (idx < 0) {
    return { pathPart: t, headingFragment: null };
  }
  const pathPart = t.slice(0, idx).trim();
  const after = t.slice(idx + 1).trim();
  return { pathPart, headingFragment: after.length > 0 ? after : null };
}

/**
 * 仅解析笔记相对路径（不含 # 后标题）；pathPart 为空表示当前文件（路径会规范为小写 `.md`）。
 */
export function resolveWikiNoteRelPath(currentRelPath: string, pathPart: string): string {
  const p = pathPart.trim().replace(/\\/g, "/");
  if (!p) {
    return normalizeMarkdownRelPath(currentRelPath);
  }
  if (p.includes("/")) {
    const dir = dirnameRel(p);
    const base = dir ? p.slice(dir.length + 1) : p;
    const raw = dir ? `${dir}/${stripMd(base)}${MD_EXT}` : `${stripMd(base)}${MD_EXT}`;
    return raw.replace(/\\/g, "/").replace(/\/+$/, "");
  }
  const dir = dirnameRel(currentRelPath);
  const file = `${stripMd(p)}${MD_EXT}`;
  const joined = dir ? `${dir}/${file}` : file;
  return joined.replace(/\\/g, "/").replace(/\/+$/, "");
}

/**
 * @param currentRelPath 当前打开的 md 相对路径（Vault 根起）
 * @param rawTarget `[[` 与 `]]` 之间的片段（已 trim 外层亦可）；须不含 `|label` 别名段，仅含可选路径与可选 `#` 标题
 */
export function resolveWikiHref(currentRelPath: string, rawTarget: string): string {
  const t = rawTarget.trim().replace(/\\/g, "/");
  if (!t) {
    return "";
  }
  const { pathPart, headingFragment } = splitWikiLinkTarget(t);
  const noteRel = resolveWikiNoteRelPath(currentRelPath, pathPart);
  if (!noteRel) {
    return "";
  }
  const encoded = encodeRelPath(noteRel);
  if (!headingFragment) {
    return encoded;
  }
  return `${encoded}#${encodeURIComponent(headingFragment)}`;
}
