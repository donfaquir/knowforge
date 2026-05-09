import type { TreeNode } from "../components/FileTree";

export type WikiSuggestFileRow = {
  relPath: string;
  /** 插入 [[...]] 内时使用的字符串（与 resolveWikiNoteRelPath 一致：去 .md，保留路径） */
  insertLabel: string;
  /** 列表主文案 */
  displayName: string;
};

function stripMdExt(rel: string): string {
  const n = rel.replace(/\\/g, "/");
  return n.toLowerCase().endsWith(".md") ? n.slice(0, -3) : n;
}

/** 将文件树叶子 Markdown 展平为 wikilink 选择列表（Vault 相对路径） */
export function flattenMarkdownTreeForWikiSuggest(nodes: TreeNode[]): WikiSuggestFileRow[] {
  const out: WikiSuggestFileRow[] = [];
  const walk = (list: TreeNode[]) => {
    for (const n of list) {
      if (n.children != null) {
        walk(n.children);
      } else {
        const relPath = n.rel_path.replace(/\\/g, "/");
        out.push({
          relPath,
          insertLabel: stripMdExt(relPath),
          displayName: n.name,
        });
      }
    }
  };
  walk(nodes);
  out.sort((a, b) => a.relPath.localeCompare(b.relPath, undefined, { sensitivity: "base" }));
  return out;
}
