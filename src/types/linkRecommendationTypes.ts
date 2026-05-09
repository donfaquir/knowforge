/** 与 Tauri `LinkRecommendation`（camelCase）对齐 */

export type LinkRecommendation = {
  targetRelPath: string;
  score: number;
  sharedTopics: string[];
  existingLink: boolean;
  reason?: string | null;
};
