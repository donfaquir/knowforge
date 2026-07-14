import { invoke } from "@tauri-apps/api/core";
import { useState } from "react";
import { useTranslation } from "react-i18next";

type Props = {
  thoughtId?: string;
  questionText: string;
  questionTemplate?: string;
};

type Phase = "idle" | "reason" | "done";

const REASONS = ["too_easy", "irrelevant", "too_vague", "duplicate"] as const;

export function ChallengeFeedbackBar({ thoughtId, questionText, questionTemplate }: Props) {
  const { t } = useTranslation();
  const [phase, setPhase] = useState<Phase>("idle");
  const [submitting, setSubmitting] = useState(false);

  const submit = async (rating: "helpful" | "not_helpful", reason?: string) => {
    setSubmitting(true);
    try {
      await invoke("submit_challenge_feedback", {
        thoughtId: thoughtId ?? null,
        questionText,
        questionTemplate: questionTemplate ?? null,
        rating,
        ratingReason: reason ?? null,
      });
    } catch {
      // best-effort
    }
    setSubmitting(false);
    setPhase("done");
  };

  if (phase === "done") {
    return (
      <div className="challenge-feedback-bar challenge-feedback-bar--done">
        <span className="challenge-feedback-bar__thanks">{t("challengeReview.feedbackThanks")}</span>
      </div>
    );
  }

  return (
    <div className="challenge-feedback-bar">
      {phase === "idle" ? (
        <div className="challenge-feedback-bar__row">
          <span className="challenge-feedback-bar__prompt">{t("challengeReview.feedbackPrompt")}</span>
          <button
            type="button"
            className="challenge-feedback-bar__btn challenge-feedback-bar__btn--helpful"
            disabled={submitting}
            onClick={() => void submit("helpful")}
          >
            {t("challengeReview.helpful")}
          </button>
          <button
            type="button"
            className="challenge-feedback-bar__btn challenge-feedback-bar__btn--not-helpful"
            disabled={submitting}
            onClick={() => setPhase("reason")}
          >
            {t("challengeReview.notHelpful")}
          </button>
        </div>
      ) : (
        <div className="challenge-feedback-bar__reasons">
          <span className="challenge-feedback-bar__prompt">{t("challengeReview.feedbackReasonHint")}</span>
          <div className="challenge-feedback-bar__tags">
            {REASONS.map((r) => (
              <button
                key={r}
                type="button"
                className="challenge-feedback-bar__tag"
                disabled={submitting}
                onClick={() => void submit("not_helpful", r)}
              >
                {t(`challengeReview.reason_${r}`)}
              </button>
            ))}
            <button
              type="button"
              className="challenge-feedback-bar__tag challenge-feedback-bar__tag--skip"
              disabled={submitting}
              onClick={() => void submit("not_helpful")}
            >
              {t("challengeReview.feedbackSkipReason")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
