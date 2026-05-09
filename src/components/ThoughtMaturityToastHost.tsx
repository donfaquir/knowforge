/**
 * 订阅 Tauri `thought-maturity-changed`，展示轻量 Toast 并在点击时打开源笔记。
 * 业务说明见 docs/iteration4/04-motivation-feedback.md
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { isTauri } from "@tauri-apps/api/core";
import { useAiNoteContext } from "../contexts/AiNoteContext";
import type { ThoughtMaturityChangedPayload } from "../types/motivationFeedback";
import "./ThoughtMaturityToastHost.css";

type ToastItem = { id: string; payload: ThoughtMaturityChangedPayload; fading: boolean };

const MATURITY_EMOJI: Record<string, string> = {
  seedling: "\u{1F331}",
  growing: "\u{1F33F}",
  mature: "\u{1F333}",
};

function noteTitleFromRelPath(relPath: string): string {
  const parts = relPath.split("/");
  return parts[parts.length - 1] || relPath;
}

export function ThoughtMaturityToastHost() {
  const { t } = useTranslation();
  const { openMarkdownTab } = useAiNoteContext();
  const [items, setItems] = useState<ToastItem[]>([]);
  const timersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const removeTimers = useCallback((id: string) => {
    const t1 = timersRef.current.get(`${id}-fade`);
    const t2 = timersRef.current.get(`${id}-rm`);
    if (t1) clearTimeout(t1);
    if (t2) clearTimeout(t2);
    timersRef.current.delete(`${id}-fade`);
    timersRef.current.delete(`${id}-rm`);
  }, []);

  const dismiss = useCallback(
    (id: string) => {
      removeTimers(id);
      setItems((prev) => prev.filter((x) => x.id !== id));
    },
    [removeTimers],
  );

  const scheduleDismiss = useCallback(
    (id: string) => {
      removeTimers(id);
      const fade = setTimeout(() => {
        setItems((prev) => prev.map((x) => (x.id === id ? { ...x, fading: true } : x)));
      }, 2700);
      const rm = setTimeout(() => dismiss(id), 3100);
      timersRef.current.set(`${id}-fade`, fade);
      timersRef.current.set(`${id}-rm`, rm);
    },
    [dismiss, removeTimers],
  );

  // 订阅只应挂载一次；若依赖 scheduleDismiss，将来 dismiss 不稳定会导致重复 listen，且 cleanup 会清空仍在展示的 toast 定时器
  const scheduleDismissRef = useRef(scheduleDismiss);
  scheduleDismissRef.current = scheduleDismiss;

  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;
    void listen<ThoughtMaturityChangedPayload>("thought-maturity-changed", (ev) => {
      if (cancelled) return;
      const p = ev.payload;
      const id = `${p.thoughtId}-${p.relPath}-${Date.now()}`;
      setItems((prev) => [...prev, { id, payload: p, fading: false }]);
      scheduleDismissRef.current(id);
    }).then((fn) => {
      if (!cancelled) unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
      timersRef.current.forEach((x) => clearTimeout(x));
      timersRef.current.clear();
    };
  }, []);

  const onClickToast = useCallback(
    (item: ToastItem) => {
      openMarkdownTab?.(item.payload.relPath);
      window.dispatchEvent(
        new CustomEvent("kf-goto-source-line", {
          detail: { relPath: item.payload.relPath, line: item.payload.startLine },
        }),
      );
      dismiss(item.id);
    },
    [dismiss, openMarkdownTab],
  );

  if (items.length === 0) {
    return null;
  }

  return (
    <div className="thought-maturity-host" aria-live="polite">
      {items.map((item) => {
        const { payload } = item;
        const fromE = MATURITY_EMOJI[payload.fromMaturity] ?? "\u{1F331}";
        const toE = MATURITY_EMOJI[payload.toMaturity] ?? "\u{1F333}";
        const title = noteTitleFromRelPath(payload.relPath);
        return (
          <button
            key={item.id}
            type="button"
            className={`thought-maturity-host__toast${item.fading ? " thought-maturity-host__toast--out" : ""}`}
            onClick={() => onClickToast(item)}
          >
            {t("maturityToast.body", { from: fromE, to: toE, note: title })}
          </button>
        );
      })}
    </div>
  );
}
