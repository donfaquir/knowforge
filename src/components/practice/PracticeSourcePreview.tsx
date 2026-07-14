/**
 * Practice Mode — right column: read-only source note preview with paragraph highlight.
 */
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import "./PracticeSourcePreview.css";

export interface PracticeSourcePreviewProps {
  /** Relative path of the file containing the thought */
  relPath: string | null;
  /** 1-based start line of the thought in the source file */
  startLine?: number;
  /** 1-based end line of the thought in the source file */
  endLine?: number;
}

export function PracticeSourcePreview({ relPath, startLine, endLine }: PracticeSourcePreviewProps) {
  const { t } = useTranslation();
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const highlightRef = useRef<HTMLDivElement>(null);

  // Load file content
  useEffect(() => {
    if (!relPath) {
      setContent(null);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<string>("read_markdown_file", { relPath })
      .then((text) => { if (!cancelled) setContent(text); })
      .catch((e) => { if (!cancelled) { setContent(null); setError(String(e)); } })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [relPath]);

  // Scroll to highlighted section after render
  useEffect(() => {
    if (highlightRef.current) {
      highlightRef.current.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [content, startLine]);

  // --- Empty / loading / error states ---
  if (!relPath) {
    return (
      <div className="practice-source-preview practice-source-preview--empty">
        <p>{t("practice.sourcePreviewEmpty", "Select a thought to view its source note")}</p>
      </div>
    );
  }

  if (loading) {
    return (
      <div className="practice-source-preview practice-source-preview--loading">
        <p>{t("practice.sourcePreviewLoading", "Loading...")}</p>
      </div>
    );
  }

  if (error || content == null) {
    return (
      <div className="practice-source-preview practice-source-preview--error">
        <p>{t("practice.sourcePreviewError", "Failed to load source note")}</p>
        {error && <p className="practice-source-preview__error-detail">{error}</p>}
      </div>
    );
  }

  // --- Render with highlight ---
  const lines = content.split("\n");
  const hasHighlight = startLine != null && startLine >= 1;
  const hlStart = (startLine ?? 1) - 1; // 0-based
  const hlEnd = endLine ?? (startLine ?? 1); // 1-based inclusive → exclusive index = hlEnd

  if (!hasHighlight) {
    // No highlight info — just render the full note
    return (
      <div className="practice-source-preview">
        <div className="practice-source-preview__path" title={relPath}>{relPath}</div>
        <div className="practice-source-preview__content">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
        </div>
      </div>
    );
  }

  const beforeText = lines.slice(0, hlStart).join("\n");
  const highlightText = lines.slice(hlStart, hlEnd).join("\n");
  const afterText = lines.slice(hlEnd).join("\n");

  return (
    <div className="practice-source-preview">
      <div className="practice-source-preview__path" title={relPath}>{relPath}</div>
      <div className="practice-source-preview__content">
        {beforeText && (
          <div className="practice-source-preview__section">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{beforeText}</ReactMarkdown>
          </div>
        )}
        <div className="practice-source-preview__highlight" ref={highlightRef}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{highlightText}</ReactMarkdown>
        </div>
        {afterText && (
          <div className="practice-source-preview__section">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{afterText}</ReactMarkdown>
          </div>
        )}
      </div>
    </div>
  );
}
