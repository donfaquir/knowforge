import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  RightPanelGraphIcon,
  RightPanelLinkRecIcon,
  RightPanelOutlineIcon,
  RightPanelReviewIcon,
} from "./treeCollapseIcons";

export type RightPanelTab = "outline" | "ai" | "graph" | "linkRec" | "review";

type Props = {
  tab: RightPanelTab;
  onViewChange: (tab: RightPanelTab) => void;
  tauriDragExclude: boolean;
  /** 有可编辑 Markdown 时大纲 tab 可点；无打开文档时置灰 */
  outlineTabEnabled: boolean;
  /** 工作区就绪且 Tauri 时可打开理解网络 */
  graphTabEnabled: boolean;
  /** 有可编辑 Markdown 时可打开链接推荐 */
  linkRecTabEnabled: boolean;
  /** 与视图切换同一行右侧（大纲折叠工具条 / AI 顶栏等） */
  outlineToolbarEnd?: ReactNode;
  outlinePanel: ReactNode;
  aiPanel: ReactNode;
  graphPanel: ReactNode;
  linkRecPanel: ReactNode;
  /** 挑战式回顾独立队列 */
  reviewPanel: ReactNode;
  /** 有待回顾条数时在回顾 tab 上显示角标；null 表示不展示 */
  reviewTabBadgeCount?: number | null;
  /** 用户从顶栏点到「AI」或「思考网络」时回调（例如折叠左侧文件树） */
  onAfterSelectAiOrGraphTab?: () => void;
};

/** 右栏：顶栏分段（大纲 | AI | 理解网络 | 链接推荐 | 回顾）+ 内容区 */
export function RightPanelShell({
  tab,
  onViewChange,
  tauriDragExclude,
  outlineTabEnabled,
  graphTabEnabled,
  linkRecTabEnabled,
  outlineToolbarEnd,
  outlinePanel,
  aiPanel,
  graphPanel,
  linkRecPanel,
  reviewPanel,
  reviewTabBadgeCount = null,
  onAfterSelectAiOrGraphTab,
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
              onAfterSelectAiOrGraphTab?.();
            }}
          >
            <span className="right-panel-shell__segment-text">AI</span>
          </button>
          <button
            type="button"
            role="tab"
            id="right-panel-tab-graph"
            aria-selected={tab === "graph"}
            aria-controls="right-panel-panel-graph"
            className={`right-panel-shell__segment${tab === "graph" ? " is-active" : ""}`}
            aria-label={t("rightPanel.graphTabTitle")}
            title={t("rightPanel.graphTabTitle")}
            disabled={!graphTabEnabled}
            onClick={() => {
              if (!graphTabEnabled) {
                return;
              }
              onViewChange("graph");
              onAfterSelectAiOrGraphTab?.();
            }}
          >
            <RightPanelGraphIcon />
          </button>
          <button
            type="button"
            role="tab"
            id="right-panel-tab-link-rec"
            aria-selected={tab === "linkRec"}
            aria-controls="right-panel-panel-link-rec"
            className={`right-panel-shell__segment${tab === "linkRec" ? " is-active" : ""}`}
            aria-label={t("rightPanel.linkRecTabTitle")}
            title={t("rightPanel.linkRecTabTitle")}
            disabled={!linkRecTabEnabled}
            onClick={() => {
              if (!linkRecTabEnabled) {
                return;
              }
              onViewChange("linkRec");
              onAfterSelectAiOrGraphTab?.();
            }}
          >
            <RightPanelLinkRecIcon />
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
        {outlineToolbarEnd != null && outlineToolbarEnd !== false && tab !== "graph" && tab !== "linkRec" ? (
          <div className="right-panel-shell__top-bar__end">{outlineToolbarEnd}</div>
        ) : null}
      </div>
      {/* 非激活面板：hidden + inert，避免未展示区仍可聚焦或参与读屏遍历 */}
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
        id="right-panel-panel-graph"
        role="tabpanel"
        aria-labelledby="right-panel-tab-graph"
        hidden={tab !== "graph"}
        inert={tab !== "graph" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {graphPanel}
      </div>
      <div
        id="right-panel-panel-link-rec"
        role="tabpanel"
        aria-labelledby="right-panel-tab-link-rec"
        hidden={tab !== "linkRec"}
        inert={tab !== "linkRec" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {linkRecPanel}
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
