/** Aligned with `vault_config::VaultConfigForUi` / `AiConfigForUi` JSON (camelCase) */

import type { CognitiveConfigForUi, CognitiveConfigSavePatch } from "./cognitiveTypes";

// --- Provider profile (read from backend) ---

export type ProviderProfileForUi = {
  id: string;
  label: string;
  baseUrl: string;
  apiKeyPresent: boolean;
  defaultModel: string;
  organizationId?: string;
  lastUsedModel?: string;
  isRemote: boolean;
};

// --- Shared parameter types ---

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

// --- AI config (read) ---

export type AiConfigForUi = {
  activeProviderId: string;
  providers: ProviderProfileForUi[];
  request: AiRequest;
  parameters: AiParameters;
  privacy: AiPrivacy;
  toolsEnabled: boolean;
  planningEnabled: boolean;
  planningApprovalEnabled: boolean;
  memoryEnabled: boolean;
  memoryReflectionMode: string;
};

// --- Semantic / Search ---

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

// --- Top-level config (read) ---

export type VaultConfigForUi = {
  readonly ["$schemaVersion"]?: number;
  ai: AiConfigForUi;
  cognitive: CognitiveConfigForUi;
  semantic?: SemanticConfigForUi;
  search?: SearchConfigForUi;
};

// --- Save payloads (write) ---

export type ProviderProfileSavePatch = {
  id: string;
  label?: string;
  baseUrl?: string;
  apiKey?: string;
  defaultModel?: string;
  organizationId?: string | null;
  lastUsedModel?: string | null;
  isRemote?: boolean;
};

export type AiConfigSavePatch = {
  activeProviderId?: string;
  providers?: ProviderProfileSavePatch[];
  request?: {
    timeoutMs: number;
    maxContextTokens: number | null;
  };
  parameters?: {
    temperature: number;
    topP: number | null;
  };
  privacy?: {
    allowPrivateContentInLocalLlm: boolean;
  };
  toolsEnabled?: boolean;
  planningEnabled?: boolean;
  planningApprovalEnabled?: boolean;
  memoryEnabled?: boolean;
  memoryReflectionMode?: string;
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

// --- Helpers ---

export function getActiveProfile(ai: AiConfigForUi): ProviderProfileForUi | undefined {
  return ai.providers.find((p) => p.id === ai.activeProviderId);
}
