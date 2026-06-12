/** 与 `vault_config::VaultConfigForUi` / `AiConfigForUi` 的 JSON（camelCase）对齐 */

import type { CognitiveConfigForUi, CognitiveConfigSavePatch } from "./cognitiveTypes";

export type ActiveProvider = "ollama" | "openai";

/** 与 JSON / IPC 的 activeProvider 对齐，避免业务代码散落字面量 */
export const ACTIVE_PROVIDER_OLLAMA = "ollama" satisfies ActiveProvider;

export type OllamaProfile = {
  baseUrl: string;
  defaultModel: string;
  lastUsedModel?: string;
};

export type OpenAiCompatibleForUi = {
  baseUrl: string;
  apiKeyPresent: boolean;
  defaultModel: string;
  organizationId?: string;
  lastUsedModel?: string;
};

export type AiRequest = {
  timeoutMs: number;
  maxContextTokens?: number;
};

export type AiParameters = {
  temperature: number;
  topP?: number;
};

export type AiPrivacy = {
  allowPrivateContentInLocalLlm: boolean;
};

export type AiConfigForUi = {
  activeProvider: ActiveProvider;
  ollama: OllamaProfile;
  openaiCompatible: OpenAiCompatibleForUi;
  request: AiRequest;
  parameters: AiParameters;
  privacy: AiPrivacy;
  /** Iter 5 #4: 主对话工具调用总开关(含内置 skills 暴露)。旧 vault 缺该字段时后端默认 true。 */
  toolsEnabled: boolean;
  planningEnabled: boolean;
};

export type SemanticConfigForUi = {
  enabled: boolean;
  embeddingModel?: string | null;
  autoIndexOnSave: boolean;
  searchWeight: number;
};

export type SearchProviderType = "searxng" | "tavily" | "aliyun-opensearch";

export type SearchConfigForUi = {
  provider?: SearchProviderType | null;
  searxng?: { baseUrl: string };
  tavily?: { apiKey: string };
  aliyunOpensearch?: { endpoint: string; apiKey: string };
};

export type VaultConfigForUi = {
  /** IPC JSON 字段名为 `$schemaVersion` */
  readonly ["$schemaVersion"]?: number;
  ai: AiConfigForUi;
  cognitive: CognitiveConfigForUi;
  /** 迭代 6.2 起由后端返回；旧配置缺失时前端用默认值 */
  semantic?: SemanticConfigForUi;
  search?: SearchConfigForUi;
};

// --- 与 `save_vault_config_patch` / `VaultConfigPatch`（camelCase JSON）对齐的保存载荷 ---

/** openaiCompatible 段：apiKey 仅在用户修改过密钥时提交 */
export type OpenAiCompatibleSavePayload = {
  baseUrl: string;
  defaultModel: string;
  organizationId: string | null;
  lastUsedModel: string | null;
  apiKey?: string;
};

export type OllamaSavePayload = {
  baseUrl: string;
  defaultModel: string;
  lastUsedModel: string | null;
};

export type AiConfigSavePatch = {
  activeProvider: ActiveProvider;
  ollama: OllamaSavePayload;
  /** 省略则不修改磁盘上的 OpenAI 兼容段（应用仅 Ollama 时可不传） */
  openaiCompatible?: OpenAiCompatibleSavePayload;
  request: {
    timeoutMs: number;
    maxContextTokens: number | null;
  };
  parameters: {
    temperature: number;
    topP: number | null;
  };
  privacy: {
    allowPrivateContentInLocalLlm: boolean;
  };
  toolsEnabled: boolean;
  planningEnabled: boolean;
};

export type SemanticConfigSavePatch = {
  enabled: boolean;
  autoIndexOnSave: boolean;
  searchWeight: number;
};

export type SearchConfigSavePatch = {
  provider?: SearchProviderType | null;
  searxng?: { baseUrl: string } | null;
  tavily?: { apiKey: string } | null;
  aliyunOpensearch?: { endpoint: string; apiKey: string } | null;
};

export type VaultConfigSavePatch = {
  ai?: AiConfigSavePatch;
  cognitive?: CognitiveConfigSavePatch;
  semantic?: SemanticConfigSavePatch;
  search?: SearchConfigSavePatch;
};
