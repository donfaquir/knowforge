import { invoke, isTauri } from "@tauri-apps/api/core";

/** 追加一行到工作区 `.knowforge/analytics.jsonl`（桌面端）；失败时静默 */
export async function trackKnowforgeEvent(
  event: string,
  payload?: Record<string, unknown>,
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("append_knowforge_analytics", {
      args: { event, payload: payload ?? null },
    });
  } catch {
    /* 埋点失败不打扰主流程 */
  }
}
