import { useEffect, useRef } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";

interface AppWindowLike {
  onCloseRequested: (handler: (event: { preventDefault: () => void }) => Promise<void>) => Promise<() => void>;
  destroy: () => Promise<void>;
}

/**
 * Handles window close lifecycle:
 * - Browser: `beforeunload` prompt when dirty tabs exist
 * - Tauri: flush dirty docs before window close, with conflict confirmation dialog
 */
export function useWindowLifecycle(opts: {
  tauriRuntime: boolean;
  appWindow: AppWindowLike | null;
  flushDirtyBeforeExit: () => Promise<{ conflictDirtyPaths: string[]; saveFailed: boolean }>;
  hasAnyDirtyTab: () => boolean;
}): void {
  const { tauriRuntime, appWindow, hasAnyDirtyTab } = opts;
  const { t } = useTranslation();

  // Keep a stable ref to avoid re-subscribing the Tauri close handler
  const flushRef = useRef(opts.flushDirtyBeforeExit);
  flushRef.current = opts.flushDirtyBeforeExit;

  // Browser: beforeunload
  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (hasAnyDirtyTab()) {
        e.preventDefault();
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [hasAnyDirtyTab]);

  // Tauri: onCloseRequested — flush, confirm conflicts, then destroy
  useEffect(() => {
    if (!tauriRuntime || !appWindow) {
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void appWindow
      .onCloseRequested(async (event) => {
        event.preventDefault();
        try {
          const { conflictDirtyPaths, saveFailed } = await flushRef.current();
          if (saveFailed) {
            return;
          }
          if (conflictDirtyPaths.length > 0) {
            const ok = await ask(
              t("dialogs.closeWindowDiskConflict", { count: conflictDirtyPaths.length }),
              {
                title: t("dialogs.close"),
                kind: "warning",
              },
            );
            if (!ok) {
              return;
            }
          }
          await appWindow.destroy();
        } catch (e) {
          // Already called preventDefault — must attempt destroy or window is stuck
          console.error(e);
          try {
            await appWindow.destroy();
          } catch (e2) {
            console.error(e2);
          }
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [appWindow, tauriRuntime, t]);
}
