/**
 * DiscoveryClusterView — Detail view for cross_doc_recurrence candidates.
 * Shows the candidate with its cluster context (recurrence across documents).
 */
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { CandidateForUi } from "./DiscoveryPane";

interface Props {
  candidate: CandidateForUi;
  onPromote: (id: string) => void;
  onDismiss: (id: string) => void;
}

export function DiscoveryClusterView({ candidate, onPromote, onDismiss }: Props) {
  const { t } = useTranslation();

  const fileName = candidate.relPath.split("/").pop()?.replace(/\.md$/i, "") ?? candidate.relPath;

  return (
    <div className="discovery-detail discovery-detail--cluster">
      <header className="discovery-detail__header">
        <h3 className="discovery-detail__title">
          {t("discovery.detail.clusterTitle", "Recurring Theme")}
        </h3>
        <span className="discovery-detail__badge discovery-detail__badge--cluster">
          {t("discovery.detail.clusterInfo", "Cross-document recurrence")}
        </span>
      </header>

      <div className="discovery-detail__section">
        <div className="discovery-detail__section-label">
          {t("discovery.detail.sourceFile", "Source")}:
          <span className="discovery-detail__file-name" title={candidate.relPath}> {fileName}</span>
          <span className="discovery-detail__line-hint"> L{candidate.startLine}–{candidate.endLine}</span>
        </div>
        <div className="discovery-detail__excerpt-block">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{candidate.excerpt}</ReactMarkdown>
        </div>
      </div>

      {candidate.pairedRelPath && (
        <div className="discovery-detail__section">
          <div className="discovery-detail__section-label">
            {t("discovery.detail.alsoFoundIn", "Also found in")}:
            <span className="discovery-detail__file-name" title={candidate.pairedRelPath}>
              {" "}{candidate.pairedRelPath.split("/").pop()?.replace(/\.md$/i, "")}
            </span>
          </div>
        </div>
      )}

      <p className="discovery-detail__hint">
        {t(
          "discovery.detail.clusterHint",
          "This idea recurs across multiple notes. Promoting it as a thought lets you track and deepen it.",
        )}
      </p>

      <div className="discovery-detail__actions">
        <button
          type="button"
          className="discovery-detail__btn discovery-detail__btn--primary"
          onClick={() => onPromote(candidate.id)}
        >
          {t("discovery.detail.createThought", "Create thought")}
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
