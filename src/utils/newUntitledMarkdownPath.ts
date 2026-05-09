import type { TreeNode } from "../components/FileTree";

/**
 * 新建 Untitled 笔记写入磁盘的初始正文：同时包含标题块与首段，
 * 避免编辑器里只能点标题、必须回车才出现正文区域。
 */
export const DEFAULT_NEW_MARKDOWN_TEMPLATE =
  "# Title\n\nStart writing here.\n";

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
