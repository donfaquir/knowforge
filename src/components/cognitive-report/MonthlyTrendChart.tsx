import { useTranslation } from "react-i18next";
import type { MonthlySnapshot } from "../../types/motivationFeedback";

type Props = { snapshots: MonthlySnapshot[] };

function monthLabel(ym: string): string {
  const parts = ym.split("-");
  return parts.length === 2 ? `${parseInt(parts[1], 10)}月` : ym;
}

export function MonthlyTrendChart({ snapshots }: Props) {
  const { t } = useTranslation();

  if (snapshots.length === 0) {
    return null;
  }

  const totals = snapshots.map((s) => s.seedling + s.growing + s.mature);
  const maxVal = Math.max(...totals, 1);

  return (
    <section className="cr-trend">
      <h3 className="cr-section-title">{t("cognitiveReport.monthlyTrend")}</h3>
      <div className="cr-trend__chart">
        {snapshots.map((s) => {
          const total = s.seedling + s.growing + s.mature;
          const hPct = (total / maxVal) * 100;
          return (
            <div key={s.yearMonth} className="cr-trend__col">
              <div className="cr-trend__bar-wrap">
                <div className="cr-trend__bar" style={{ height: `${hPct}%` }}>
                  {s.mature > 0 && (
                    <div
                      className="cr-trend__seg cr-trend__seg--mature"
                      style={{ flex: s.mature }}
                    />
                  )}
                  {s.growing > 0 && (
                    <div
                      className="cr-trend__seg cr-trend__seg--growing"
                      style={{ flex: s.growing }}
                    />
                  )}
                  {s.seedling > 0 && (
                    <div
                      className="cr-trend__seg cr-trend__seg--seedling"
                      style={{ flex: s.seedling }}
                    />
                  )}
                </div>
                {total > 0 && <span className="cr-trend__count">{total}</span>}
              </div>
              <span className="cr-trend__label">{monthLabel(s.yearMonth)}</span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
