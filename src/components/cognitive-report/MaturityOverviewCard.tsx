import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../../types/motivationFeedback";

type Props = { data: CognitiveReportForUi };

function delta(cur: number, prev: number): string {
  const diff = cur - prev;
  if (diff > 0) return `+${diff}`;
  if (diff < 0) return `${diff}`;
  return "0";
}

export function MaturityOverviewCard({ data }: Props) {
  const { t } = useTranslation();
  const { seedling, growing, mature } = data.maturity;
  const total = seedling + growing + mature;

  const pct = (n: number) => (total > 0 ? (n / total) * 100 : 0);

  return (
    <section className="cr-maturity">
      <h3 className="cr-section-title">{t("cognitiveReport.maturityDist")}</h3>
      {total === 0 ? (
        <p className="cr-muted">{t("cognitiveReport.noData")}</p>
      ) : (
        <>
          <div className="cr-maturity__bar">
            {seedling > 0 && (
              <div
                className="cr-maturity__seg cr-maturity__seg--seedling"
                style={{ width: `${pct(seedling)}%` }}
                title={`🌱 ${seedling}`}
              >
                {pct(seedling) > 12 && <span>{seedling}</span>}
              </div>
            )}
            {growing > 0 && (
              <div
                className="cr-maturity__seg cr-maturity__seg--growing"
                style={{ width: `${pct(growing)}%` }}
                title={`🌿 ${growing}`}
              >
                {pct(growing) > 12 && <span>{growing}</span>}
              </div>
            )}
            {mature > 0 && (
              <div
                className="cr-maturity__seg cr-maturity__seg--mature"
                style={{ width: `${pct(mature)}%` }}
                title={`🌳 ${mature}`}
              >
                {pct(mature) > 12 && <span>{mature}</span>}
              </div>
            )}
          </div>

          <div className="cr-maturity__legend">
            <span className="cr-maturity__legend-item">
              <span className="cr-maturity__dot cr-maturity__dot--seedling" /> 🌱 {seedling}
            </span>
            <span className="cr-maturity__legend-item">
              <span className="cr-maturity__dot cr-maturity__dot--growing" /> 🌿 {growing}
            </span>
            <span className="cr-maturity__legend-item">
              <span className="cr-maturity__dot cr-maturity__dot--mature" /> 🌳 {mature}
            </span>
          </div>

          {data.prevMonthMaturity && (
            <p className="cr-muted cr-maturity__delta">
              {t("cognitiveReport.vsLastMonth")}:
              {" 🌱 "}{delta(seedling, data.prevMonthMaturity.seedling)}
              {" · 🌿 "}{delta(growing, data.prevMonthMaturity.growing)}
              {" · 🌳 "}{delta(mature, data.prevMonthMaturity.mature)}
            </p>
          )}
        </>
      )}
    </section>
  );
}
