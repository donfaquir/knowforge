import { useTranslation } from "react-i18next";
import type { WritingCoachLinkItem } from "../types/writingCoach";
import "./WritingCoachBubble.css";

type Props = {
  bubbleVisible: boolean;
  bubbleFading: boolean;
  anchorTopPx: number;
  panelOpen: boolean;
  panelLoading: boolean;
  panelError: string | null;
  reasoningQuestions: string[];
  links: WritingCoachLinkItem[];
  knowledgeModuleSkipped: boolean;
  onBubbleClick: () => void;
  onCollapsePanel: () => void;
  onHelpful: () => void;
  onOpenLink: (relPath: string) => void;
};

export function WritingCoachBubble({
  bubbleVisible,
  bubbleFading,
  anchorTopPx,
  panelOpen,
  panelLoading,
  panelError,
  reasoningQuestions,
  links,
  knowledgeModuleSkipped,
  onBubbleClick,
  onCollapsePanel,
  onHelpful,
  onOpenLink,
}: Props) {
  const { t } = useTranslation();

  return (
    <>
      {bubbleVisible && !panelOpen ? (
        <button
          type="button"
          className={`writing-coach-bubble${bubbleFading ? " writing-coach-bubble--fading" : ""}`}
          style={{ top: anchorTopPx }}
          onClick={onBubbleClick}
          aria-label={t("main.writingCoachBubbleAria")}
          title={t("main.writingCoachBubbleHint")}
        >
          {/* 柔光与菱形标：弱化为「参考提示」而非主按钮 */}
          <span className="writing-coach-bubble__halo" aria-hidden />
          <svg
            className="writing-coach-bubble__icon"
            viewBox="0 0 24 24"
            width={16}
            height={16}
            aria-hidden
          >
            <path
              fill="currentColor"
              fillOpacity={0.18}
              stroke="currentColor"
              strokeWidth={1.15}
              strokeLinejoin="round"
              d="M12 5 17 12 12 19 7 12 12 5z"
            />
          </svg>
        </button>
      ) : null}
      {panelOpen ? (
        <div className="writing-coach-panel" role="dialog" aria-label={t("main.writingCoachPanelTitle")}>
          <h3 className="writing-coach-panel__title">{t("main.writingCoachPanelTitle")}</h3>
          {panelError ? <p className="writing-coach-panel__err">{panelError}</p> : null}
          {panelLoading ? <p className="writing-coach-panel__list">{t("main.writingCoachLoading")}</p> : null}
          {!panelLoading && reasoningQuestions.length > 0 ? (
            <section className="writing-coach-panel__section">
              <ul className="writing-coach-panel__list">
                {reasoningQuestions.map((q, i) => (
                  <li key={i}>{q}</li>
                ))}
              </ul>
            </section>
          ) : null}
          {!panelLoading && !knowledgeModuleSkipped && links.length > 0 ? (
            <section className="writing-coach-panel__section">
              <div className="writing-coach-panel__title" style={{ fontSize: 12 }}>
                {t("main.writingCoachKnowledge")}
              </div>
              {links.map((lk, i) => (
                <button
                  key={`${lk.relPath}-${i}`}
                  type="button"
                  className="writing-coach-panel__link"
                  onClick={() => onOpenLink(lk.relPath)}
                  title={lk.excerpt ?? lk.relPath}
                >
                  [[{lk.title}]] · {lk.relPath}
                </button>
              ))}
            </section>
          ) : null}
          {!panelLoading && !panelError && reasoningQuestions.length === 0 && links.length === 0 ? (
            <p className="writing-coach-panel__list">{t("main.writingCoachNoSuggestions")}</p>
          ) : null}
          <div className="writing-coach-panel__actions">
            <button type="button" className="writing-coach-panel__btn" onClick={onCollapsePanel}>
              {t("main.writingCoachCollapse")}
            </button>
            <button type="button" className="writing-coach-panel__btn writing-coach-panel__btn--primary" onClick={onHelpful}>
              {t("main.writingCoachHelpful")}
            </button>
          </div>
        </div>
      ) : null}
    </>
  );
}
