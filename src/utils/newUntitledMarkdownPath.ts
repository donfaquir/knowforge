import type { TreeNode } from "../components/FileTree";

/**
 * 新建笔记的兜底模板（固定 H1）；弹窗「新建 Markdown」应使用 `initialMarkdownFromBasename`，
 * 使首行标题与文件名 stem 一致。
 */
export const DEFAULT_NEW_MARKDOWN_TEMPLATE =
  "# Title\n\nStart writing here.\n";

/** 去掉 .md / .markdown 后缀（不区分大小写），保留 stem 原始大小写 */
function stemFromMarkdownBasename(basename: string): string {
  const lower = basename.toLowerCase();
  if (lower.endsWith(".markdown")) {
    return basename.slice(0, -".markdown".length);
  }
  if (lower.endsWith(".md")) {
    return basename.slice(0, -".md".length);
  }
  return basename;
}

/** 用于 ATX 标题行：去换行/制表、压空格 */
function sanitizeHeadingStem(stem: string): string {
  return stem
    .replace(/\r\n|\n|\r|\t/g, " ")
    .replace(/ +/g, " ")
    .trim();
}

/** 与新建/重命名同步标题一致：用于比对或写入首行 H1 文案 */
export function displayHeadingStemFromBasename(basename: string): string {
  return sanitizeHeadingStem(stemFromMarkdownBasename(basename)) || "Untitled";
}

/**
 * 由已通过 `normalizeMarkdownBasename` 的文件基础名生成初始正文：首行 H1 与 stem 对齐。
 */
export function initialMarkdownFromBasename(basename: string): string {
  const heading = displayHeadingStemFromBasename(basename);
  return `# ${heading}\n\nStart writing here.\n`;
}

/** 收集树中所有 Markdown 文件的 rel_path（仅叶子节点） */
export function collectMarkdownFileRelPaths(nodes: TreeNode[]): Set<string> {
  const out = new Set<string>();
  /** 深度优先遍历，目录走 children */
  const walk = (list: TreeNode[]) => {
    for (const n of list) {
      if (n.children != null) {
        walk(n.children);
      } else {
        out.add(n.rel_path);
      }
    }
  };
  walk(nodes);
  return out;
}

/**
 * 在工作区根目录生成不与树冲突的相对路径：Untitled.md、Untitled-2.md、…
 * 与磁盘上已有但不在树中的文件仍可能冲突时，由 write 失败另行处理。
 */
export function nextUntitledRelPath(existing: Set<string>): string {
  return nextUntitledRelPathInDir("", existing);
}

/**
 * 在指定父目录下生成不冲突的 Untitled 相对路径；dirRel 为空表示工作区根。
 */
export function nextUntitledRelPathInDir(dirRel: string, existing: Set<string>): string {
  const d = dirRel.trim().replace(/\/+$/, "");
  const rel = (name: string) => (d ? `${d}/${name}` : name);
  if (!existing.has(rel("Untitled.md"))) {
    return rel("Untitled.md");
  }
  for (let i = 2; i < 10_000; i++) {
    const p = rel(`Untitled-${i}.md`);
    if (!existing.has(p)) {
      return p;
    }
  }
  return rel(`Untitled-${Date.now()}.md`);
}

/** 取文件相对路径的父目录；根下文件返回 "" */
export function parentDirOfRelPath(relPath: string): string {
  const i = relPath.lastIndexOf("/");
  return i < 0 ? "" : relPath.slice(0, i);
}

/** 将目录前缀与文件名拼成 rel_path（dir 为空表示工作区根） */
export function joinRelPath(dirRel: string, fileName: string): string {
  const d = dirRel.trim().replace(/\/+$/, "");
  const f = fileName.trim();
  if (!f) {
    return d;
  }
  return d ? `${d}/${f}` : f;
}

/**
 * 校验用户输入的文件基础名：禁止路径分隔符，自动补全 .md。
 * 非法时返回 null。
 */
export function normalizeMarkdownBasename(input: string): string | null {
  const t = input.trim();
  if (!t || t === "." || t === "..") {
    return null;
  }
  if (t.includes("/") || t.includes("\\")) {
    return null;
  }
  let base = t;
  const lower = base.toLowerCase();
  if (!lower.endsWith(".md") && !lower.endsWith(".markdown")) {
    base = `${base}.md`;
  }
  return base;
}

/**
 * 校验新建文件夹的基础名：禁止路径分隔符，不补全扩展名。
 */
export function normalizeFolderBasename(input: string): string | null {
  const t = input.trim();
  if (!t || t === "." || t === "..") {
    return null;
  }
  if (t.includes("/") || t.includes("\\")) {
    return null;
  }
  return t;
}
