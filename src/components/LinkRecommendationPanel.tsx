import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import { useTranslation } from "react-i18next";
import type { EmbeddingIndexStatus } from "../types/semanticTypes";
import type { LinkRecommendation } from "../types/linkRecommendationTypes";
import type { CrepeMarkdownEditorApi } from "./CrepeMarkdownEditor";
import "./LinkRecommendationPanel.css";

const IGNORED_STORAGE_PREFIX = "knowforge.linkRec.ignored:v1:";

function IconBase({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <svg
      className={className ?? "link-rec-panel__icon-svg"}
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      {children}
    </svg>
  );
}

/** 获取推荐：火花 / 灵感推荐（避免链环「像普通超链接」的误导） */
function IconFetchSuggest() {
  return (
    <IconBase>
      <path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z" />
      <path d="M5 3v4" />
      <path d="M19 17v4" />
      <path d="M3 5h4" />
      <path d="M17 19h4" />
    </IconBase>
  );
}

/** 加载中 */
function IconLoader() {
  return (
    <IconBase className="link-rec-panel__icon-svg link-rec-panel__icon-svg--spin">
      <path d="M21 12a9 9 0 1 1-6.219-8.56" />
    </IconBase>
  );
}

/** 插入到光标：加号（在光标处插入链接） */
function IconInsertAtCursor() {
  return (
    <IconBase>
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </IconBase>
  );
}

/** 追加到相关笔记：列表 + 加号 */
function IconAppendRelated() {
  return (
    <IconBase>
      <path d="M11 12H3" />
      <path d="M16 6H3" />
      <path d="M16 12H12" />
      <path d="M16 18H3" />
      <path d="M18 9v6" />
      <path d="M21 12h-6" />
    </IconBase>
  );
}

/** 忽略 */
function IconIgnore() {
  return (
    <IconBase>
      <circle cx="12" cy="12" r="10" />
      <path d="m15 9-6 6" />
      <path d="m9 9 6 6" />
    </IconBase>
  );
}

export type LinkRecommendationPanelProps = {
  workspaceRoot: string | null;
  activeRelPath: string | null;
  /** 当前为右栏「链接推荐」独立 tab 且侧栏打开时为 true */
  panelActive: boolean;
  savedMarkdownSnapshot?: string | undefined;
  editorContentInjectEpoch?: number;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  crepeApiRef: MutableRefObject<CrepeMarkdownEditorApi | null>;
};

/** 与 flattenMarkdownTreeForWikiSuggest.insertLabel 一致：去 `.md`，保留路径 */
function wikilinkInnerFromRelPath(rel: string): string {
  const n = rel.replace(/\\/g, "/");
  return n.toLowerCase().endsWith(".md") ? n.slice(0, -3) : n;
}

function displayTitleFromRelPath(rel: string): string {
  const n = rel.replace(/\\/g, "/");
  const i = n.lastIndexOf("/");
  const base = i >= 0 ? n.slice(i + 1) : n;
  return base.toLowerCase().endsWith(".md") ? base.slice(0, -3) : base;
}

function ignoredStorageKey(root: string): string {
  return IGNORED_STORAGE_PREFIX + encodeURIComponent(root);
}

function loadIgnoredSet(root: string | null): Set<string> {
  if (!root) {
    return new Set();
  }
  try {
    const raw = localStorage.getItem(ignoredStorageKey(root));
    if (!raw) {
      return new Set();
    }
    const a = JSON.parse(raw) as unknown;
    if (!Array.isArray(a)) {
      return new Set();
    }
    return new Set(a.filter((x): x is string => typeof x === "string").map((x) => x.replace(/\\/g, "/")));
  } catch {
    return new Set();
  }
}

function persistIgnoredSet(root: string, set: Set<string>): void {
  try {
    localStorage.setItem(ignoredStorageKey(root), JSON.stringify([...set]));
  } catch {
    /* 配额等：静默失败 */
  }
}

export function LinkRecommendationPanel({
  workspaceRoot,
  activeRelPath,
  panelActive,
  savedMarkdownSnapshot,
  editorContentInjectEpoch = 0,
  workspaceReady,
  tauriRuntime,
  crepeApiRef,
}: LinkRecommendationPanelProps) {
  const { t } = useTranslation();
  const [indexStatus, setIndexStatus] = useState<EmbeddingIndexStatus | null>(null);
  const [statusErr, setStatusErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [listErr, setListErr] = useState<string | null>(null);
  const [recs, setRecs] = useState<LinkRecommendation[]>([]);
  const [includeReasons, setIncludeReasons] = useState(false);
  const prevSavedRef = useRef<{ snap: string | undefined; epoch: number } | null>(null);
  const suggestSeqRef = useRef(0);

  useEffect(() => {
    prevSavedRef.current = null;
    setRecs([]);
    setListErr(null);
  }, [activeRelPath]);

  const indexHasDocVectors = (indexStatus?.docChunkCount ?? 0) > 0;
  const modelReady = indexStatus?.modelReady ?? false;
  const indexReadyForSuggest = modelReady && indexHasDocVectors;

  const refreshIndexStatus = useCallback(async () => {
    if (!workspaceReady || !tauriRuntime || !isTauri() || !panelActive) {
      return;
    }
    setStatusErr(null);
    try {
      const s = await invoke<EmbeddingIndexStatus>("get_embedding_status");
      setIndexStatus(s);
    } catch (e) {
      setIndexStatus(null);
      setStatusErr(e instanceof Error ? e.message : String(e));
    }
  }, [workspaceReady, tauriRuntime, panelActive]);

  useEffect(() => {
    void refreshIndexStatus();
  }, [refreshIndexStatus]);

  const runSuggest = useCallback(async () => {
    if (!activeRelPath || !workspaceReady || !tauriRuntime || !isTauri()) {
      return;
    }
    if (!indexReadyForSuggest) {
      setListErr(t("linkRecommendation.indexRequired"));
      return;
    }
    const seq = ++suggestSeqRef.current;
    setLoading(true);
    setListErr(null);
    try {
      const fromEditor = crepeApiRef.current?.getCurrentMarkdown?.() ?? null;
      const editorMarkdownOverride =
        typeof fromEditor === "string" && fromEditor.trim().length > 0 ? fromEditor : null;
      const rows = await invoke<LinkRecommendation[]>("suggest_related_notes", {
        relPath: activeRelPath,
        maxResults: 5,
        includeReasons,
        editorMarkdownOverride,
      });
      if (seq !== suggestSeqRef.current) {
        return;
      }
      const ig = workspaceRoot ? loadIgnoredSet(workspaceRoot) : new Set<string>();
      const filtered = rows.filter((r) => !ig.has(r.targetRelPath.replace(/\\/g, "/")));
      setRecs(filtered);
    } catch (e) {
      if (seq !== suggestSeqRef.current) {
        return;
      }
      const msg = e instanceof Error ? e.message : String(e);
      setRecs([]);
      setListErr(msg);
    } finally {
      if (seq === suggestSeqRef.current) {
        setLoading(false);
      }
    }
  }, [
    activeRelPath,
    workspaceReady,
    tauriRuntime,
    indexReadyForSuggest,
    includeReasons,
    workspaceRoot,
    crepeApiRef,
  ]);

  useEffect(() => {
    if (!panelActive || !indexReadyForSuggest) {
      return;
    }
    if (!activeRelPath || !workspaceReady || !tauriRuntime || !isTauri()) {
      return;
    }
    const prev = prevSavedRef.current;
    const snap = savedMarkdownSnapshot;
    const ep = editorContentInjectEpoch;
    if (prev !== null && prev.snap === snap && prev.epoch === ep) {
      return;
    }
    prevSavedRef.current = { snap, epoch: ep };
    if (prev === null && snap === undefined) {
      return;
    }
    const tid = window.setTimeout(() => {
      void runSuggest();
    }, 450);
    return () => window.clearTimeout(tid);
  }, [
    savedMarkdownSnapshot,
    editorContentInjectEpoch,
    panelActive,
    indexReadyForSuggest,
    activeRelPath,
    workspaceReady,
    tauriRuntime,
    runSuggest,
  ]);

  const onIgnore = (targetRelPath: string) => {
    if (!workspaceRoot) {
      return;
    }
    const next = loadIgnoredSet(workspaceRoot);
    next.add(targetRelPath.replace(/\\/g, "/"));
    persistIgnoredSet(workspaceRoot, next);
    setRecs((prev) => prev.filter((r) => r.targetRelPath.replace(/\\/g, "/") !== targetRelPath.replace(/\\/g, "/")));
  };

  const onInsertAtCursor = (targetRelPath: string) => {
    const api = crepeApiRef.current;
    if (!api || !api.insertTextAtCursor) {
      return;
    }
    const inner = wikilinkInnerFromRelPath(targetRelPath);
    const ok = api.insertTextAtCursor(`[[${inner}]]`);
    if (!ok) {
      setListErr(t("linkRecommendation.insertFailed"));
    } else {
      setListErr(null);
      void runSuggest();
    }
  };

  const onAppendToRelatedSection = (targetRelPath: string) => {
    const api = crepeApiRef.current;
    if (!api || !api.appendRelatedNotesWikiLinkLine) {
      setListErr(t("linkRecommendation.insertFailed"));
      return;
    }
    const inner = wikilinkInnerFromRelPath(targetRelPath);
    const ok = api.appendRelatedNotesWikiLinkLine(inner);
    if (!ok) {
      setListErr(t("linkRecommendation.appendDuplicateOrFailed"));
    } else {
      setListErr(null);
      void runSuggest();
    }
  };

  const hint = !workspaceReady || !tauriRuntime
    ? t("linkRecommendation.workspaceRequired")
    : !modelReady
      ? t("linkRecommendation.modelNotReady")
      : !indexHasDocVectors
        ? t("linkRecommendation.noChunks")
        : statusErr
          ? statusErr
          : null;

  return (
    <section className="link-rec-panel link-rec-panel--page" aria-label={t("linkRecommendation.sectionAria")}>
      <div className="link-rec-panel__head">
        <span className="link-rec-panel__title">{t("linkRecommendation.title")}</span>
        <div className="link-rec-panel__actions">
          <button
            type="button"
            className="link-rec-panel__btn link-rec-panel__btn--icon"
            disabled={!activeRelPath || !workspaceReady || !tauriRuntime || loading || !indexReadyForSuggest}
            title={loading ? t("linkRecommendation.loading") : t("linkRecommendation.fetchTooltip")}
            aria-label={loading ? t("linkRecommendation.loading") : t("linkRecommendation.fetchTooltip")}
            onClick={() => void runSuggest()}
          >
            {loading ? <IconLoader /> : <IconFetchSuggest />}
          </button>
        </div>
      </div>
      <div className="link-rec-panel__options">
        <label>
          <input
            type="checkbox"
            checked={includeReasons}
            onChange={(e) => setIncludeReasons(e.target.checked)}
          />
          {t("linkRecommendation.includeReasons")}
        </label>
      </div>
      {hint ? <p className="link-rec-panel__hint">{hint}</p> : null}
      {listErr ? <p className="link-rec-panel__error">{listErr}</p> : null}
      {loading && recs.length === 0 ? (
        <p className="link-rec-panel__empty">{t("linkRecommendation.loading")}</p>
      ) : recs.length === 0 && !loading && !listErr ? (
        <p className="link-rec-panel__empty">{t("linkRecommendation.empty")}</p>
      ) : (
        <ul className="link-rec-panel__list">
          {recs.map((r) => (
            <li key={r.targetRelPath} className="link-rec-panel__item">
              <div className="link-rec-panel__item-top">
                <span className="link-rec-panel__item-name" title={r.targetRelPath}>
                  {displayTitleFromRelPath(r.targetRelPath)}
                </span>
                <span className="link-rec-panel__item-score">
                  {t("linkRecommendation.scoreLabel", { score: r.score.toFixed(3) })}
                </span>
              </div>
              {r.sharedTopics.length > 0 ? (
                <div
                  className="link-rec-panel__keywords"
                  role="group"
                  aria-label={t("linkRecommendation.keywordsAria")}
                >
                  <span className="link-rec-panel__keywords-label">{t("linkRecommendation.keywordsLabel")}</span>
                  <ul className="link-rec-panel__keywords-list">
                    {r.sharedTopics.map((k) => (
                      <li key={`${r.targetRelPath}:${k}`} className="link-rec-panel__keyword-chip">
                        {k}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {r.reason?.trim() ? (
                <p className="link-rec-panel__item-reason">{r.reason.trim()}</p>
              ) : null}
              <div className="link-rec-panel__item-actions">
                <button
                  type="button"
                  className="link-rec-panel__btn link-rec-panel__btn--icon"
                  title={t("linkRecommendation.insertAtCursorTooltip")}
                  aria-label={t("linkRecommendation.insertAtCursorTooltip")}
                  onClick={() => onInsertAtCursor(r.targetRelPath)}
                >
                  <IconInsertAtCursor />
                </button>
                <button
                  type="button"
                  className="link-rec-panel__btn link-rec-panel__btn--icon"
                  title={t("linkRecommendation.appendToRelatedSectionTooltip")}
                  aria-label={t("linkRecommendation.appendToRelatedSectionTooltip")}
                  onClick={() => onAppendToRelatedSection(r.targetRelPath)}
                >
                  <IconAppendRelated />
                </button>
                <button
                  type="button"
                  className="link-rec-panel__btn link-rec-panel__btn--icon"
                  title={t("linkRecommendation.ignoreTooltip")}
                  aria-label={t("linkRecommendation.ignoreTooltip")}
                  onClick={() => onIgnore(r.targetRelPath)}
                >
                  <IconIgnore />
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
