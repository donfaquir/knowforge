import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ParseNoteThoughtsResponse } from "../types/cognitiveTypes";
import { endPerfTrace, logPerfMark, startPerfTrace } from "../utils/perfTrace";
import "./EditorThoughtsPanel.css";

const MAX_THOUGHTS_PARSE_CACHE_ENTRIES = 24;

type Props = {
  activeRelPath: string | null;
  /** 右栏大纲 tab 可见且打开时才会发起 parse IPC；避免切到 AI/图谱 时仍随文档切换打后端 */
  outlinePanelActive: boolean;
  /** 当前笔记已落盘的 Markdown；保存或从磁盘重载后变化，用于触发重新 parse（与侧车读盘一致） */
  savedMarkdownSnapshot?: string | undefined;
  /** `reloadFromDisk` 等会递增，与 saved 快照组合避免仅靠字符串引用漏刷新 */
  editorContentInjectEpoch?: number;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  onOpenNote: (relPath: string) => void;
};

export function EditorThoughtsPanel({
  activeRelPath,
  outlinePanelActive,
  savedMarkdownSnapshot,
  editorContentInjectEpoch = 0,
  workspaceReady,
  tauriRuntime,
  onOpenNote,
}: Props) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [data, setData] = useState<ParseNoteThoughtsResponse | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const parseCacheRef = useRef<
    Map<
      string,
      {
        savedMarkdownSnapshot: string | undefined;
        editorContentInjectEpoch: number;
        data: ParseNoteThoughtsResponse;
      }
    >
  >(new Map());
  const requestSeqRef = useRef(0);

  const load = useCallback(async (opts?: { force?: boolean }) => {
    if (!activeRelPath || !workspaceReady || !tauriRuntime || !isTauri()) {
      setData(null);
      setErr(null);
      return;
    }
    if (!outlinePanelActive) {
      setLoading(false);
      return;
    }
    const cached = parseCacheRef.current.get(activeRelPath);
    if (
      !opts?.force &&
      cached &&
      cached.savedMarkdownSnapshot === savedMarkdownSnapshot &&
      cached.editorContentInjectEpoch === editorContentInjectEpoch
    ) {
      parseCacheRef.current.delete(activeRelPath);
      parseCacheRef.current.set(activeRelPath, cached);
      logPerfMark("markdown.thoughts.parse_note_cache_hit", {
        relPath: activeRelPath,
        count: cached.data.meta.length,
      });
      setData(cached.data);
      setErr(null);
      setLoading(false);
      return;
    }
    const seq = ++requestSeqRef.current;
    setLoading(true);
    setErr(null);
    const parseTrace = startPerfTrace("markdown.thoughts.parse_note", { relPath: activeRelPath });
    try {
      const resp = await invoke<ParseNoteThoughtsResponse>("parse_note_thoughts", {
        relPath: activeRelPath,
      });
      parseCacheRef.current.delete(activeRelPath);
      parseCacheRef.current.set(activeRelPath, {
        savedMarkdownSnapshot,
        editorContentInjectEpoch,
        data: resp,
      });
      while (parseCacheRef.current.size > MAX_THOUGHTS_PARSE_CACHE_ENTRIES) {
        const oldestKey = parseCacheRef.current.keys().next().value;
        if (oldestKey == null) break;
        parseCacheRef.current.delete(oldestKey);
      }
      endPerfTrace(parseTrace, {
        status: "ok",
        count: resp.meta.length,
      });
      if (requestSeqRef.current !== seq) {
        return;
      }
      setData(resp);
    } catch (e) {
      endPerfTrace(parseTrace, { status: "error" });
      if (requestSeqRef.current !== seq) {
        return;
      }
      setData(null);
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) {
        setLoading(false);
      }
    }
  }, [
    activeRelPath,
    editorContentInjectEpoch,
    savedMarkdownSnapshot,
    workspaceReady,
    tauriRuntime,
    outlinePanelActive,
  ]);

  const loadRef = useRef(load);
  loadRef.current = load;

  const outlinePanelActiveRef = useRef(outlinePanelActive);
  outlinePanelActiveRef.current = outlinePanelActive;

  /** 切离大纲 tab 时丢弃进行中的 parse 结果，避免晚到响应污染 UI */
  useEffect(() => {
    if (!outlinePanelActive) {
      requestSeqRef.current += 1;
      setLoading(false);
    }
  }, [outlinePanelActive]);

  /** saved / injectEpoch 变化时须重新 parse；未变化的路径切回时直接使用缓存 */
  useEffect(() => {
    void load();
  }, [load]);

  /** 其它进程或本应用 IPC 写盘后 emit，缓冲区 saved 未变时也要刷新侧栏读盘结果 */
  useEffect(() => {
    if (!isTauri() || !activeRelPath) {
      return;
    }
    let cancelled = false;
    /** 用 ref 承接异步返回的 unlisten，cleanup 与 then 读写同一槽，避免 then 未跑完时 let 未赋值导致漏拆 */
    const unlistenRef = { current: null as null | (() => void) };

    void listen<{ relPath: string }>("markdown-disk-changed", (event) => {
      if (cancelled || event.payload.relPath !== activeRelPath) {
        return;
      }
      if (!outlinePanelActiveRef.current) {
        return;
      }
      void loadRef.current({ force: true });
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlistenRef.current = fn;
      })
      .catch(() => {
        /* listen 失败时无 fn，仅依赖卸载后换路径重挂 */
      });

    return () => {
      cancelled = true;
      const u = unlistenRef.current;
      unlistenRef.current = null;
      u?.();
    };
  }, [activeRelPath]);

  if (!activeRelPath || !workspaceReady || !tauriRuntime) {
    return null;
  }

  const count = data?.meta?.length ?? 0;
  if (!loading && count === 0 && !err) {
    return null;
  }

  return (
    <section className="editor-thoughts-panel" aria-label={t("thoughtPanel.title")}>
      <button
        type="button"
        className="editor-thoughts-panel__toggle"
        aria-expanded={expanded}
        onClick={() => setExpanded((e) => !e)}
      >
        <span className="editor-thoughts-panel__toggle-label">
          {t("thoughtPanel.title")} {count > 0 ? `(${count})` : ""}
        </span>
        <span className="editor-thoughts-panel__toggle-hint">
          {expanded ? t("thoughtPanel.collapse") : t("thoughtPanel.expand")}
        </span>
      </button>
      {expanded ? (
        <div className="editor-thoughts-panel__body">
          {loading ? <p className="editor-thoughts-panel__muted">{t("thoughtPanel.loading")}</p> : null}
          {err ? (
            <p className="editor-thoughts-panel__err" role="alert">
              {err}
            </p>
          ) : null}
          {data?.yamlWarnings && data.yamlWarnings.length > 0 ? (
            <ul className="editor-thoughts-panel__warnings">
              {data.yamlWarnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          ) : null}
          {!loading && count === 0 ? (
            <p className="editor-thoughts-panel__muted">{t("thoughtPanel.empty")}</p>
          ) : null}
          <ul className="editor-thoughts-panel__list">
            {(data?.meta ?? []).map((m, i) => {
              const excerpt = data?.blocks[i]?.excerpt?.trim() ?? "";
              return (
                <li key={m.id || String(i)} className="editor-thoughts-panel__item">
                  <div className="editor-thoughts-panel__item-head">
                    <code className="editor-thoughts-panel__id">{m.id || "—"}</code>
                    <span className="editor-thoughts-panel__mat">{m.maturity}</span>
                    {m.temporary ? (
                      <span className="editor-thoughts-panel__badge">{t("thoughtPanel.temporary")}</span>
                    ) : null}
                  </div>
                  {excerpt ? (
                    <p className="editor-thoughts-panel__excerpt">{excerpt}</p>
                  ) : (
                    <p className="editor-thoughts-panel__muted">{t("thoughtPanel.noExcerpt")}</p>
                  )}
                </li>
              );
            })}
          </ul>
          {activeRelPath ? (
            <button
              type="button"
              className="editor-thoughts-panel__open"
              onClick={() => onOpenNote(activeRelPath)}
            >
              {t("thoughtPanel.openNote")}
            </button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
