import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkspaceTextSearchResponse } from "../types/workspaceTextSearch";
import "./WorkspaceSearchModal.css";

type Props = {
  open: boolean;
  onClose: () => void;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  /** 打开笔记并定位到行；携带全文检索词供父级打开篇内查找 */
  onNavigateToHit: (
    relPath: string,
    line: number,
    search: { query: string; caseSensitive: boolean },
  ) => void | Promise<void>;
};

export function WorkspaceSearchModal({
  open,
  onClose,
  workspaceReady,
  tauriRuntime,
  onNavigateToHit,
}: Props) {
  const { t } = useTranslation();
  const [q, setQ] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [resp, setResp] = useState<WorkspaceTextSearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const queryInputRef = useRef<HTMLInputElement>(null);

  const runSearch = useCallback(async () => {
    const query = q.trim();
    if (!open || !workspaceReady || !tauriRuntime || !isTauri() || !query) {
      return;
    }
    setLoading(true);
    setErr(null);
    try {
      const out = await invoke<WorkspaceTextSearchResponse>("search_workspace_text", {
        args: {
          query,
          caseSensitive,
        },
      });
      setResp(out);
    } catch (e) {
      setResp(null);
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [open, workspaceReady, tauriRuntime, q, caseSensitive]);

  useEffect(() => {
    if (!open) {
      return;
    }
    setQ("");
    setCaseSensitive(false);
    setResp(null);
    setErr(null);
    const raf = requestAnimationFrame(() => {
      queryInputRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCloseRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (!open) {
    return null;
  }

  const hits = resp?.hits ?? [];
  const meta = resp?.meta;

  return (
    <div
      className="workspace-search-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="workspace-search" role="dialog" aria-label={t("workspaceSearch.title")}>
        <header className="workspace-search__head">
          <h2 className="workspace-search__title">{t("workspaceSearch.title")}</h2>
          <button type="button" className="workspace-search__close" onClick={onClose}>
            {t("workspaceSearch.close")}
          </button>
        </header>
        <div className="workspace-search__row-input">
          <input
            ref={queryInputRef}
            className="workspace-search__input"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={t("workspaceSearch.placeholder")}
            aria-label={t("workspaceSearch.placeholder")}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void runSearch();
              }
            }}
          />
          <label className="workspace-search__case">
            <input
              type="checkbox"
              checked={caseSensitive}
              onChange={(e) => setCaseSensitive(e.target.checked)}
            />
            {t("workspaceSearch.caseSensitive")}
          </label>
          <button
            type="button"
            className="workspace-search__submit"
            disabled={!q.trim() || loading}
            onClick={() => void runSearch()}
          >
            {t("workspaceSearch.search")}
          </button>
        </div>
        <p className="workspace-search__muted">{t("workspaceSearch.diskOnlyHint")}</p>
        {loading ? <p className="workspace-search__muted">{t("workspaceSearch.loading")}</p> : null}
        {err ? (
          <p className="workspace-search__err" role="alert">
            {err}
          </p>
        ) : null}
        {meta && !loading ? (
          <p className="workspace-search__muted">
            {t("workspaceSearch.metaLine", {
              hits: meta.hitCount,
              scanned: meta.scannedFiles,
              ms: meta.elapsedMs,
            })}
            {meta.truncated ? ` ${t("workspaceSearch.truncated")}` : ""}
            {meta.skippedLargeFiles > 0
              ? ` ${t("workspaceSearch.skippedLarge", { n: meta.skippedLargeFiles })}`
              : ""}
            {meta.omittedPrivatePreviews > 0
              ? ` ${t("workspaceSearch.privatePreviews", { n: meta.omittedPrivatePreviews })}`
              : ""}
          </p>
        ) : null}
        {!loading && resp && hits.length === 0 && q.trim() ? (
          <p className="workspace-search__muted">{t("workspaceSearch.empty")}</p>
        ) : null}
        <ul className="workspace-search__list">
          {hits.map((h, i) => (
            <li key={`${h.relPath}-${h.line}-${h.column}-${i}`} className="workspace-search__li">
              <button
                type="button"
                className="workspace-search__hit"
                onClick={() => {
                  void onNavigateToHit(h.relPath, h.line, {
                    query: q.trim(),
                    caseSensitive,
                  });
                  onClose();
                }}
              >
                <span className="workspace-search__path">{h.relPath}</span>
                <span className="workspace-search__loc">
                  {t("workspaceSearch.lineCol", { line: h.line, col: h.column })}
                </span>
                {h.privateOmitted ? (
                  <span className="workspace-search__private">{t("workspaceSearch.privateRow")}</span>
                ) : h.preview ? (
                  <span className="workspace-search__preview">{h.preview}</span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
