/**
 * DiscoveryFilterBar — Filter tabs + sort dropdown for the Discovery pane.
 */
import { useTranslation } from "react-i18next";

export interface DiscoveryReasonCounts {
  highSimilarity: number;
  crossDocRecurrence: number;
  semanticIsolated: number;
}

export interface DiscoveryFilterBarProps {
  counts: DiscoveryReasonCounts;
  activeFilter: string | null; // null = all
  onFilterChange: (reason: string | null) => void;
  sortBy: string;
  onSortChange: (sort: string) => void;
  /** LLM confirmation status filter: "confirmed" | "downgraded" | "unconfirmed" | null (all) */
  llmStatus: string | null;
  onLlmStatusChange: (status: string | null) => void;
  /** Number of LLM-confirmed candidates */
  confirmedCount: number;
}

type FilterOption = {
  key: string | null;
  labelKey: string;
  fallback: string;
  countFn: (c: DiscoveryReasonCounts) => number;
};

const FILTER_OPTIONS: FilterOption[] = [
  { key: null, labelKey: "discovery.filterAll", fallback: "All", countFn: (c) => c.highSimilarity + c.crossDocRecurrence + c.semanticIsolated },
  { key: "high_similarity", labelKey: "discovery.filterSimilarity", fallback: "Similar", countFn: (c) => c.highSimilarity },
  { key: "cross_doc_recurrence", labelKey: "discovery.filterCluster", fallback: "Cluster", countFn: (c) => c.crossDocRecurrence },
  { key: "semantic_isolated", labelKey: "discovery.filterIsolated", fallback: "Isolated", countFn: (c) => c.semanticIsolated },
];

type SortOption = {
  value: string;
  labelKey: string;
  fallback: string;
};

const SORT_OPTIONS: SortOption[] = [
  { value: "freshness", labelKey: "discovery.sortFreshness", fallback: "Freshness" },
  { value: "similarity", labelKey: "discovery.sortSimilarity", fallback: "Similarity" },
  { value: "age", labelKey: "discovery.sortAge", fallback: "Age" },
];

export function DiscoveryFilterBar({
  counts,
  activeFilter,
  onFilterChange,
  sortBy,
  onSortChange,
  llmStatus,
  onLlmStatusChange,
  confirmedCount,
}: DiscoveryFilterBarProps) {
  const { t } = useTranslation();

  return (
    <div className="discovery-filter-bar">
      <div className="discovery-filter-bar__tabs" role="tablist">
        {FILTER_OPTIONS.map((opt) => {
          const isActive = activeFilter === opt.key && !llmStatus;
          const count = opt.countFn(counts);
          return (
            <button
              key={opt.key ?? "all"}
              type="button"
              role="tab"
              className={`discovery-filter-bar__tab${isActive ? " discovery-filter-bar__tab--active" : ""}`}
              aria-selected={isActive}
              onClick={() => { onFilterChange(opt.key); onLlmStatusChange(null); }}
            >
              {t(opt.labelKey, opt.fallback)}
              <span className="discovery-filter-bar__count">{count}</span>
            </button>
          );
        })}
        {/* Spec 11: AI confirmed filter */}
        {confirmedCount > 0 && (
          <button
            type="button"
            role="tab"
            className={`discovery-filter-bar__tab discovery-filter-bar__tab--ai${llmStatus === "confirmed" ? " discovery-filter-bar__tab--active" : ""}`}
            aria-selected={llmStatus === "confirmed"}
            onClick={() => { onLlmStatusChange(llmStatus === "confirmed" ? null : "confirmed"); onFilterChange(null); }}
          >
            {t("discovery.filterConfirmed", "\u2713 AI")}
            <span className="discovery-filter-bar__count">{confirmedCount}</span>
          </button>
        )}
      </div>
      <div className="discovery-filter-bar__sort">
        <label className="discovery-filter-bar__sort-label" htmlFor="discovery-sort-select">
          {t("discovery.sortLabel", "Sort:")}
        </label>
        <select
          id="discovery-sort-select"
          className="discovery-filter-bar__sort-select"
          value={sortBy}
          onChange={(e) => onSortChange(e.target.value)}
        >
          {SORT_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {t(opt.labelKey, opt.fallback)}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
