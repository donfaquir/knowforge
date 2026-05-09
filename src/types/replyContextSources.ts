import type { VaultSnippetKind } from "./vaultContextSearch";

/** 与 Rust `llm::ReplyCurrentNoteMode` 对齐（serde lowercase） */
export type ReplyCurrentNoteMode = "full" | "redacted";

export type ReplyCurrentNoteSource = {
  relPath: string;
  mode: ReplyCurrentNoteMode;
};

export type ReplyVaultKeywordEntry = {
  relPath: string;
  kind: VaultSnippetKind;
};

export type ReplyVaultKeywordSource = {
  entries: ReplyVaultKeywordEntry[];
  truncated: boolean;
};

export type ReplySemanticSource = {
  injected: boolean;
  documentPaths: string[];
  thoughtIds: string[];
};

export type ReplyThoughtFocusSource = {
  thoughtId: string;
};

/** 与 Rust `llm::ReplyContextSources` 对齐（camelCase） */
export type ReplyContextSources = {
  currentNote?: ReplyCurrentNoteSource | null;
  vaultKeyword?: ReplyVaultKeywordSource | null;
  semantic?: ReplySemanticSource;
  thoughtFocus?: ReplyThoughtFocusSource | null;
};

export function hasReplyContextSourcesToShow(s: ReplyContextSources | undefined): boolean {
  if (!s) return false;
  if (s.currentNote?.relPath) return true;
  const kw = s.vaultKeyword?.entries;
  if (Array.isArray(kw) && kw.length > 0) return true;
  if (s.semantic?.injected) return true;
  if (s.thoughtFocus?.thoughtId) return true;
  return false;
}
