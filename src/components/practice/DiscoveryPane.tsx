/**
 * DiscoveryPane — Minimal discovery sub-tab for Practice Mode.
 * Lists latent candidates with type filtering, sorting, and promote/dismiss actions.
 */
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { DiscoveryFilterBar, type DiscoveryReasonCounts } from "./DiscoveryFilterBar";
import "./DiscoveryPane.css";

// ---------------------------------------------------------------------------
// Types aligned with Rust backend (camelCase via serde rename)
// ---------------------------------------------------------------------------

export interface CandidateForUi {
  id: string;
  relPath: string;
  excerpt: string;
  markingReason: string;
  similarityScore: number | null;
  pairedRelPath: string | null;
  startLine: number;
  endLine: number;
  // LLM confirmation (Spec 11)
  llmConfirmed: "confirmed" | "downgraded" | "rejected" | null;
  llmReason: string | null;
}

interface DiscoveryFilter {
  markingReason: string | null;
  sortBy: string | null;
  llmStatus: string | null;
  offset: number;
  limit: number;
}

interface DiscoveryListResponse {
  items: CandidateForUi[];
  total: number;
  byReason: DiscoveryReasonCounts;
  confirmedCount: number;
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface DiscoveryPaneProps {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  onSelectCandidate: (candidate: CandidateForUi | null) => void;
  /** Increment to force a data refetch (e.g. after promote/dismiss from detail view) */
  refreshKey?: number;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PAGE_SIZE = 30;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function DiscoveryPane({
  workspaceReady,
  tauriRuntime,
  onSelectCandidate,
  refreshKey = 0,
}: DiscoveryPaneProps) {
  const { t } = useTranslation();

  // Filter / sort / pagination state
  const [filterReason, setFilterReason] = useState<string | null>(null);
  const [sortBy, setSortBy] = useState<string>("freshness");
  const [llmStatus, setLlmStatus] = useState<string | null>(null);
  const [offset, setOffset] = useState(0);

  // Data
  const [response, setResponse] = useState<DiscoveryListResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Selection (single-click for detail view)
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Multi-select for batch operations
  const [checkedIds, setCheckedIds] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const isSelecting = checkedIds.size > 0;

  // Computed filter object for invoke
  const filter: DiscoveryFilter = useMemo(
    () => ({ markingReason: filterReason, sortBy, llmStatus, offset, limit: PAGE_SIZE }),
    [filterReason, sortBy, llmStatus, offset],
  );

  // Fetch data
  const fetchData = useCallback(async () => {
    if (!workspaceReady || !tauriRuntime) return;
    setLoading(true);
    setError(null);
    try {
      const res = await invoke<DiscoveryListResponse>("list_discovery_candidates", { filter });
      setResponse(res);
    } catch (e) {
      setError(String(e));
      setResponse(null);
    } finally {
      setLoading(false);
    }
  }, [workspaceReady, tauriRuntime, filter]);

  useEffect(() => {
    void fetchData();
  }, [fetchData, refreshKey]);

  // Reset offset when filter/sort changes
  useEffect(() => {
    setOffset(0);
  }, [filterReason, sortBy, llmStatus]);

  // Counts (default zeros before first load)
  const counts: DiscoveryReasonCounts = response?.byReason ?? {
    highSimilarity: 0,
    crossDocRecurrence: 0,
    semanticIsolated: 0,
  };

  // Handlers
  const handleSelect = useCallback(
    (candidate: CandidateForUi) => {
      const newId = candidate.id === selectedId ? null : candidate.id;
      setSelectedId(newId);
      onSelectCandidate(newId ? candidate : null);
    },
    [selectedId, onSelectCandidate],
  );

  const handlePromote = useCallback(
    async (candidateId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        await invoke<string>("promote_candidate_to_thought", { candidateId });
        // Optimistic removal
        setResponse((prev) => {
          if (!prev) return prev;
          const items = prev.items.filter((i) => i.id !== candidateId);
          return { ...prev, items, total: prev.total - 1 };
        });
        if (selectedId === candidateId) {
          setSelectedId(null);
          onSelectCandidate(null);
        }
        // Re-fetch to update counts
        void fetchData();
      } catch (err) {
        console.error("promote_candidate_to_thought failed:", err);
      }
    },
    [selectedId, onSelectCandidate, fetchData],
  );

  const handleDismiss = useCallback(
    async (candidateId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        await invoke<void>("dismiss_latent_candidate", { candidateId });
        // Optimistic removal
        setResponse((prev) => {
          if (!prev) return prev;
          const items = prev.items.filter((i) => i.id !== candidateId);
          return { ...prev, items, total: prev.total - 1 };
        });
        if (selectedId === candidateId) {
          setSelectedId(null);
          onSelectCandidate(null);
        }
        // Re-fetch to update counts
        void fetchData();
      } catch (err) {
        console.error("dismiss_latent_candidate failed:", err);
      }
    },
    [selectedId, onSelectCandidate, fetchData],
  );

  // --- Multi-select handlers ---

  const handleToggleCheck = useCallback((id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    setCheckedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleSelectAll = useCallback(() => {
    if (!response) return;
    const allIds = new Set(response.items.map((i) => i.id));
    setCheckedIds(allIds);
  }, [response]);

  const handleClearSelection = useCallback(() => {
    setCheckedIds(new Set());
  }, []);

  const handleBatchDismiss = useCallback(async () => {
    if (checkedIds.size === 0 || batchBusy) return;
    setBatchBusy(true);
    try {
      const ids = Array.from(checkedIds);
      await invoke<number>("batch_dismiss_candidates", { candidateIds: ids });
      setCheckedIds(new Set());
      setSelectedId(null);
      onSelectCandidate(null);
      void fetchData();
    } catch (err) {
      console.error("batch_dismiss_candidates failed:", err);
    } finally {
      setBatchBusy(false);
    }
  }, [checkedIds, batchBusy, onSelectCandidate, fetchData]);

  const handleBatchPromote = useCallback(async () => {
    if (checkedIds.size === 0 || batchBusy) return;
    setBatchBusy(true);
    try {
      const ids = Array.from(checkedIds);
      await invoke<string[]>("batch_promote_candidates", { candidateIds: ids });
      setCheckedIds(new Set());
      setSelectedId(null);
      onSelectCandidate(null);
      void fetchData();
    } catch (err) {
      console.error("batch_promote_candidates failed:", err);
    } finally {
      setBatchBusy(false);
    }
  }, [checkedIds, batchBusy, onSelectCandidate, fetchData]);

  // Pagination
  const totalPages = response ? Math.ceil(response.total / PAGE_SIZE) : 0;
  const currentPage = Math.floor(offset / PAGE_SIZE) + 1;

  // LLM lazy confirmation (Spec 11)
  useLazyLlmConfirm(response?.items, setResponse);

  // ---------------------------------------------------------------------------
  // Empty state rendering
  // ---------------------------------------------------------------------------

  const renderEmptyState = () => {
    if (loading) {
      return (
        <div className="discovery-pane__empty">
          <span className="discovery-pane__spinner" />
          <p>{t("discovery.loading", "Analyzing notes...")}</p>
        </div>
      );
    }
    if (error) {
      return (
        <div className="discovery-pane__empty discovery-pane__empty--error">
          <p>{t("discovery.error", "Failed to load candidates")}</p>
          <p className="discovery-pane__error-detail">{error}</p>
        </div>
      );
    }
    if (!response || response.total === 0) {
      if (filterReason) {
        return (
          <div className="discovery-pane__empty">
            <p>{t("discovery.emptyFilter", "No candidates of this type")}</p>
          </div>
        );
      }
      return (
        <div className="discovery-pane__empty">
          <p>{t("discovery.emptyAll", "All discoveries reviewed!")}</p>
        </div>
      );
    }
    return null;
  };

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div className="discovery-pane">
      <DiscoveryFilterBar
        counts={counts}
        activeFilter={filterReason}
        onFilterChange={setFilterReason}
        sortBy={sortBy}
        onSortChange={setSortBy}
        llmStatus={llmStatus}
        onLlmStatusChange={setLlmStatus}
        confirmedCount={response?.confirmedCount ?? 0}
      />

      <div className="discovery-pane__list">
        {renderEmptyState() ?? (
          <>
            {response!.items.map((item) => (
              <CandidateCard
                key={item.id}
                item={item}
                selected={item.id === selectedId}
                checked={checkedIds.has(item.id)}
                isSelecting={isSelecting}
                onSelect={handleSelect}
                onToggleCheck={handleToggleCheck}
                onPromote={handlePromote}
                onDismiss={handleDismiss}
              />
            ))}

            {totalPages > 1 && (
              <div className="discovery-pane__pagination">
                <button
                  type="button"
                  className="discovery-pane__page-btn"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
                >
                  {t("discovery.prevPage", "Prev")}
                </button>
                <span className="discovery-pane__page-info">
                  {currentPage} / {totalPages}
                </span>
                <button
                  type="button"
                  className="discovery-pane__page-btn"
                  disabled={currentPage >= totalPages}
                  onClick={() => setOffset(offset + PAGE_SIZE)}
                >
                  {t("discovery.nextPage", "Next")}
                </button>
              </div>
            )}
          </>
        )}
      </div>

      {/* Batch operations bar */}
      {response && response.items.length > 0 && (
        <div className={`discovery-pane__batch${isSelecting ? " discovery-pane__batch--active" : ""}`}>
          <label className="discovery-pane__batch-select-all">
            <input
              type="checkbox"
              checked={checkedIds.size === response.items.length && response.items.length > 0}
              onChange={(e) => { if (e.target.checked) handleSelectAll(); else handleClearSelection(); }}
            />
            {t("discovery.batch.selectAll", "Select all")} ({response.items.length})
          </label>
          {isSelecting && (
            <>
              <span className="discovery-pane__batch-count">
                {t("discovery.batch.selected", "{{count}} selected", { count: checkedIds.size })}
              </span>
              <button
                type="button"
                className="discovery-pane__batch-btn discovery-pane__batch-btn--dismiss"
                disabled={batchBusy}
                onClick={() => void handleBatchDismiss()}
              >
                {t("discovery.batch.dismiss", "Batch dismiss")}
              </button>
              <button
                type="button"
                className="discovery-pane__batch-btn discovery-pane__batch-btn--promote"
                disabled={batchBusy}
                onClick={() => void handleBatchPromote()}
              >
                {t("discovery.batch.promote", "Batch promote")}
              </button>
              <button
                type="button"
                className="discovery-pane__batch-btn discovery-pane__batch-btn--clear"
                disabled={batchBusy}
                onClick={handleClearSelection}
              >
                {t("discovery.batch.clear", "Clear")}
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// CandidateCard sub-component
// ---------------------------------------------------------------------------

interface CandidateCardProps {
  item: CandidateForUi;
  selected: boolean;
  checked: boolean;
  isSelecting: boolean;
  onSelect: (item: CandidateForUi) => void;
  onToggleCheck: (id: string, e: React.MouseEvent) => void;
  onPromote: (id: string, e: React.MouseEvent) => void;
  onDismiss: (id: string, e: React.MouseEvent) => void;
}

function CandidateCard({ item, selected, checked, isSelecting, onSelect, onToggleCheck, onPromote, onDismiss }: CandidateCardProps) {
  const { t } = useTranslation();

  const reasonLabel = useMemo(() => {
    switch (item.markingReason) {
      case "high_similarity":
        return `\u{1F4C4}\u00D7\u{1F4C4} \u00B7 ${item.similarityScore != null ? item.similarityScore.toFixed(2) : ""}`;
      case "cross_doc_recurrence":
        return `\u{1F4C4}\u00D7N \u00B7 ${t("discovery.cluster", "cluster")}`;
      case "semantic_isolated":
        return `\u{1F4C4} \u00B7 ${t("discovery.isolated", "isolated")}`;
      default:
        return item.markingReason;
    }
  }, [item.markingReason, item.similarityScore, t]);

  const fileName = item.relPath.split("/").pop()?.replace(/\.md$/i, "") ?? item.relPath;

  const llmClass = item.llmConfirmed === "confirmed" ? " discovery-card--confirmed"
    : item.llmConfirmed === "downgraded" ? " discovery-card--downgraded" : "";

  return (
    <div
      className={`discovery-card${selected ? " discovery-card--selected" : ""}${checked ? " discovery-card--checked" : ""}${llmClass}`}
      onClick={() => onSelect(item)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") onSelect(item); }}
    >
      {isSelecting && (
        <input
          type="checkbox"
          className="discovery-card__checkbox"
          checked={checked}
          onClick={(e) => onToggleCheck(item.id, e as unknown as React.MouseEvent)}
          onChange={() => {/* controlled via onClick */}}
        />
      )}
      <div className="discovery-card__body">
        <div className="discovery-card__excerpt">{item.excerpt}</div>
        <div className="discovery-card__meta">
          <span className="discovery-card__reason">{reasonLabel}</span>
          <LlmConfirmBadge status={item.llmConfirmed} />
          <span className="discovery-card__file" title={item.relPath}>{fileName}</span>
        </div>
        {item.llmReason && (
          <div className="discovery-card__llm-reason">
            <span className="discovery-card__llm-reason-icon" aria-hidden>&#128161;</span>
            <span className="discovery-card__llm-reason-text">{item.llmReason}</span>
          </div>
        )}
        {!isSelecting && (
          <div className="discovery-card__actions">
            <button
              type="button"
              className="discovery-card__btn discovery-card__btn--promote"
              onClick={(e) => onPromote(item.id, e)}
              title={t("discovery.promote", "Promote")}
            >
              {t("discovery.promote", "Promote")}
            </button>
            <button
              type="button"
              className="discovery-card__btn discovery-card__btn--dismiss"
              onClick={(e) => onDismiss(item.id, e)}
              title={t("discovery.dismiss", "Dismiss")}
            >
              {t("discovery.dismiss", "Dismiss")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// LLM Confirm Badge sub-component (Spec 11)
// ---------------------------------------------------------------------------

function LlmConfirmBadge({ status }: { status: string | null }) {
  if (!status) {
    return <span className="discovery-card__llm-badge discovery-card__llm-badge--pending" title="Awaiting AI review">&#9203;</span>;
  }
  switch (status) {
    case "confirmed":
      return <span className="discovery-card__llm-badge discovery-card__llm-badge--confirmed" title="AI confirmed">&#10003;</span>;
    case "downgraded":
      return <span className="discovery-card__llm-badge discovery-card__llm-badge--downgraded" title="Low priority">&#9675;</span>;
    case "rejected":
      return null; // rejected items should be filtered out by backend
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// LLM lazy confirmation hook (Spec 11)
// ---------------------------------------------------------------------------

interface ConfirmResult {
  candidateId: string;
  verdict: string;
  reason: string;
}

/**
 * Trigger LLM confirmation for unconfirmed candidates in the current view.
 * Runs once per data load — non-blocking, silent on failure.
 */
export function useLazyLlmConfirm(
  items: CandidateForUi[] | undefined,
  setResponse: React.Dispatch<React.SetStateAction<DiscoveryListResponse | null>>,
) {
  const confirmedRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!items || items.length === 0) return;

    // Find unconfirmed items not already in-flight
    const unconfirmed = items
      .filter((i) => i.llmConfirmed === null && !confirmedRef.current.has(i.id))
      .slice(0, 5)
      .map((i) => i.id);

    if (unconfirmed.length === 0) return;

    // Mark as in-flight to prevent duplicate requests
    for (const id of unconfirmed) {
      confirmedRef.current.add(id);
    }

    void invoke<ConfirmResult[]>("confirm_discovery_batch", { candidateIds: unconfirmed })
      .then((results) => {
        if (!results || results.length === 0) return;
        setResponse((prev) => {
          if (!prev) return prev;
          const updated = prev.items.map((item) => {
            const result = results.find((r) => r.candidateId === item.id);
            if (!result) return item;
            return {
              ...item,
              llmConfirmed: result.verdict as CandidateForUi["llmConfirmed"],
              llmReason: result.reason,
            };
          });
          return { ...prev, items: updated };
        });
      })
      .catch(() => {
        // Silent failure — LLM confirmation is a pure enhancement layer
      });
  }, [items, setResponse]);
}
