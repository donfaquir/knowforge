/**
 * 认知成长报告：展示 `generate_cognitive_report` 返回的统计事实（无评判语气）。
 */

import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../types/motivationFeedback";
import "./CognitiveReportPanel.css";

const DISABLE_KEY = "knowforge:disableCognitiveReport";

type Props = {
  open: boolean;
  onClose: () => void;
};

export function CognitiveReportPanel({ open, onClose }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<CognitiveReportForUi | null>(null);
  const [disabled, setDisabled] = useState(() => {
    try {
      return localStorage.getItem(DISABLE_KEY) === "1";
    } catch {
      return false;
    }
  });

  const load = useCallback(async () => {
    if (!isTauri()) {
      setError("Not available.");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<CognitiveReportForUi>("generate_cognitive_report");
      setData(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open && !disabled) {
      void load();
    }
  }, [open, disabled, load]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const toggleDisabled = useCallback((next: boolean) => {
    setDisabled(next);
    try {
      if (next) localStorage.setItem(DISABLE_KEY, "1");
      else localStorage.removeItem(DISABLE_KEY);
    } catch {
      /* ignore */
    }
  }, []);

  if (!open) {
    return null;
  }

  return (
    <div className="cognitive-report-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="cognitive-report" role="dialog" aria-labelledby="cognitive-report-title">
        <header className="cognitive-report__header">
          <h2 id="cognitive-report-title" className="cognitive-report__title">
            {t("cognitiveReport.title")}
          </h2>
          <button type="button" className="cognitive-report__close" onClick={onClose} aria-label={t("cognitiveReport.close")}>
            ×
          </button>
        </header>
        <div className="cognitive-report__body">
          {disabled ? (
            <p className="cognitive-report__muted">{t("cognitiveReport.disabledHint")}</p>
          ) : loading ? (
            <p className="cognitive-report__muted">{t("cognitiveReport.loading")}</p>
          ) : error ? (
            <p role="alert">{error}</p>
          ) : data ? (
            <>
              <p className="cognitive-report__muted">
                {t("cognitiveReport.scanMeta", { files: data.scannedFiles, thoughts: data.totalThoughts })}
              </p>
              <dl className="cognitive-report__grid">
                <div className="cognitive-report__stat">
                  <dt>{t("cognitiveReport.newThisMonth")}</dt>
                  <dd>{data.newThisMonth}</dd>
                </div>
                <div className="cognitive-report__stat">
                  <dt>{t("cognitiveReport.updatedThisMonth")}</dt>
                  <dd>{data.updatedThisMonth}</dd>
                </div>
                <div className="cognitive-report__stat">
                  <dt>{t("cognitiveReport.totalThoughts")}</dt>
                  <dd>{data.totalThoughts}</dd>
                </div>
                <div className="cognitive-report__stat">
                  <dt>{t("cognitiveReport.aiRefs")}</dt>
                  <dd>{data.totalAiReferences}</dd>
                </div>
              </dl>
              <h3 className="cognitive-report__section-title">{t("cognitiveReport.maturityDist")}</h3>
              <p>
                {t("cognitiveReport.maturityLine", {
                  s: data.maturity.seedling,
                  g: data.maturity.growing,
                  m: data.maturity.mature,
                })}
              </p>
              {data.prevMonthMaturity ? (
                <p className="cognitive-report__muted">
                  {t("cognitiveReport.prevMonthLine", {
                    s: data.prevMonthMaturity.seedling,
                    g: data.prevMonthMaturity.growing,
                    m: data.prevMonthMaturity.mature,
                  })}
                </p>
              ) : (
                <p className="cognitive-report__muted">{t("cognitiveReport.noPrevMonth")}</p>
              )}
              <h3 className="cognitive-report__section-title">{t("cognitiveReport.timelines")}</h3>
              {data.timelines.length === 0 ? (
                <p className="cognitive-report__muted">{t("cognitiveReport.noTimelines")}</p>
              ) : (
                data.timelines.map((row) => (
                  <section key={`${row.relPath}-${row.thoughtId}`} style={{ marginBottom: 14 }}>
                    <p style={{ margin: "0 0 6px" }}>
                      <strong>{row.relPath}</strong> — <code>{row.thoughtId}</code>
                    </p>
                    <p className="cognitive-report__muted" style={{ margin: "0 0 6px" }}>
                      {row.excerpt}
                    </p>
                    <ul className="cognitive-report__timeline">
                      {row.history.map((h, i) => (
                        <li key={`${h.date}-${i}`}>
                          {h.date} · {h.type} · {h.source}
                          {h.diffSummary ? ` — ${h.diffSummary}` : ""}
                        </li>
                      ))}
                    </ul>
                  </section>
                ))
              )}
            </>
          ) : null}
        </div>
        <footer className="cognitive-report__footer">
          <label>
            <input
              type="checkbox"
              checked={disabled}
              onChange={(e) => toggleDisabled(e.target.checked)}
            />
            {t("cognitiveReport.disableCheckbox")}
          </label>
          {!disabled ? (
            <button type="button" className="cognitive-report__close" onClick={() => void load()}>
              {t("cognitiveReport.refresh")}
            </button>
          ) : null}
        </footer>
      </div>
    </div>
  );
}
