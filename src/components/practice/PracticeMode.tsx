import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ReviewQueueItem } from "../../types/cognitiveTypes";
import { PracticeReviewPane } from "./PracticeReviewPane";
import "./PracticeMode.css";

export type PracticeSubTab = "review" | "discovery";

export interface PracticeModeProps {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  workspaceRoot: string | null;
}

export function PracticeMode({ workspaceReady, tauriRuntime }: PracticeModeProps) {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState<PracticeSubTab>("review");
  const [reviewCompleted, setReviewCompleted] = useState(false);
  const [_focusedThought, setFocusedThought] = useState<ReviewQueueItem | null>(null);

  const handleReviewCompleted = useCallback(() => {
    setReviewCompleted(true);
  }, []);

  return (
    <div className="practice-mode">
      <header className="practice-mode__header">
        <div className="practice-mode__tabs" role="tablist">
          <button
            type="button"
            role="tab"
            className={`practice-mode__tab${subTab === "review" ? " practice-mode__tab--active" : ""}`}
            aria-selected={subTab === "review"}
            onClick={() => { setSubTab("review"); setReviewCompleted(false); }}
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
        {subTab === "review" && !reviewCompleted && (
          <PracticeReviewPane
            workspaceReady={workspaceReady}
            tauriRuntime={tauriRuntime}
            onReviewCompleted={handleReviewCompleted}
            onFocusThought={setFocusedThought}
          />
        )}
        {subTab === "review" && reviewCompleted && (
          <div className="practice-mode__placeholder">
            <p>{t("practice.reviewCompleted", "Today's review completed!")}</p>
            <button
              type="button"
              className="practice-mode__tab"
              onClick={() => setSubTab("discovery")}
            >
              {t("practice.goToDiscovery", "Explore discoveries")}
            </button>
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
