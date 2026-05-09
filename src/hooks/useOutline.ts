import { useDeferredValue, useMemo } from "react";
import { extractOutline } from "../utils/extractOutline";

/** 按 ES 字符串迭代器统计码点（代理对算 1），避免 Array.from 的 O(n) 临时数组 */
function countCodePoints(s: string): number {
  let n = 0;
  for (const _ of s) {
    n++;
  }
  return n;
}

export function useOutline(activePath: string | null, content: string | undefined) {
  const deferredContent = useDeferredValue(content ?? "");

  const outline = useMemo(() => {
    if (!activePath) {
      return [];
    }
    return extractOutline(deferredContent);
  }, [activePath, deferredContent]);

  const characterCount = useMemo(() => countCodePoints(deferredContent), [deferredContent]);

  return {
    characterCount,
    outline,
  };
}
