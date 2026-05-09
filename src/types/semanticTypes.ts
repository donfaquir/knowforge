/** 与 Tauri `semantic_index` IPC 载荷（camelCase）对齐 */

export type EmbeddingIndexStatus = {
  modelReady: boolean;
  modelId: string;
  docChunkCount: number;
  thoughtEmbeddingCount: number;
  trackedFileCount: number;
  staleFileCount: number;
};

export type IndexBuildResult = {
  indexedChunks: number;
  indexedThoughts: number;
  elapsedMs: number;
};

export type SemanticSearchHit = {
  relPath?: string;
  thoughtId?: string;
  chunkText: string;
  score: number;
  sourceType: string;
};

export type SemanticSearchArgs = {
  query: string;
  topK?: number;
  searchScope?: "docs" | "thoughts" | "all" | string;
};

export type EmbeddingIndexProgressPayload = {
  phase: string;
  current: number;
  total: number;
  message: string;
};

/** 与 `.knowforge/semantic/rebuild_progress.json` 及 `semantic:rebuild-checkpoint` 对齐 */
export type EmbeddingRebuildProgress = {
  version: number;
  rebuildId: string;
  phase: string;
  startedAt: string;
  updatedAt: string;
  docsTotal: number;
  docsCompleted: number;
  thoughtsTotal: number;
  thoughtsNextIndex: number;
  lastMessage?: string | null;
  lastError?: string | null;
};
