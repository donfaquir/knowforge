import { invoke, isTauri } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ThoughtMgmtAiConversationPanel } from "./ThoughtMgmtAiConversationPanel";
import { ThoughtMgmtAiConversationToolbar } from "./ThoughtMgmtAiConversationToolbar";
import { ThoughtGrowthStoryCard } from "./ThoughtGrowthStoryCard";
import type { ThoughtFocusContext } from "../types/aiConversation";
import type { ThoughtDetail, VaultThoughtListPage, VaultThoughtListRow } from "../types/cognitiveTypes";
import "./ThoughtManagementPanel.css";

type FilterKey = "all" | "standalone" | "linked" | "temporary";

const FILTER_MENU_OPTIONS: { key: FilterKey; labelKey: string }[] = [
  { key: "all", labelKey: "thoughtManagement.filterAll" },
  { key: "standalone", labelKey: "thoughtManagement.filterStandalone" },
  { key: "linked", labelKey: "thoughtManagement.filterLinked" },
  { key: "temporary", labelKey: "thoughtManagement.filterTemporary" },
];

const THOUGHT_LIST_PAGE_SIZE = 10;

const TOOLBAR_ICON_STROKE = 1.65;
const FILTER_CHEVRON_STROKE = 2;

function IconChevronDown() {
  return (
    <svg
      className="thought-mgmt__filter-trigger-chevron"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={FILTER_CHEVRON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

function IconPlus() {
  return (
    <svg
      className="thought-mgmt__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M5 12h14" />
      <path d="M12 5v14" />
    </svg>
  );
}

function IconSearch() {
  return (
    <svg
      className="thought-mgmt__search-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <circle cx="11" cy="11" r="8" />
      <path d="m21 21-4.35-4.35" />
    </svg>
  );
}

function IconSave() {
  return (
    <svg
      className="thought-mgmt__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
      <path d="M17 21v-8H7v8" />
      <path d="M7 3v5h8" />
    </svg>
  );
}

function IconExternalNote() {
  return (
    <svg
      className="thought-mgmt__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
      <path d="M15 3h6v6" />
      <path d="M10 14 21 3" />
    </svg>
  );
}

function IconTrash() {
  return (
    <svg
      className="thought-mgmt__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M3 6h18" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
    </svg>
  );
}

function IconPanelClose() {
  return (
    <svg
      className="thought-mgmt__toolbar-icon-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={TOOLBAR_ICON_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

type Props = {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  onOpenNote: (relPath: string) => void;
  /** 正文相对已加载详情是否未保存，供顶栏退出等全局逻辑使用 */
  onThoughtDetailDirtyChange?: (dirty: boolean) => void;
  /** 判断给定路径是否为 kf-private */
  isPathKfPrivate?: (relPath: string) => boolean;
};

type DeleteThoughtResponse = {
  deleted: boolean;
  orphanCalloutMayRemain?: boolean;
};

function shortThoughtId(id: string, max = 14) {
  if (id.length <= max) return id;
  return `${id.slice(0, max)}…`;
}

function isoDateHead(iso: string, len = 10) {
  const s = iso.trim();
  return s.length >= len ? s.slice(0, len) : s;
}

export function ThoughtManagementPanel({
  workspaceReady,
  tauriRuntime,
  onOpenNote,
  onThoughtDetailDirtyChange,
  isPathKfPrivate,
}: Props) {
  const { t } = useTranslation();
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<FilterKey>("all");
  const [rows, setRows] = useState<VaultThoughtListRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [hint, setHint] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<ThoughtDetail | null>(null);
  const [editBody, setEditBody] = useState("");
  const [saving, setSaving] = useState(false);
  const [showNew, setShowNew] = useState(false);
  const [newBody, setNewBody] = useState("");
  const [filterMenuOpen, setFilterMenuOpen] = useState(false);
  const [growthStoryOpen, setGrowthStoryOpen] = useState(false);
  const filterPopoverRef = useRef<HTMLDivElement>(null);
  const [listPage, setListPage] = useState(1);
  const [totalCount, setTotalCount] = useState(0);
  const listPageRef = useRef(1);
  listPageRef.current = listPage;
  /** 保存后 refresh：当前页可能不含该条，避免列表 effect 把选中改到首条从而重载详情覆盖编辑区 */
  const preserveSelectionAfterSaveThoughtIdRef = useRef<string | null>(null);

  const filterLabel = useMemo(() => {
    const opt = FILTER_MENU_OPTIONS.find((o) => o.key === filter);
    return opt ? t(opt.labelKey) : "";
  }, [filter, t]);

  const qRef = useRef(q);
  const filterRef = useRef(filter);
  qRef.current = q;
  filterRef.current = filter;

  const refresh = useCallback(async () => {
    if (!workspaceReady || !tauriRuntime || !isTauri()) {
      return;
    }
    setLoading(true);
    setErr(null);
    try {
      const f = filterRef.current;
      const page = listPageRef.current;
      const offset = (page - 1) * THOUGHT_LIST_PAGE_SIZE;
      const res = await invoke<VaultThoughtListPage>("list_vault_thoughts", {
        args: {
          query: qRef.current.trim() || null,
          limit: THOUGHT_LIST_PAGE_SIZE,
          offset,
          filter: f === "all" ? null : f,
        },
      });
      setRows(res.rows);
      setTotalCount(res.total);
    } catch (e) {
      setRows([]);
      setTotalCount(0);
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [workspaceReady, tauriRuntime]);

  useEffect(() => {
    listPageRef.current = 1;
    setListPage(1);
  }, [q, filter]);

  useEffect(() => {
    if (!filterMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (filterPopoverRef.current && !filterPopoverRef.current.contains(e.target as Node)) {
        setFilterMenuOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setFilterMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [filterMenuOpen]);

  useEffect(() => {
    const delay = q.trim() ? 280 : 0;
    const id = window.setTimeout(() => {
      void refresh();
    }, delay);
    return () => window.clearTimeout(id);
  }, [q, filter, listPage, refresh]);

  const totalPages = Math.max(1, Math.ceil(totalCount / THOUGHT_LIST_PAGE_SIZE));

  useEffect(() => {
    if (listPage > totalPages) {
      const next = totalPages;
      listPageRef.current = next;
      setListPage(next);
    }
  }, [listPage, totalPages]);

  // 列表加载后：无选中或当前选中不在本页时，默认选中第一条；保存触发的 refresh 则优先保留刚保存的 thoughtId
  useEffect(() => {
    if (loading) return;
    const keepId = preserveSelectionAfterSaveThoughtIdRef.current;
    if (keepId) {
      preserveSelectionAfterSaveThoughtIdRef.current = null;
      if (rows.length > 0) {
        setSelectedId(keepId);
      }
      return;
    }
    setSelectedId((prev) => {
      if (rows.length === 0) return null;
      if (prev && rows.some((r) => r.thoughtId === prev)) return prev;
      return rows[0].thoughtId;
    });
  }, [rows, loading]);

  useEffect(() => {
    if (!selectedId || !workspaceReady || !tauriRuntime || !isTauri()) {
      setDetail(null);
      setEditBody("");
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const d = await invoke<ThoughtDetail | null>("get_thought_detail", {
          args: { thoughtId: selectedId },
        });
        if (!cancelled) {
          setDetail(d);
          setEditBody(d?.body ?? "");
        }
      } catch {
        if (!cancelled) {
          setDetail(null);
          setEditBody("");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectedId, workspaceReady, tauriRuntime]);

  const thoughtFocusFromDetail = useMemo((): ThoughtFocusContext | null => {
    if (!detail) {
      return null;
    }
    return {
      thoughtId: detail.thoughtId,
      thoughtBody: editBody,
      maturity: detail.maturity,
    };
  }, [detail, editBody]);

  const linkedNoteRelPath = detail && !detail.standalone ? detail.noteRelPath : null;

  const isBodyDirty = useMemo(
    () => detail != null && editBody !== detail.body,
    [detail, editBody],
  );

  useEffect(() => {
    onThoughtDetailDirtyChange?.(isBodyDirty);
    return () => onThoughtDetailDirtyChange?.(false);
  }, [isBodyDirty, onThoughtDetailDirtyChange]);

  /** 若正文已改未存，弹窗确认是否放弃；确认则返回 true 可继续导航 */
  const confirmDiscardBodyIfDirty = useCallback(async (): Promise<boolean> => {
    if (!detail || editBody === detail.body) return true;
    const ok = await ask(t("thoughtManagement.leaveDetailUnsavedMessage"), {
      title: t("thoughtManagement.exitUnsavedTitle"),
      kind: "warning",
    });
    return !!ok;
  }, [detail, editBody, t]);

  const onSave = useCallback(async () => {
    if (!detail || !isTauri()) return;
    const savedThoughtId = detail.thoughtId;
    setSaving(true);
    setErr(null);
    try {
      await invoke("update_thought_body", {
        args: {
          thoughtId: savedThoughtId,
          body: editBody,
          summary: detail.summary,
        },
      });
      setDetail((prev) => (prev ? { ...prev, body: editBody } : null));
      preserveSelectionAfterSaveThoughtIdRef.current = savedThoughtId;
      await refresh();
      try {
        const d = await invoke<ThoughtDetail | null>("get_thought_detail", {
          args: { thoughtId: savedThoughtId },
        });
        if (d) {
          setDetail(d);
          setEditBody(d.body);
        }
      } catch {
        /* 列表已刷新；详情元数据拉取失败不覆盖已保存正文 */
      }
    } catch (e) {
      preserveSelectionAfterSaveThoughtIdRef.current = null;
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [detail, editBody, refresh]);

  const openLinkedNoteWithOptionalConfirm = useCallback(
    async (relPath: string) => {
      if (!(await confirmDiscardBodyIfDirty())) return;
      onOpenNote(relPath);
    },
    [confirmDiscardBodyIfDirty, onOpenNote],
  );

  const closeDetailWithOptionalConfirm = useCallback(async () => {
    if (!(await confirmDiscardBodyIfDirty())) return;
    setSelectedId(null);
  }, [confirmDiscardBodyIfDirty]);

  const onDelete = useCallback(async () => {
    if (!detail || !isTauri()) return;
    const ok = await ask(t("thoughtManagement.confirmDelete"), {
      title: t("dialogs.delete"),
      kind: "warning",
    });
    if (!ok) {
      return;
    }
    setErr(null);
    setHint(null);
    try {
      const res = await invoke<DeleteThoughtResponse>("delete_thought", {
        args: { thoughtId: detail.thoughtId },
      });
      if (res.orphanCalloutMayRemain) {
        setHint(t("thoughtManagement.orphanCalloutHint"));
      }
      setSelectedId(null);
      setDetail(null);
      await refresh();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }, [detail, refresh, t]);

  const onCreateStandalone = useCallback(async () => {
    const body = newBody.trim();
    if (!body || !isTauri()) return;
    setErr(null);
    try {
      const tid = await invoke<string>("create_standalone_thought", {
        args: { body, summary: null },
      });
      setNewBody("");
      setShowNew(false);
      listPageRef.current = 1;
      setListPage(1);
      await refresh();
      setSelectedId(tid);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }, [newBody, refresh]);

  if (!workspaceReady || !tauriRuntime || !isTauri()) {
    return null;
  }

  return (
    <div className="thought-mgmt">
      <div className="thought-mgmt__body">
        <aside className="thought-mgmt__nav" aria-label={t("thoughtManagement.leftNavAria")}>
          <div className="thought-mgmt__nav-toolbar">
            <div className="thought-mgmt__nav-search-row">
              <div className="thought-mgmt__search-wrap">
                <span className="thought-mgmt__search-icon" aria-hidden={true}>
                  <IconSearch />
                </span>
                <input
                  className="thought-mgmt__search"
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                  placeholder={t("thoughtManagement.searchPlaceholder")}
                  aria-label={t("thoughtManagement.searchPlaceholder")}
                />
              </div>
            </div>
            <div className="thought-mgmt__nav-actions-row">
              <div className="thought-mgmt__nav-inline-actions">
                <div className="thought-mgmt__filter-popover" ref={filterPopoverRef}>
                  <button
                    type="button"
                    className={`thought-mgmt__filter-trigger${filterMenuOpen ? " is-open" : ""}`}
                    id="thought-mgmt-filter-trigger"
                    aria-expanded={filterMenuOpen}
                    aria-haspopup="menu"
                    title={`${t("thoughtManagement.filterButtonTitle")}: ${filterLabel}`}
                    aria-label={`${t("thoughtManagement.filterButtonTitle")}: ${filterLabel}`}
                    onClick={() => {
                      setShowNew(false);
                      setFilterMenuOpen((o) => !o);
                    }}
                  >
                    <span className="thought-mgmt__filter-trigger-label">{filterLabel}</span>
                    <IconChevronDown />
                  </button>
                  {filterMenuOpen ? (
                    <div
                      className="thought-mgmt__filter-menu"
                      role="menu"
                      aria-labelledby="thought-mgmt-filter-trigger"
                    >
                      {FILTER_MENU_OPTIONS.map((opt) => (
                        <button
                          key={opt.key}
                          type="button"
                          role="menuitemradio"
                          aria-checked={filter === opt.key}
                          className={`thought-mgmt__filter-menu-item${filter === opt.key ? " is-selected" : ""}`}
                          onClick={() => {
                            setFilter(opt.key);
                            setFilterMenuOpen(false);
                          }}
                        >
                          <span className="thought-mgmt__filter-menu-item-label">{t(opt.labelKey)}</span>
                          {filter === opt.key ? (
                            <span className="thought-mgmt__filter-menu-check" aria-hidden={true}>
                              ✓
                            </span>
                          ) : null}
                        </button>
                      ))}
                    </div>
                  ) : null}
                </div>
                <button
                  type="button"
                  className={`thought-mgmt__toolbar-icon-btn${showNew ? " is-active" : ""}`}
                  title={t("thoughtManagement.newStandalone")}
                  aria-label={t("thoughtManagement.newStandalone")}
                  onClick={() => {
                    setFilterMenuOpen(false);
                    setShowNew((v) => !v);
                  }}
                >
                  <IconPlus />
                </button>
              </div>
            </div>
          </div>
          <div className="thought-mgmt__nav-toolbar-divider" role="separator" aria-hidden={true} />
          {showNew ? (
            <div className="thought-mgmt__new-form">
              <textarea
                className="thought-mgmt__textarea thought-mgmt__textarea--compact"
                value={newBody}
                onChange={(e) => setNewBody(e.target.value)}
                placeholder={t("thoughtManagement.newBodyPlaceholder")}
                aria-label={t("thoughtManagement.newBodyPlaceholder")}
              />
              <div className="thought-mgmt__actions">
                <button type="button" className="thought-mgmt__btn" onClick={() => void onCreateStandalone()}>
                  {t("thoughtManagement.create")}
                </button>
                <button
                  type="button"
                  className="thought-mgmt__btn"
                  onClick={() => {
                    setShowNew(false);
                    setNewBody("");
                  }}
                >
                  {t("thoughtManagement.cancel")}
                </button>
              </div>
            </div>
          ) : null}
          {loading ? <p className="thought-mgmt__muted thought-mgmt__nav-status">{t("thoughtManagement.loading")}</p> : null}
          {err ? (
            <p className="thought-mgmt__err thought-mgmt__nav-status" role="alert">
              {err}
            </p>
          ) : null}
          {hint ? <p className="thought-mgmt__muted thought-mgmt__nav-status">{hint}</p> : null}
          {!loading && rows.length === 0 ? (
            <p className="thought-mgmt__muted thought-mgmt__nav-status">{t("thoughtManagement.empty")}</p>
          ) : null}
          <ul className="thought-mgmt__list">
            {rows.map((r) => (
              <li key={`${r.relPath}-${r.thoughtId}`} className="thought-mgmt__li">
                <button
                  type="button"
                  className={`thought-mgmt__row thought-mgmt__row--compact${
                    selectedId === r.thoughtId ? " is-active" : ""
                  }`}
                  onClick={() => {
                    void (async () => {
                      if (selectedId === r.thoughtId) return;
                      if (!(await confirmDiscardBodyIfDirty())) return;
                      setSelectedId(r.thoughtId);
                    })();
                  }}
                >
                  <span className="thought-mgmt__row-excerpt">{r.excerpt || "—"}</span>
                  <span className="thought-mgmt__row-foot">
                    <code className="thought-mgmt__row-id">{shortThoughtId(r.thoughtId)}</code>
                    <span className="thought-mgmt__row-meta">
                      {r.maturity}
                      {r.standalone ? ` · ${t("thoughtManagement.sourceStandalone")}` : ""}
                      {r.temporary ? ` · ${t("thoughtPanel.temporary")}` : ""}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {totalCount > 0 ? (
            <div className="thought-mgmt__pager" aria-label={t("thoughtManagement.pagerAria")}>
              <button
                type="button"
                className="thought-mgmt__pager-btn"
                disabled={listPage <= 1 || loading}
                onClick={() => setListPage((p) => Math.max(1, p - 1))}
                aria-label={t("thoughtManagement.prevPage")}
              >
                {t("thoughtManagement.prevPage")}
              </button>
              <span className="thought-mgmt__pager-meta">
                {t("thoughtManagement.pageIndicator", { current: listPage, total: totalPages })}
              </span>
              <button
                type="button"
                className="thought-mgmt__pager-btn"
                disabled={listPage >= totalPages || loading}
                onClick={() => setListPage((p) => p + 1)}
                aria-label={t("thoughtManagement.nextPage")}
              >
                {t("thoughtManagement.nextPage")}
              </button>
            </div>
          ) : null}
        </aside>
        <main className="thought-mgmt__main">
          <div className="thought-mgmt__detail-scroll">
            {!detail ? (
              <div className="thought-mgmt__empty-detail">
                <p className="thought-mgmt__muted">{t("thoughtManagement.selectHint")}</p>
              </div>
            ) : (
              <div className="thought-mgmt__detail">
                <h2 className="thought-mgmt__sr-only">{t("thoughtManagement.detailHeading")}</h2>
                <label className="thought-mgmt__sr-only" htmlFor="thought-mgmt-body-edit">
                  {t("thoughtManagement.fieldBody")}
                </label>
                <div className="thought-mgmt__body-editor-shell">
                  <textarea
                    id="thought-mgmt-body-edit"
                    className="thought-mgmt__textarea thought-mgmt__textarea--body-editor"
                    value={editBody}
                    onChange={(e) => setEditBody(e.target.value)}
                    aria-label={t("thoughtManagement.edit")}
                  />
                  <div className="thought-mgmt__body-editor-toolbar" role="toolbar" aria-label={t("thoughtManagement.editorToolbarAria")}>
                    <button
                      type="button"
                      className={`thought-mgmt__toolbar-icon-btn${
                        isBodyDirty && !saving ? " thought-mgmt__toolbar-icon-btn--unsaved" : ""
                      }`}
                      disabled={saving}
                      title={
                        saving
                          ? t("toolbar.saving")
                          : isBodyDirty
                            ? `${t("thoughtManagement.save")} — ${t("thoughtManagement.saveUnsavedHint")}`
                            : t("thoughtManagement.save")
                      }
                      aria-label={
                        saving
                          ? t("toolbar.saving")
                          : isBodyDirty
                            ? `${t("thoughtManagement.save")} — ${t("thoughtManagement.saveUnsavedHint")}`
                            : t("thoughtManagement.save")
                      }
                      onClick={() => void onSave()}
                    >
                      <IconSave />
                    </button>
                    {!detail.standalone ? (
                      <button
                        type="button"
                        className="thought-mgmt__toolbar-icon-btn"
                        title={t("thoughtManagement.openLinkedNote")}
                        aria-label={t("thoughtManagement.openLinkedNote")}
                        onClick={() => void openLinkedNoteWithOptionalConfirm(detail.noteRelPath)}
                      >
                        <IconExternalNote />
                      </button>
                    ) : null}
                    <button
                      type="button"
                      className="thought-mgmt__toolbar-icon-btn thought-mgmt__toolbar-icon-btn--danger"
                      title={t("thoughtManagement.delete")}
                      aria-label={t("thoughtManagement.delete")}
                      onClick={() => void onDelete()}
                    >
                      <IconTrash />
                    </button>
                    <button
                      type="button"
                      className="thought-mgmt__toolbar-icon-btn"
                      title={t("thoughtManagement.closeEditor")}
                      aria-label={t("thoughtManagement.closeEditor")}
                      onClick={() => void closeDetailWithOptionalConfirm()}
                    >
                      <IconPanelClose />
                    </button>
                  </div>
                </div>
                <div className="thought-mgmt__detail-meta-strip" aria-label={t("thoughtManagement.detailMetaAria")}>
                  <div className="thought-mgmt__detail-meta-row">
                    <code className="thought-mgmt__detail-meta-id" title={detail.thoughtId}>
                      {shortThoughtId(detail.thoughtId, 22)}
                    </code>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span>{detail.maturity}</span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span
                      className="thought-mgmt__detail-meta-path"
                      title={detail.standalone ? t("thoughtManagement.sourceStandalone") : detail.noteRelPath}
                    >
                      {detail.standalone ? t("thoughtManagement.sourceStandalone") : detail.noteRelPath}
                    </span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span>
                      {detail.temporary ? t("thoughtPanel.temporary") : t("thoughtManagement.flagNormal")}
                    </span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    {(() => {
                      // Check if thought is private
                      const isPrivate = isPathKfPrivate
                        && !detail.standalone
                        && detail.noteRelPath
                        && isPathKfPrivate(detail.noteRelPath);
                      if (isPrivate) return null;
                      return (
                        <button
                          className="thought-mgmt__detail-growth-story-btn"
                          onClick={() => setGrowthStoryOpen(true)}
                          title={t("growthStory.viewGrowthStory", "查看成长故事")}
                        >
                          {t("growthStory.viewGrowthStory", "成长故事")}
                        </button>
                      );
                    })()}
                  </div>
                  <div className="thought-mgmt__detail-meta-row thought-mgmt__detail-meta-row--sub">
                    <span title={detail.createdAt}>
                      {t("thoughtManagement.metaCreatedShort")} {isoDateHead(detail.createdAt)}
                    </span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span title={detail.updatedAt}>
                      {t("thoughtManagement.metaUpdatedShort")} {isoDateHead(detail.updatedAt)}
                    </span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span>
                      {t("thoughtManagement.metaChallengeShort")} {detail.challengePassCount}
                    </span>
                    <span className="thought-mgmt__detail-meta-sep" aria-hidden={true}>
                      ·
                    </span>
                    <span title={detail.lastReviewedAt ?? undefined}>
                      {t("thoughtManagement.metaReviewShort")}{" "}
                      {detail.lastReviewedAt ? isoDateHead(detail.lastReviewedAt) : "—"}
                    </span>
                  </div>
                  {detail.summary ? (
                    <p className="thought-mgmt__detail-summary-hint">{detail.summary}</p>
                  ) : null}
                </div>
              </div>
            )}
          </div>
        </main>
        <aside
          className="thought-mgmt__ai-panel"
          aria-label={t("thoughtManagement.rightAiPanelAria")}
        >
          <div className="thought-mgmt__ai-caption-row" data-tauri-drag-region-exclude={true}>
            <span className="thought-mgmt__ai-caption-title">{t("thoughtManagement.aiDockCaption")}</span>
            <ThoughtMgmtAiConversationToolbar />
          </div>
          <div className="thought-mgmt__ai-inner">
            <ThoughtMgmtAiConversationPanel
              thoughtFocusFromDetail={thoughtFocusFromDetail}
              linkedNoteRelPath={linkedNoteRelPath}
            />
          </div>
        </aside>
      </div>
      {detail && (
        <ThoughtGrowthStoryCard
          thoughtId={detail.thoughtId}
          open={growthStoryOpen}
          onClose={() => setGrowthStoryOpen(false)}
        />
      )}
    </div>
  );
}
