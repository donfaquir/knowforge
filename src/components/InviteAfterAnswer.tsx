/**
 * 邀请区组件：主回答完成后展示，引导用户深入探讨。
 * 浅档永不展示；中档轻量邀请；深档完整邀请。
 */

import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  ThoughtRetrievalResult,
  ThoughtMaturity,
  DepthMode,
} from "../types/cognitiveTypes";
import "./InviteAfterAnswer.css";

type Props = {
  depthMode: DepthMode;
  thought: ThoughtRetrievalResult | null;
  question: string;
  onAccept: (question: string, thought: ThoughtRetrievalResult | null) => void;
  onDismiss: () => void;
  onSnooze?: (days: number) => void;
  disabled?: boolean;
};

const MATURITY_ICON: Record<ThoughtMaturity, string> = {
  seedling: "\u{1F331}",
  growing: "\u{1F33F}",
  mature: "\u{1F333}",
};

const SNOOZE_OPTIONS = [3, 7, 14] as const;

export function InviteAfterAnswer({
  depthMode,
  thought,
  question,
  onAccept,
  onDismiss,
  onSnooze,
  disabled,
}: Props) {
  const { t } = useTranslation();
  const [snoozeConfirm, setSnoozeConfirm] = useState(false);

  const handleAccept = useCallback(() => {
    onAccept(question, thought);
  }, [onAccept, question, thought]);

  const handleSnooze = useCallback(
    (days: number) => {
      onSnooze?.(days);
      setSnoozeConfirm(false);
    },
    [onSnooze],
  );

  // 浅档永不展示
  if (depthMode === "shallow") return null;

  const hasThought = thought && !thought.privateOmitted && thought.excerpt;
  const isDeep = depthMode === "deep";

  return (
    <div className="invite-after-answer">
      {isDeep && hasThought && (
        <div className="invite-thought-ref">
          <span className="invite-thought-icon">
            {MATURITY_ICON[thought.maturity] ?? "\u{1F331}"}
          </span>
          <span className="invite-thought-path">{thought.relPath}</span>
          <span className="invite-thought-excerpt">
            {thought.excerpt.length > 80
              ? thought.excerpt.slice(0, 80) + "..."
              : thought.excerpt}
          </span>
        </div>
      )}

      <div className="invite-question">{question}</div>

      <div className="invite-actions">
        <button
          className="invite-btn invite-btn--accept"
          onClick={handleAccept}
          disabled={disabled}
        >
          {hasThought ? t("invite.explore") : t("invite.chat")}
        </button>
        <button
          className="invite-btn invite-btn--dismiss"
          onClick={onDismiss}
          disabled={disabled}
        >
          {t("invite.enough")}
        </button>
        {onSnooze && !snoozeConfirm && (
          <button
            className="invite-btn invite-btn--snooze"
            onClick={() => setSnoozeConfirm(true)}
            disabled={disabled}
          >
            {t("invite.snoozeBtn")}
          </button>
        )}
      </div>

      {snoozeConfirm && (
        <div className="invite-snooze-confirm">
          <span className="invite-snooze-label">{t("invite.snoozeConfirm")}</span>
          <div className="invite-snooze-options">
            {SNOOZE_OPTIONS.map((d) => (
              <button
                key={d}
                className="invite-btn invite-btn--snooze-option"
                onClick={() => handleSnooze(d)}
                disabled={disabled}
              >
                {t("invite.snoozeDays", { count: d })}
              </button>
            ))}
            <button
              className="invite-btn invite-btn--snooze-cancel"
              onClick={() => setSnoozeConfirm(false)}
              disabled={disabled}
            >
              {t("invite.snoozeCancel")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
