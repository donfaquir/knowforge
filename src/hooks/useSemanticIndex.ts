import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  EmbeddingIndexProgressPayload,
  EmbeddingIndexStatus,
  EmbeddingRebuildProgress,
  IndexBuildResult,
} from "../types/semanticTypes";

export type UseSemanticIndexOptions = {
  workspaceReady: boolean;
  tauriRuntime: boolean;
};

export function useSemanticIndex(opts: UseSemanticIndexOptions) {
  const [status, setStatus] = useState<EmbeddingIndexStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [progressMessage, setProgressMessage] = useState<string | null>(null);
  const [lastBuildResult, setLastBuildResult] = useState<IndexBuildResult | null>(null);
  const [rebuildProgress, setRebuildProgress] = useState<EmbeddingRebuildProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const disposed = useRef(false);

  useEffect(() => {
    disposed.current = false;
    return () => {
      disposed.current = true;
    };
  }, []);

  const loadRebuildProgress = useCallback(async () => {
    if (!opts.tauriRuntime || !opts.workspaceReady) {
      return;
    }
    try {
      const p = await invoke<EmbeddingRebuildProgress | null>("get_embedding_rebuild_progress");
      if (!disposed.current) {
        setRebuildProgress(p);
      }
    } catch {
      if (!disposed.current) {
        setRebuildProgress(null);
      }
    }
  }, [opts.tauriRuntime, opts.workspaceReady]);

  const refresh = useCallback(async () => {
    if (!opts.tauriRuntime || !opts.workspaceReady) {
      return;
    }
    try {
      const s = await invoke<EmbeddingIndexStatus>("get_embedding_status");
      if (!disposed.current) {
        setStatus(s);
        setError(null);
      }
    } catch (e) {
      if (!disposed.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    }
  }, [opts.tauriRuntime, opts.workspaceReady]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    void loadRebuildProgress();
  }, [loadRebuildProgress]);

  useEffect(() => {
    if (!opts.tauriRuntime || !opts.workspaceReady) {
      return;
    }
    const unsubs: UnlistenFn[] = [];
    let cancelled = false;
    void (async () => {
      const u1 = await listen<EmbeddingIndexProgressPayload>("semantic:index-progress", (ev) => {
        const p = ev.payload;
        const msg = p.message?.trim() || p.phase;
        if (!cancelled) {
          setProgressMessage(msg);
        }
      });
      const u2 = await listen("semantic:index-complete", () => {
        if (!cancelled) {
          setProgressMessage(null);
          setBusy(false);
          void refresh();
          void loadRebuildProgress();
        }
      });
      const u3 = await listen<EmbeddingRebuildProgress>("semantic:rebuild-checkpoint", (ev) => {
        if (!cancelled) {
          setRebuildProgress(ev.payload);
        }
      });
      if (!cancelled) {
        unsubs.push(u1, u2, u3);
      } else {
        u1();
        u2();
        u3();
      }
    })();
    return () => {
      cancelled = true;
      unsubs.forEach((u) => {
        u();
      });
    };
  }, [opts.tauriRuntime, opts.workspaceReady, refresh, loadRebuildProgress]);

  const rebuild = useCallback(
    async (resume = false) => {
      if (!opts.tauriRuntime || !opts.workspaceReady) {
        return;
      }
      setBusy(true);
      setError(null);
      setProgressMessage(null);
      if (!resume) {
        setLastBuildResult(null);
      }
      try {
        // 与 Rust 侧扁平参数 `resume: Option<bool>` 对齐（Tauri 2：invoke 顶层键对应各参数名 camelCase）
        const r = await invoke<IndexBuildResult>("rebuild_embeddings", { resume });
        if (!disposed.current) {
          setLastBuildResult(r);
        }
      } catch (e) {
        if (!disposed.current) {
          setError(e instanceof Error ? e.message : String(e));
          setProgressMessage(null);
        }
      } finally {
        if (!disposed.current) {
          setBusy(false);
          void refresh();
          void loadRebuildProgress();
        }
      }
    },
    [opts.tauriRuntime, opts.workspaceReady, refresh, loadRebuildProgress],
  );

  return {
    status,
    busy,
    error,
    progressMessage,
    lastBuildResult,
    rebuildProgress,
    rebuild,
    refresh,
    loadRebuildProgress,
  };
}
