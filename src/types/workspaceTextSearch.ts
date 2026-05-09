/** 与 `workspace_text_search::SearchWorkspaceTextArgs` / Response 对齐（camelCase） */

export type SearchWorkspaceTextArgs = {
  query: string;
  caseSensitive?: boolean;
  maxFilesToScan?: number;
  maxBytesPerFile?: number;
  /** 每文件最多读入并检索的字节数（磁盘前缀） */
  maxScanBytesPerFile?: number;
  maxHitsPerFile?: number;
  maxTotalHits?: number;
  maxDurationMs?: number;
};

export type WorkspaceTextSearchHit = {
  relPath: string;
  line: number;
  column: number;
  preview?: string | null;
  privateOmitted?: boolean;
};

export type WorkspaceTextSearchMeta = {
  scannedFiles: number;
  hitCount: number;
  truncated: boolean;
  stoppedEarlyDeadline: boolean;
  stoppedEarlyMaxHits: boolean;
  skippedLargeFiles: number;
  elapsedMs: number;
  omittedPrivatePreviews: number;
  /** 仅扫描了前缀、未覆盖全文的文件数 */
  filesScannedAsPrefixOnly?: number;
};

export type WorkspaceTextSearchResponse = {
  hits: WorkspaceTextSearchHit[];
  meta: WorkspaceTextSearchMeta;
};
