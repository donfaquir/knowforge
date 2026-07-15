/**
 * InlineReviewReminder — Lightweight reminder shown after a conversation ends.
 * Replaces the full inline challenge Q&A with a one-line prompt to visit Practice Mode.
 */
import { useTranslation } from "react-i18next";
import "./InlineReviewReminder.css";

export interface InlineReviewReminderProps {
  /** Excerpt of the thought that is due for review */
  thoughtExcerpt: string;
  /** How many days overdue (negative means not yet due, 0 = due today) */
  overdueDays: number;
  /** Navigate to Practice Mode */
  onGoToPractice: () => void;
  /** Dismiss this reminder */
  onDismiss: () => void;
}

export function InlineReviewReminder({
  thoughtExcerpt,
  overdueDays,
  onGoToPractice,
  onDismiss,
}: InlineReviewReminderProps) {
  const { t } = useTranslation();

  const overdueLabel =
    overdueDays > 0
      ? t("reviewReminder.overdueDays", { count: overdueDays, defaultValue: "{{count}} days overdue" })
      : t("reviewReminder.dueToday", "due today");

  const truncatedExcerpt =
    thoughtExcerpt.length > 60
      ? `${thoughtExcerpt.slice(0, 60)}…`
      : thoughtExcerpt;

  return (
    <div className="inline-review-reminder">
      <div className="inline-review-reminder__divider" role="separator" />
      <div className="inline-review-reminder__body">
        <span className="inline-review-reminder__icon" aria-hidden="true">
          💡
        </span>
        <div className="inline-review-reminder__text">
          <span className="inline-review-reminder__label">
            {t("reviewReminder.label", "A related thought is ready for review:")}
          </span>
          <span className="inline-review-reminder__excerpt">
            "{truncatedExcerpt}" ({overdueLabel})
          </span>
        </div>
        <div className="inline-review-reminder__actions">
          <button
            type="button"
            className="inline-review-reminder__btn inline-review-reminder__btn--primary"
            onClick={onGoToPractice}
          >
            {t("reviewReminder.goToPractice", "Practice →")}
          </button>
          <button
            type="button"
            className="inline-review-reminder__btn inline-review-reminder__btn--ghost"
            onClick={onDismiss}
          >
            {t("reviewReminder.dismiss", "Not now")}
          </button>
        </div>
      </div>
    </div>
  );
}
