/**
 * 通道二：对话末尾内联挑战式回顾（单次提交 + 非流式点评，子轮不进主 messages）。
 */
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { getAppLocale } from "../i18n";
import { invoke } from "@tauri-apps/api/core";
import type {
  DepthMode,
  EvaluateChallengeAnswerResponse,
  ThoughtRetrievalResult,
} from "../types/cognitiveTypes";
import { trackKnowforgeEvent } from "../utils/knowforgeAnalytics";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import "./ChallengeReviewInline.css";

type Props = {
  depthMode: DepthMode;
  thought: ThoughtRetrievalResult;
  question: string;
  templateKind: string;
  onDismiss: () => void;
};

export function ChallengeReviewInline({
  depthMode,
  thought,
  question,
  templateKind,
  onDismiss,
}: Props) {
  const { t } = useTranslation();
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState<"input" | "done">("input");
  const [result, setResult] = useState<EvaluateChallengeAnswerResponse | null>(null);

  const handleSubmit = useCallback(async () => {
    const trimmed = answer.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    void trackKnowforgeEvent("review.inline_submit", {
      templateKind,
      thoughtId: thought.thoughtId,
    });
    try {
      const ev = await invoke<EvaluateChallengeAnswerResponse>("evaluate_challenge_answer", {
        args: {
          question,
          userAnswer: trimmed,
          thoughtExcerpt: thought.excerpt,
          depthMode,
          uiLocale: getAppLocale(),
        },
      });
      setResult(ev);
      setPhase("done");
      void trackKnowforgeEvent("review.inline_evaluated", {
        passed: ev.passed,
        sloppy: ev.sloppy,
        thoughtId: thought.thoughtId,
      });
      await invoke("apply_challenge_pass_to_thought", {
        args: {
          relPath: thought.relPath,
          thoughtId: thought.thoughtId,
          passed: ev.passed && !ev.sloppy,
          sloppy: ev.sloppy,
        },
      });
      if (ev.passed && !ev.sloppy) {
        void trackKnowforgeEvent("review.inline_pass_applied", { thoughtId: thought.thoughtId });
      }
    } catch {
      setResult({
        passed: false,
        sloppy: false,
        commentaryMd: t("challengeReview.evaluateError"),
      });
      setPhase("done");
    } finally {
      setBusy(false);
    }
  }, [answer, busy, depthMode, question, t, templateKind, thought.excerpt, thought.relPath, thought.thoughtId]);

  if (depthMode === "shallow") return null;

  return (
    <div className="challenge-review-inline">
      <div className="challenge-review-inline__divider" role="separator" />
      <div className="challenge-review-inline__label">{t("challengeReview.title")}</div>
      {thought.excerpt && !thought.privateOmitted ? (
        <div className="challenge-review-inline__excerpt">{thought.excerpt}</div>
      ) : null}
      <div className="challenge-review-inline__question">{question}</div>

      {phase === "input" ? (
        <>
          <textarea
            className="challenge-review-inline__textarea"
            value={answer}
            onChange={(e) => setAnswer(e.target.value)}
            rows={3}
            disabled={busy}
            placeholder={t("challengeReview.answerPlaceholder")}
          />
          <div className="challenge-review-inline__actions">
            <button
              type="button"
              className="challenge-review-inline__btn challenge-review-inline__btn--primary"
              disabled={busy || !answer.trim()}
              onClick={() => void handleSubmit()}
            >
              {t("challengeReview.share")}
            </button>
            <button
              type="button"
              className="challenge-review-inline__btn challenge-review-inline__btn--ghost"
              disabled={busy}
              onClick={onDismiss}
            >
              {t("challengeReview.skip")}
            </button>
          </div>
        </>
      ) : (
        <div className="challenge-review-inline__result">
          {result?.sloppy ? (
            <p className="challenge-review-inline__sloppy">{t("challengeReview.sloppyHint")}</p>
          ) : null}
          {result?.passed ? (
            <p className="challenge-review-inline__pass">{t("challengeReview.passed")}</p>
          ) : null}
          <AiAssistantMarkdown
            className="challenge-review-inline__commentary"
            content={result?.commentaryMd ?? ""}
          />
          <button type="button" className="challenge-review-inline__btn challenge-review-inline__btn--ghost" onClick={onDismiss}>
            {t("challengeReview.close")}
          </button>
        </div>
      )}
    </div>
  );
}
