import { useState } from "react";
import { useTranslation } from "react-i18next";
import "./PracticeMode.css";

export type PracticeSubTab = "review" | "discovery";

export interface PracticeModeProps {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  workspaceRoot: string | null;
}

export function PracticeMode(_props: PracticeModeProps) {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState<PracticeSubTab>("review");

  return (
    <div className="practice-mode">
      <header className="practice-mode__header">
        <div className="practice-mode__tabs" role="tablist">
          <button
            type="button"
            role="tab"
            className={`practice-mode__tab${subTab === "review" ? " practice-mode__tab--active" : ""}`}
            aria-selected={subTab === "review"}
            onClick={() => setSubTab("review")}
          >
            {t("practice.tabReview", "Review")}
          </button>
          <button
            type="button"
            role="tab"
            className={`practice-mode__tab${subTab === "discovery" ? " practice-mode__tab--active" : ""}`}
            aria-selected={subTab === "discovery"}
            onClick={() => setSubTab("discovery")}
          >
            {t("practice.tabDiscovery", "Discovery")}
          </button>
        </div>
      </header>

      <div className="practice-mode__body">
        {subTab === "review" && (
          <div className="practice-mode__placeholder">
            <p>{t("practice.reviewPlaceholder", "Review pane — coming soon")}</p>
          </div>
        )}
        {subTab === "discovery" && (
          <div className="practice-mode__placeholder">
            <p>{t("practice.discoveryPlaceholder", "Discovery pane — coming soon")}</p>
          </div>
        )}
      </div>
    </div>
  );
}
