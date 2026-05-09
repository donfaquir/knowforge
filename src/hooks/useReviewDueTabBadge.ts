import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { ListReviewQueueResponse } from "../types/cognitiveTypes";
import type { VaultConfigForUi } from "../types/vaultAiConfig";
import { isIndependentChallengeReviewEntryReady } from "../utils/isChallengeReviewLlmReady";
import { VAULT_CONFIG_UPDATED_EVENT } from "../utils/vaultConfigBroadcast";

/**
 * 侧栏「回顾」标签角标：与独立回顾入口门控一致，拉取 list_review_queue.totalDue。
 */
export function useReviewDueTabBadge(params: {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  workspaceRoot: string | null;
}) {
  const { workspaceReady, tauriRuntime, workspaceRoot } = params;
  const [dueCount, setDueCount] = useState(0);

  const refresh = useCallback(async () => {
    if (!tauriRuntime || !workspaceReady || !workspaceRoot) {
      setDueCount(0);
      return;
    }
    try {
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      if (!isIndependentChallengeReviewEntryReady(cfg)) {
        setDueCount(0);
        return;
      }
      const q = await invoke<ListReviewQueueResponse>("list_review_queue");
      setDueCount(q.totalDue);
    } catch {
      setDueCount(0);
    }
  }, [tauriRuntime, workspaceReady, workspaceRoot]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const onVis = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, [refresh]);

  useEffect(() => {
    const onCfg = () => {
      void refresh();
    };
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
    return () => window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
  }, [refresh]);

  return { dueCount, refresh };
}
