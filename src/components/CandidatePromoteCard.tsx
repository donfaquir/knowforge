import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

type Props = {
  candidateId: string;
  onDone: () => void;
};

export function CandidatePromoteCard({ candidateId, onDone }: Props) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<"idle" | "promoted" | "dismissed">("idle");

  const promote = async () => {
    setBusy(true);
    try {
      await invoke("promote_candidate_to_thought", { candidateId });
      setOutcome("promoted");
    } finally {
      setBusy(false);
    }
  };

  const dismiss = async () => {
    setBusy(true);
    try {
      await invoke("dismiss_latent_candidate", { candidateId });
      setOutcome("dismissed");
    } finally {
      setBusy(false);
    }
  };

  if (outcome === "promoted") {
    return (
      <div className="candidate-promote-card candidate-promote-card--done">
        <p className="candidate-promote-card__msg">{t("challengeReview.promoteSuccess")}</p>
        <button type="button" className="challenge-review-panel__linkish" onClick={onDone}>
          {t("challengeReview.continueNext")}
        </button>
      </div>
    );
  }

  if (outcome === "dismissed") {
    return (
      <div className="candidate-promote-card candidate-promote-card--done">
        <p className="candidate-promote-card__msg">{t("challengeReview.promoteDismissed")}</p>
        <button type="button" className="challenge-review-panel__linkish" onClick={onDone}>
          {t("challengeReview.continueNext")}
        </button>
      </div>
    );
  }

  return (
    <div className="candidate-promote-card">
      <p className="candidate-promote-card__prompt">{t("challengeReview.promotePrompt")}</p>
      <div className="candidate-promote-card__actions">
        <button
          type="button"
          className="challenge-review-panel__btn challenge-review-panel__btn--primary"
          disabled={busy}
          onClick={() => void promote()}
        >
          {t("challengeReview.promoteTrack")}
        </button>
        <button
          type="button"
          className="challenge-review-panel__btn"
          disabled={busy}
          onClick={() => void dismiss()}
        >
          {t("challengeReview.promoteDismiss")}
        </button>
        <button
          type="button"
          className="challenge-review-panel__linkish"
          disabled={busy}
          onClick={onDone}
        >
          {t("challengeReview.promoteSkip")}
        </button>
      </div>
    </div>
  );
}
