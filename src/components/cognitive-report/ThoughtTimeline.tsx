import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../../types/motivationFeedback";

type Props = {
  timelines: CognitiveReportForUi["timelines"];
  onExportGrowthStory?: (thoughtId: string) => void;
};

export function ThoughtTimeline({ timelines, onExportGrowthStory }: Props) {
  const { t } = useTranslation();

  if (timelines.length === 0) {
    return (
      <section className="cr-timeline">
        <h3 className="cr-section-title">{t("cognitiveReport.topTimelines")}</h3>
        <p className="cr-muted">{t("cognitiveReport.noData")}</p>
      </section>
    );
  }

  return (
    <section className="cr-timeline">
      <h3 className="cr-section-title">{t("cognitiveReport.topTimelines")}</h3>
      <div className="cr-timeline__list">
        {timelines.map((tl) => (
          <div key={`${tl.relPath}:${tl.thoughtId}`} className="cr-timeline__item">
            <div className="cr-timeline__dot" />
            <div className="cr-timeline__content">
              <p className="cr-timeline__excerpt">{tl.excerpt || tl.thoughtId}</p>
              <span className="cr-timeline__meta">
                {tl.relPath} · {tl.history.length}{" "}
                {t("cognitiveReport.entries")}
              </span>
              <ul className="cr-timeline__history">
                {tl.history.slice(0, 5).map((h, i) => (
                  <li key={i} className="cr-timeline__entry">
                    <span className="cr-timeline__date">{h.date.slice(0, 10)}</span>
                    <span className="cr-timeline__type">{h.type}</span>
                    {h.diffSummary && (
                      <span className="cr-timeline__diff">{h.diffSummary}</span>
                    )}
                  </li>
                ))}
              </ul>
              {onExportGrowthStory && (
                <button
                  className="cr-timeline__export-btn"
                  onClick={() => onExportGrowthStory(tl.thoughtId)}
                  title={t("growthStory.viewGrowthStory", "成长故事")}
                >
                  {t("growthStory.viewGrowthStory", "成长故事")}
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
