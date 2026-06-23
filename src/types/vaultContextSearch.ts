/** Aligned with `search_workspace_context` / `start_chat_stream` vaultContext (camelCase) */

export type VaultSnippetKind = "excerpt" | "privateOmitted";

export type VaultSnippetRecord = {
  relPath: string;
  kind: VaultSnippetKind;
  excerpt?: string;
};

export type SearchWorkspaceContextMeta = {
  scannedFiles: number;
  stoppedEarly: boolean;
  elapsedMs: number;
};

export type SearchWorkspaceContextResponse = {
  snippets: VaultSnippetRecord[];
  meta: SearchWorkspaceContextMeta;
};

export type SearchWorkspaceLimits = {
  maxFilesToScan?: number;
  maxSnippets?: number;
  maxCharsPerSnippet?: number;
  maxTotalChars?: number;
  readBytesPerFile?: number;
  maxDurationMs?: number;
};
