import type { ThoughtMaturity } from "./cognitiveTypes";

/** 与 Tauri `understanding_graph::UnderstandingGraphForUi` 对齐；`thoughtCount===0` 时前端以灰色渲染节点 */
export type UnderstandingGraphNode = {
  relPath: string;
  thoughtCount: number;
  maxMaturity: ThoughtMaturity;
  lastUpdated: number;
};

export type UnderstandingGraphEdge = {
  fromRelPath: string;
  toRelPath: string;
};

export type UnderstandingGraphForUi = {
  nodes: UnderstandingGraphNode[];
  edges: UnderstandingGraphEdge[];
  candidateThoughtNoteCount: number;
  indexedMarkdownCount: number;
  hiddenNodeCount: number;
};
