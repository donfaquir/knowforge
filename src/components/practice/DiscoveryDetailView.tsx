/**
 * DiscoveryDetailView — Routes to the appropriate detail sub-component
 * based on the candidate's marking_reason.
 */
import { useTranslation } from "react-i18next";
import type { CandidateForUi } from "./DiscoveryPane";
import { DiscoverySimilarityCompare } from "./DiscoverySimilarityCompare";
import { DiscoveryClusterView } from "./DiscoveryClusterView";
import { DiscoveryIsolatedContext } from "./DiscoveryIsolatedContext";
import "./DiscoveryDetailView.css";

export interface DiscoveryDetailViewProps {
  candidate: CandidateForUi | null;
  onPromote: (candidateId: string) => void;
  onDismiss: (candidateId: string) => void;
  onViewInSource?: (relPath: string, startLine: number) => void;
}

export function DiscoveryDetailView({
  candidate,
  onPromote,
  onDismiss,
  onViewInSource,
}: DiscoveryDetailViewProps) {
  const { t } = useTranslation();

  if (!candidate) {
    return (
      <div className="discovery-detail discovery-detail--empty">
        <p>{t("discovery.detail.emptyState", "Select a candidate to view details")}</p>
      </div>
    );
  }

  switch (candidate.markingReason) {
    case "high_similarity":
      return (
        <DiscoverySimilarityCompare
          candidate={candidate}
          onPromote={onPromote}
          onDismiss={onDismiss}
        />
      );
    case "cross_doc_recurrence":
      return (
        <DiscoveryClusterView
          candidate={candidate}
          onPromote={onPromote}
          onDismiss={onDismiss}
        />
      );
    case "semantic_isolated":
      return (
        <DiscoveryIsolatedContext
          candidate={candidate}
          onPromote={onPromote}
          onDismiss={onDismiss}
          onViewInSource={onViewInSource}
        />
      );
    default:
      // Generic fallback for unknown reason types
      return (
        <div className="discovery-detail">
          <header className="discovery-detail__header">
            <h3 className="discovery-detail__title">{candidate.markingReason}</h3>
          </header>
          <div className="discovery-detail__section">
            <p className="discovery-detail__excerpt-block">{candidate.excerpt}</p>
          </div>
          <div className="discovery-detail__actions">
            <button
              type="button"
              className="discovery-detail__btn discovery-detail__btn--primary"
              onClick={() => onPromote(candidate.id)}
            >
              {t("discovery.promote", "Promote")}
            </button>
            <button
              type="button"
              className="discovery-detail__btn discovery-detail__btn--ghost"
              onClick={() => onDismiss(candidate.id)}
            >
              {t("discovery.dismiss", "Dismiss")}
            </button>
          </div>
        </div>
      );
  }
}
