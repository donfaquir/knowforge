import type { EmbeddingRebuildProgress } from "../types/semanticTypes";

/** 与 Rust `RebuildProgress::display_percent` 规则一致 */
export function rebuildCheckpointPercent(p: EmbeddingRebuildProgress): number {
  switch (p.phase) {
    case "completed":
      return 100;
    case "scanning":
      return 0;
    case "documents": {
      const d = Math.max(1, p.docsTotal);
      return Math.round(Math.min(1, p.docsCompleted / d) * 92);
    }
    case "thoughts": {
      const t = Math.max(1, p.thoughtsTotal);
      const th = Math.min(1, p.thoughtsNextIndex / t);
      return Math.round(92 + 8 * th);
    }
    case "failed":
      if (p.thoughtsTotal > 0 && p.docsCompleted >= p.docsTotal) {
        const t = Math.max(1, p.thoughtsTotal);
        const th = Math.min(1, p.thoughtsNextIndex / t);
        return Math.round(Math.min(99, 92 + 8 * th));
      }
      {
        const d = Math.max(1, p.docsTotal);
        const docPart = Math.min(1, p.docsCompleted / d);
        return Math.round(Math.min(99, docPart * 92));
      }
    default:
      return 0;
  }
}

export function canResumeRebuildCheckpoint(p: EmbeddingRebuildProgress | null): boolean {
  if (!p || p.phase === "completed") {
    return false;
  }
  if (p.phase === "thoughts") {
    return p.thoughtsNextIndex < p.thoughtsTotal;
  }
  if (p.phase === "failed") {
    if (p.docsCompleted < p.docsTotal) {
      return true;
    }
    if (p.thoughtsTotal > 0 && p.thoughtsNextIndex < p.thoughtsTotal) {
      return true;
    }
    return false;
  }
  if (p.phase === "documents" || p.phase === "scanning") {
    return p.docsCompleted < p.docsTotal;
  }
  return false;
}
