import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  RightPanelOutlineIcon,
  RightPanelReviewIcon,
} from "./treeCollapseIcons";

export type RightPanelTab = "outline" | "ai" | "review";

type Props = {
  tab: RightPanelTab;
  onViewChange: (tab: RightPanelTab) => void;
  tauriDragExclude: boolean;
  outlineTabEnabled: boolean;
  outlineToolbarEnd?: ReactNode;
  outlinePanel: ReactNode;
  aiPanel: ReactNode;
  reviewPanel: ReactNode;
  reviewTabBadgeCount?: number | null;
  onAfterSelectAiTab?: () => void;
};

export function RightPanelShell({
  tab,
  onViewChange,
  tauriDragExclude,
  outlineTabEnabled,
  outlineToolbarEnd,
  outlinePanel,
  aiPanel,
  reviewPanel,
  reviewTabBadgeCount = null,
  onAfterSelectAiTab,
}: Props) {
  const { t } = useTranslation();
  const exclude = tauriDragExclude
    ? ({ "data-tauri-drag-region-exclude": true } as const)
    : {};

  return (
    <div className="right-panel-shell">
      <div
        className="right-panel-shell__top-bar"
        role="toolbar"
        aria-label={t("rightPanel.sidePanel")}
        {...exclude}
      >
        <div
          className="right-panel-shell__segmented"
          role="tablist"
          aria-label={t("rightPanel.switchTabs")}
        >
          <button
            type="button"
            role="tab"
            id="right-panel-tab-outline"
            aria-selected={tab === "outline"}
            aria-controls="right-panel-panel-outline"
            className={`right-panel-shell__segment${tab === "outline" ? " is-active" : ""}`}
            aria-label={t("rightPanel.outline")}
            title={t("rightPanel.outlineTitle")}
            disabled={!outlineTabEnabled}
            onClick={() => {
              if (outlineTabEnabled) onViewChange("outline");
            }}
          >
            <RightPanelOutlineIcon />
          </button>
          <button
            type="button"
            role="tab"
            id="right-panel-tab-ai"
            aria-selected={tab === "ai"}
            aria-controls="right-panel-panel-ai"
            className={`right-panel-shell__segment${tab === "ai" ? " is-active" : ""}`}
            aria-label={t("rightPanel.aiTitle")}
            title={t("rightPanel.aiTitle")}
            onClick={() => {
              onViewChange("ai");
              onAfterSelectAiTab?.();
            }}
          >
            <span className="right-panel-shell__segment-text">AI</span>
          </button>
          <button
            type="button"
            role="tab"
            id="right-panel-tab-review"
            aria-selected={tab === "review"}
            aria-controls="right-panel-panel-review"
            className={`right-panel-shell__segment${tab === "review" ? " is-active" : ""}`}
            aria-label={
              typeof reviewTabBadgeCount === "number" && reviewTabBadgeCount > 0
                ? `${t("rightPanel.reviewTabTitle")}（${t("rightPanel.reviewTabBadgeAria", { count: reviewTabBadgeCount })}）`
                : t("rightPanel.reviewTabTitle")
            }
            title={t("rightPanel.reviewTab")}
            onClick={() => onViewChange("review")}
          >
            <span className="right-panel-shell__segment-icon-wrap">
              <RightPanelReviewIcon />
              {typeof reviewTabBadgeCount === "number" && reviewTabBadgeCount > 0 ? (
                <span className="right-panel-shell__segment-badge" aria-hidden={true}>
                  {reviewTabBadgeCount > 99 ? "99+" : reviewTabBadgeCount}
                </span>
              ) : null}
            </span>
          </button>
        </div>
        {outlineToolbarEnd != null && outlineToolbarEnd !== false ? (
          <div className="right-panel-shell__top-bar__end">{outlineToolbarEnd}</div>
        ) : null}
      </div>
      <div
        id="right-panel-panel-outline"
        role="tabpanel"
        aria-labelledby="right-panel-tab-outline"
        hidden={tab !== "outline"}
        inert={tab !== "outline" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {outlinePanel}
      </div>
      <div
        id="right-panel-panel-ai"
        role="tabpanel"
        aria-labelledby="right-panel-tab-ai"
        hidden={tab !== "ai"}
        inert={tab !== "ai" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {aiPanel}
      </div>
      <div
        id="right-panel-panel-review"
        role="tabpanel"
        aria-labelledby="right-panel-tab-review"
        hidden={tab !== "review"}
        inert={tab !== "review" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {reviewPanel}
      </div>
    </div>
  );
}
