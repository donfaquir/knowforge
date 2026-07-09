import { useTranslation } from "react-i18next";
import type { CognitiveReportForUi } from "../../types/motivationFeedback";

type Props = { data: CognitiveReportForUi };

export function StatsGrid({ data }: Props) {
  const { t } = useTranslation();
  const items = [
    { label: t("cognitiveReport.newThisMonth"), value: data.newThisMonth },
    { label: t("cognitiveReport.updatedThisMonth"), value: data.updatedThisMonth },
    { label: t("cognitiveReport.totalThoughts"), value: data.totalThoughts },
    { label: t("cognitiveReport.aiRefs"), value: data.totalAiReferences },
  ];
  return (
    <dl className="cr-stats">
      {items.map((it) => (
        <div key={it.label} className="cr-stats__card">
          <dd className="cr-stats__value">{it.value}</dd>
          <dt className="cr-stats__label">{it.label}</dt>
        </div>
      ))}
    </dl>
  );
}
