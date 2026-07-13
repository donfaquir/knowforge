import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../../types/motivationFeedback";
import { StatsGrid } from "./StatsGrid";
import { MaturityOverviewCard } from "./MaturityOverviewCard";
import { MonthlyTrendChart } from "./MonthlyTrendChart";
import { ThoughtTimeline } from "./ThoughtTimeline";
import { ThoughtGrowthStoryCard } from "../ThoughtGrowthStoryCard";
import "./CognitiveReportPanel.css";

const DISABLE_KEY = "knowforge:disableCognitiveReport";

type Props = { open: boolean; onClose: () => void };

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
  const [growthStoryThoughtId, setGrowthStoryThoughtId] = useState<string | null>(null);

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
    if (open && !disabled) void load();
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
    } catch { /* ignore */ }
  }, []);

  const handleExportGrowthStory = useCallback((thoughtId: string) => {
    setGrowthStoryThoughtId(thoughtId);
  }, []);

  if (!open) return null;

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
            <p className="cr-muted">{t("cognitiveReport.disabledHint")}</p>
          ) : loading ? (
            <p className="cr-muted">{t("cognitiveReport.loading")}</p>
          ) : error ? (
            <p role="alert">{error}</p>
          ) : data ? (
            <>
              <p className="cr-muted">
                {t("cognitiveReport.scanMeta", { files: data.scannedFiles, thoughts: data.totalThoughts })}
              </p>
              <StatsGrid data={data} />
              <MaturityOverviewCard data={data} />
              <MonthlyTrendChart snapshots={data.monthlySnapshots} />
              <ThoughtTimeline timelines={data.timelines} onExportGrowthStory={handleExportGrowthStory} />
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
          {!disabled && (
            <button type="button" className="cognitive-report__close" onClick={() => void load()}>
              {t("cognitiveReport.refresh")}
            </button>
          )}
        </footer>
      </div>
      <ThoughtGrowthStoryCard
        thoughtId={growthStoryThoughtId ?? ""}
        open={growthStoryThoughtId !== null}
        onClose={() => setGrowthStoryThoughtId(null)}
      />
    </div>
  );
}
