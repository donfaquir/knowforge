import type { TFunction } from "i18next";
import type { PassiveHighlightMarked } from "../types/passiveHighlight";
import "./PassiveHighlightSaveCue.css";

type Props = {
  t: TFunction;
  state: PassiveHighlightMarked;
  onSaveClick: () => void;
  disabled?: boolean;
};

/** 用户气泡下方：被动高亮「保存为想法」轻量提示区 */
export function PassiveHighlightSaveCue({ t, state, onSaveClick, disabled }: Props) {
  // `saved` 可选：仅在为 true 时视为已落库；undefined / false 均展示保存入口（与「尚未写入」一致）
  if (state.saved === true) {
    return (
      <div className="passive-highlight-cue" data-passive-highlight="saved">
        <span className="passive-highlight-cue__text">{t("aiPanel.passiveHighlightSavedShort")}</span>
      </div>
    );
  }
  return (
    <div className="passive-highlight-cue" data-passive-highlight="marked">
      <span className="passive-highlight-cue__text">{t("aiPanel.passiveHighlightSaveCue")}</span>
      <button
        type="button"
        className="passive-highlight-cue__btn"
        onClick={onSaveClick}
        disabled={disabled}
        aria-label={t("aiPanel.passiveHighlightSaveCueAria")}
        title={t("aiPanel.passiveHighlightSaveCueAria")}
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden={true}
        >
          <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
        </svg>
      </button>
    </div>
  );
}
