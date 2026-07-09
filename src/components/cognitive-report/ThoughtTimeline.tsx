import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../../types/motivationFeedback";

type Props = { timelines: CognitiveReportForUi["timelines"] };

export function ThoughtTimeline({ timelines }: Props) {
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
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
