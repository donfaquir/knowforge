import { ACTIVE_PROVIDER_OLLAMA, type VaultConfigForUi } from "../types/vaultAiConfig";

/** 与 Rust `pick_ollama_model` 一致：lastUsedModel 优先，否则 defaultModel */
export function isOllamaModelSelected(cfg: VaultConfigForUi): boolean {
  const m = (cfg.ai?.ollama?.lastUsedModel ?? cfg.ai?.ollama?.defaultModel ?? "").trim();
  return m.length > 0;
}

/** 独立回顾工具栏/顶栏提示：须 Ollama + 已选模型（AI 不可用时隐藏入口） */
export function isIndependentChallengeReviewEntryReady(cfg: VaultConfigForUi): boolean {
  if (cfg.cognitive?.independentReviewEnabled !== true) return false;
  if (cfg.ai?.activeProvider !== ACTIVE_PROVIDER_OLLAMA) return false;
  return isOllamaModelSelected(cfg);
}

/** 通道二生成问句：须 Ollama 且已选模型 */
export function isChallengeInlineLlmReady(cfg: VaultConfigForUi): boolean {
  if (cfg.ai?.activeProvider !== ACTIVE_PROVIDER_OLLAMA) return false;
  return isOllamaModelSelected(cfg);
}
