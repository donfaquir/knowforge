/**
 * 是否为代码块容器类节点的 `type.name`（Crepe：`code_block` / CodeMirror 块：`codeMirror`）。
 * 刻意不用子串匹配，避免自定义节点名含 `code_block` 片段被误判。
 */
export function isCodeBlockContainerNodeName(name: string): boolean {
  return name === "code_block" || name === "codeMirror";
}
