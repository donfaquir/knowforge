/**
 * PracticeReviewCompletion — Transition screen shown after daily review is completed.
 * Displays progress stats + daily discovery picks to encourage exploration.
 */
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CandidateForUi } from "./DiscoveryPane";
import "./PracticeReviewCompletion.css";

export interface PracticeReviewCompletionProps {
  onGoToDiscovery: () => void;
  onExitPractice: () => void;
}

export function PracticeReviewCompletion({
  onGoToDiscovery,
  onExitPractice,
}: PracticeReviewCompletionProps) {
  const { t } = useTranslation();
  const [dailyPicks, setDailyPicks] = useState<CandidateForUi[]>([]);
  const [loadingPicks, setLoadingPicks] = useState(true);

  // Fetch daily picks on mount
  useEffect(() => {
    let cancelled = false;
    invoke<CandidateForUi[]>("list_discovery_daily_picks")
      .then((picks) => { if (!cancelled) setDailyPicks(picks); })
      .catch(() => { /* silently ignore */ })
      .finally(() => { if (!cancelled) setLoadingPicks(false); });
    return () => { cancelled = true; };
  }, []);

  return (
    <div className="review-completion">
      <div className="review-completion__hero">
        <span className="review-completion__check" aria-hidden="true">&#x2713;</span>
        <h2 className="review-completion__title">
          {t("practice.completion.title", "Today's review completed")}
        </h2>
      </div>

      {/* Daily picks teaser */}
      {!loadingPicks && dailyPicks.length > 0 && (
        <div className="review-completion__picks">
          <p className="review-completion__picks-label">
            {t("practice.completion.picksLabel", "Worth a look?")}
          </p>
          {dailyPicks.map((pick) => (
            <div key={pick.id} className="review-completion__pick-card">
              <span className="review-completion__pick-excerpt">
                "{pick.excerpt.length > 80 ? `${pick.excerpt.slice(0, 80)}...` : pick.excerpt}"
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="review-completion__actions">
        <button
          type="button"
          className="review-completion__btn review-completion__btn--primary"
          onClick={onExitPractice}
        >
          {t("practice.completion.exit", "Back to notes")}
        </button>
        <button
          type="button"
          className="review-completion__btn review-completion__btn--secondary"
          onClick={onGoToDiscovery}
        >
          {t("practice.completion.goDiscovery", "Explore discoveries")}
        </button>
      </div>
    </div>
  );
}
