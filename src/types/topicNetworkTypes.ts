/** 与 Tauri `topic_network` IPC 载荷（camelCase）对齐 */

export type TopicNetworkMeta = {
  topicNodeCap: number;
  docNodeCap: number;
  truncatedTopicCount: number;
  truncatedDocCount: number;
  extractSkippedNoLlm: boolean;
};

export type TopicNode = {
  id: string;
  name: string;
  docCount: number;
  relatedTopicCount: number;
};

export type DocNode = {
  relPath: string;
  topicCount: number;
  thoughtCount: number;
  maxMaturity: string;
};

export type TopicDocEdge = {
  topicId: string;
  docRelPath: string;
};

export type TopicTopicEdge = {
  sourceTopicId: string;
  targetTopicId: string;
  weight: number;
};

export type TopicNetworkForUi = {
  topicNodes: TopicNode[];
  docNodes: DocNode[];
  topicDocEdges: TopicDocEdge[];
  topicTopicEdges: TopicTopicEdge[];
  meta: TopicNetworkMeta;
};

export type TopicCacheStatus = {
  docTopicsRowCount: number;
  dictionaryTopicCount: number;
  distinctDocPaths: number;
};

export type TopicMarkdownExportSummary = {
  topicsWritten: number;
  exportDirRel: string;
};

export type AddManualTopicResult = {
  graph: TopicNetworkForUi;
  associatedDocCount: number;
  canonical: string;
};
