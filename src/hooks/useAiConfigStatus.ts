import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "@tauri-apps/api/core";
import { getActiveProfile, type VaultConfigForUi } from "../types/vaultAiConfig";
import { dispatchOpenAiSettings, VAULT_CONFIG_UPDATED_EVENT } from "../utils/vaultConfigBroadcast";

interface AiConfigStatus {
  isConfigured: boolean;
  isLoading: boolean;
  openSettings: () => void;
}

export function useAiConfigStatus(workspaceReady: boolean): AiConfigStatus {
  const [isConfigured, setIsConfigured] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  const check = useCallback(async () => {
    if (!isTauri() || !workspaceReady) {
      setIsConfigured(false);
      setIsLoading(false);
      return;
    }
    try {
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      const profile = cfg.ai ? getActiveProfile(cfg.ai) : undefined;
      const hasModel = !!(profile?.lastUsedModel?.trim() || profile?.defaultModel?.trim());
      const hasKey = profile?.isRemote === false || profile?.apiKeyPresent;
      setIsConfigured(!!profile && !!hasKey && hasModel);
    } catch {
      setIsConfigured(false);
    } finally {
      setIsLoading(false);
    }
  }, [workspaceReady]);

  useEffect(() => {
    check();
    const handler = () => void check();
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, handler);
    return () => window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, handler);
  }, [check]);

  return { isConfigured, isLoading, openSettings: dispatchOpenAiSettings };
}
