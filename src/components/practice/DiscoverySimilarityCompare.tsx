/**
 * DiscoverySimilarityCompare — Detail view for high_similarity candidates.
 * Shows the candidate excerpt alongside its paired document path + similarity score.
 */
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { CandidateForUi } from "./DiscoveryPane";

interface Props {
  candidate: CandidateForUi;
  onPromote: (id: string) => void;
  onDismiss: (id: string) => void;
}

export function DiscoverySimilarityCompare({ candidate, onPromote, onDismiss }: Props) {
  const { t } = useTranslation();
  const [pairedContent, setPairedContent] = useState<string | null>(null);
  const [loadingPaired, setLoadingPaired] = useState(false);

  // Load paired file content for context
  useEffect(() => {
    if (!candidate.pairedRelPath) {
      setPairedContent(null);
      return;
    }
    let cancelled = false;
    setLoadingPaired(true);
    invoke<string>("read_markdown_file", { relPath: candidate.pairedRelPath })
      .then((text) => { if (!cancelled) setPairedContent(text); })
      .catch(() => { if (!cancelled) setPairedContent(null); })
      .finally(() => { if (!cancelled) setLoadingPaired(false); });
    return () => { cancelled = true; };
  }, [candidate.pairedRelPath]);

  const score = candidate.similarityScore != null
    ? (candidate.similarityScore * 100).toFixed(0) + "%"
    : "—";

  const pairedFileName = candidate.pairedRelPath?.split("/").pop()?.replace(/\.md$/i, "") ?? "";

  // Extract a snippet around the relevant area from paired content
  const pairedSnippet = pairedContent
    ? pairedContent.split("\n").slice(0, 20).join("\n")
    : null;

  return (
    <div className="discovery-detail discovery-detail--similarity">
      <header className="discovery-detail__header">
        <h3 className="discovery-detail__title">
          {t("discovery.detail.similarityTitle", "Similar Passages")}
        </h3>
        <span className="discovery-detail__badge discovery-detail__badge--similarity">
          {t("discovery.detail.similarityScore", "Similarity: {{score}}", { score })}
        </span>
      </header>

      <div className="discovery-detail__compare">
        <div className="discovery-detail__compare-col">
          <div className="discovery-detail__compare-label">
            {candidate.relPath.split("/").pop()?.replace(/\.md$/i, "") ?? candidate.relPath}
            <span className="discovery-detail__line-hint">L{candidate.startLine}–{candidate.endLine}</span>
          </div>
          <div className="discovery-detail__compare-content">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{candidate.excerpt}</ReactMarkdown>
          </div>
        </div>

        <div className="discovery-detail__compare-divider">
          <span className="discovery-detail__compare-vs">≈</span>
        </div>

        <div className="discovery-detail__compare-col">
          <div className="discovery-detail__compare-label">
            {pairedFileName || t("discovery.detail.pairedDoc", "Paired document")}
          </div>
          <div className="discovery-detail__compare-content">
            {loadingPaired ? (
              <p className="discovery-detail__loading">{t("discovery.detail.loading", "Loading...")}</p>
            ) : pairedSnippet ? (
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{pairedSnippet}</ReactMarkdown>
            ) : (
              <p className="discovery-detail__empty-text">
                {t("discovery.detail.pairedUnavailable", "Paired content unavailable")}
              </p>
            )}
          </div>
        </div>
      </div>

      <div className="discovery-detail__actions">
        <button
          type="button"
          className="discovery-detail__btn discovery-detail__btn--primary"
          onClick={() => onPromote(candidate.id)}
        >
          {t("discovery.detail.promoteOne", "Track as thought")}
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
