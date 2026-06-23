import { getActiveProfile, type VaultConfigForUi } from "../types/vaultAiConfig";

function isModelSelected(cfg: VaultConfigForUi): boolean {
  const p = cfg.ai ? getActiveProfile(cfg.ai) : undefined;
  return !!p && !!(p.lastUsedModel?.trim() || p.defaultModel?.trim());
}

export function isIndependentChallengeReviewEntryReady(cfg: VaultConfigForUi): boolean {
  if (cfg.cognitive?.independentReviewEnabled !== true) return false;
  return isModelSelected(cfg);
}

export function isChallengeInlineLlmReady(cfg: VaultConfigForUi): boolean {
  return isModelSelected(cfg);
}
