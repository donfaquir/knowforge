/** 激励反馈相关 IPC / 事件载荷（camelCase，与 Rust serde 对齐） */

import type { KfThoughtHistoryEntry, ThoughtMaturity } from "./cognitiveTypes";

export type ThoughtMaturityChangedPayload = {
  relPath: string;
  thoughtId: string;
  fromMaturity: ThoughtMaturity;
  toMaturity: ThoughtMaturity;
  startLine: number;
};

export type CognitiveReportForUi = {
  scannedFiles: number;
  totalThoughts: number;
  newThisMonth: number;
  updatedThisMonth: number;
  maturity: {
    seedling: number;
    growing: number;
    mature: number;
  };
  prevMonthMaturity: {
    seedling: number;
    growing: number;
    mature: number;
  } | null;
  totalAiReferences: number;
  timelines: Array<{
    relPath: string;
    thoughtId: string;
    excerpt: string;
    history: KfThoughtHistoryEntry[];
  }>;
};
