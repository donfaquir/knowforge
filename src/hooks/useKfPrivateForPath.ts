import { useCallback } from "react";
import type { DocState } from "./useOpenDocs";
import { markdownTreatAsKfPrivateForUi } from "../utils/kfPrivateMarkdown";

/**
 * 与 `App` 原 `isPathKfPrivate` 一致：已打开且就绪的文档以缓冲区为准，否则用文件树磁盘快照路径集合。
 */
export function useKfPrivateForPath(
  docByPath: Record<string, DocState>,
  kfPrivatePathsFromTree: Set<string>,
): (relPath: string) => boolean {
  return useCallback(
    (relPath: string) => {
      const doc = docByPath[relPath];
      if (doc && !doc.loading && !doc.loadError) {
        return markdownTreatAsKfPrivateForUi(doc.content);
      }
      return kfPrivatePathsFromTree.has(relPath);
    },
    [docByPath, kfPrivatePathsFromTree],
  );
}
