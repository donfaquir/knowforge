import type { EditorState } from "@milkdown/prose/state";
import { THOUGHT_CALLOUT_LINE_PREFIX } from "./thoughtCalloutConstants";

/**
 * 判断选区是否落在 `> [!thought]` blockquote 内（鼓励自由表达，写作教练不触发）。
 */
export function selectionInsideThoughtCallout(state: EditorState): boolean {
  const { $from } = state.selection;
  for (let d = $from.depth; d > 0; d -= 1) {
    const node = $from.node(d);
    if (node.type.name !== "blockquote") continue;
    const firstChild = node.firstChild;
    const text = (firstChild?.textContent ?? "").trimStart().toLowerCase();
    if (text.startsWith(THOUGHT_CALLOUT_LINE_PREFIX)) {
      return true;
    }
  }
  return false;
}
