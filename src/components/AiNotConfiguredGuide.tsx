import { useTranslation } from "react-i18next";
import { dispatchOpenAiSettings } from "../utils/vaultConfigBroadcast";
import "./AiNotConfiguredGuide.css";

function IconKey() {
  return (
    <svg
      className="ai-guide__icon"
      width="36"
      height="36"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="15.5" cy="8.5" r="5.5" />
      <path d="M11.5 12.5 3 21" />
      <path d="M3 21h4v-3h3v-3" />
    </svg>
  );
}

interface Props {
  featureName: string;
  featureDescription?: string;
  compact?: boolean;
}

export default function AiNotConfiguredGuide({ featureName, featureDescription, compact }: Props) {
  const { t } = useTranslation();

  return (
    <div className={`ai-guide ${compact ? "ai-guide--compact" : ""}`}>
      <IconKey />
      <h3 className="ai-guide__title">
        {t("aiGuide.title", { feature: featureName })}
      </h3>
      {!compact && featureDescription && (
        <p className="ai-guide__desc">{featureDescription}</p>
      )}
      <button
        className="app-modal__btn app-modal__btn--primary ai-guide__btn"
        onClick={dispatchOpenAiSettings}
      >
        {t("aiGuide.configure")}
      </button>
    </div>
  );
}
