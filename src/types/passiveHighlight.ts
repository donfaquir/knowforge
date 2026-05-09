/** 与 Rust 检测 JSON / IPC 对齐 */

export type PassiveHighlightKind = "integrate" | "correct" | "cross_domain";

export type PassiveHighlightPhase = "pending" | "rejected" | "marked" | "dismissed";

type PassiveHighlightBase = {
  phase: PassiveHighlightPhase;
};

export type PassiveHighlightPending = PassiveHighlightBase & { phase: "pending" };

export type PassiveHighlightRejected = PassiveHighlightBase & { phase: "rejected" };

export type PassiveHighlightDismissed = PassiveHighlightBase & { phase: "dismissed" };

export type PassiveHighlightMarked = PassiveHighlightBase & {
  phase: "marked";
  kind: PassiveHighlightKind;
  confidence: number;
  summary: string;
  useRawFallback: boolean;
  /** `true` 已写入想法；缺省或 `false` 表示仍可保存（UI 与 strip 逻辑均按 `=== true` 判断） */
  saved?: boolean;
  overlayOpen?: boolean;
};

export type PassiveHighlightState =
  | PassiveHighlightPending
  | PassiveHighlightRejected
  | PassiveHighlightMarked
  | PassiveHighlightDismissed;

/**
 * IPC `detect_passive_highlight` 响应。
 * Rust 在 `detected === true` 时仅序列化 `normalize_kind` 后的三种 kind；否则 kind 省略。
 */
export type DetectPassiveHighlightResponse = {
  detected: boolean;
  kind?: PassiveHighlightKind;
  confidence?: number;
  summary?: string;
  useRawFallback?: boolean;
};
