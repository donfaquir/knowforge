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
}: DiscoveryFilterBarProps) {
  const { t } = useTranslation();

  return (
    <div className="discovery-filter-bar">
      <div className="discovery-filter-bar__tabs" role="tablist">
        {FILTER_OPTIONS.map((opt) => {
          const isActive = activeFilter === opt.key;
          const count = opt.countFn(counts);
          return (
            <button
              key={opt.key ?? "all"}
              type="button"
              role="tab"
              className={`discovery-filter-bar__tab${isActive ? " discovery-filter-bar__tab--active" : ""}`}
              aria-selected={isActive}
              onClick={() => onFilterChange(opt.key)}
            >
              {t(opt.labelKey, opt.fallback)}
              <span className="discovery-filter-bar__count">{count}</span>
            </button>
          );
        })}
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
