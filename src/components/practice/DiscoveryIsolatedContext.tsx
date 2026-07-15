/**
 * DiscoveryIsolatedContext — Detail view for semantic_isolated candidates.
 * Shows the candidate with its surrounding context from the source note.
 */
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { CandidateForUi } from "./DiscoveryPane";

interface Props {
  candidate: CandidateForUi;
  onPromote: (id: string) => void;
  onDismiss: (id: string) => void;
  onViewInSource?: (relPath: string, startLine: number) => void;
}

/** How many lines of context to show before/after the isolated paragraph */
const CONTEXT_LINES = 5;

export function DiscoveryIsolatedContext({ candidate, onPromote, onDismiss, onViewInSource }: Props) {
  const { t } = useTranslation();
  const [fileContent, setFileContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const highlightRef = useRef<HTMLDivElement>(null);

  // Load source file
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<string>("read_markdown_file", { relPath: candidate.relPath })
      .then((text) => { if (!cancelled) setFileContent(text); })
      .catch(() => { if (!cancelled) setFileContent(null); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [candidate.relPath]);

  // Scroll to highlighted section
  useEffect(() => {
    if (highlightRef.current) {
      highlightRef.current.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [fileContent, candidate.startLine]);

  const fileName = candidate.relPath.split("/").pop()?.replace(/\.md$/i, "") ?? candidate.relPath;

  // Extract context around the candidate paragraph
  const renderContext = () => {
    if (loading) {
      return <p className="discovery-detail__loading">{t("discovery.detail.loading", "Loading...")}</p>;
    }
    if (!fileContent) {
      return <p className="discovery-detail__empty-text">{t("discovery.detail.contextUnavailable", "Context unavailable")}</p>;
    }

    const lines = fileContent.split("\n");
    const hlStart = Math.max(0, candidate.startLine - 1); // 0-based
    const hlEnd = candidate.endLine; // exclusive
    const ctxStart = Math.max(0, hlStart - CONTEXT_LINES);
    const ctxEnd = Math.min(lines.length, hlEnd + CONTEXT_LINES);

    const beforeText = lines.slice(ctxStart, hlStart).join("\n");
    const highlightText = lines.slice(hlStart, hlEnd).join("\n");
    const afterText = lines.slice(hlEnd, ctxEnd).join("\n");

    return (
      <div className="discovery-detail__context-block">
        {beforeText && (
          <div className="discovery-detail__context-dim">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{beforeText}</ReactMarkdown>
          </div>
        )}
        <div className="discovery-detail__context-highlight" ref={highlightRef}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{highlightText}</ReactMarkdown>
        </div>
        {afterText && (
          <div className="discovery-detail__context-dim">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{afterText}</ReactMarkdown>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="discovery-detail discovery-detail--isolated">
      <header className="discovery-detail__header">
        <h3 className="discovery-detail__title">
          {t("discovery.detail.isolatedTitle", "Isolated Paragraph")}
        </h3>
        <span className="discovery-detail__badge discovery-detail__badge--isolated">
          {t("discovery.detail.isolatedInfo", "Never linked to other notes")}
        </span>
      </header>

      <div className="discovery-detail__section">
        <div className="discovery-detail__section-label">
          {fileName}
          <span className="discovery-detail__line-hint"> L{candidate.startLine}–{candidate.endLine}</span>
        </div>
        {renderContext()}
      </div>

      <div className="discovery-detail__actions">
        <button
          type="button"
          className="discovery-detail__btn discovery-detail__btn--primary"
          onClick={() => onPromote(candidate.id)}
        >
          {t("discovery.detail.startTracking", "Start tracking")}
        </button>
        {onViewInSource && (
          <button
            type="button"
            className="discovery-detail__btn discovery-detail__btn--secondary"
            onClick={() => onViewInSource(candidate.relPath, candidate.startLine)}
          >
            {t("discovery.detail.viewInSource", "View in source ↗")}
          </button>
        )}
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
