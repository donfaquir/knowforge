import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  RightPanelOutlineIcon,
  RightPanelLinkRecIcon,
} from "./treeCollapseIcons";

export type RightPanelTab = "outline" | "ai" | "linkRec";

type Props = {
  tab: RightPanelTab;
  onViewChange: (tab: RightPanelTab) => void;
  tauriDragExclude: boolean;
  outlineTabEnabled: boolean;
  linkRecTabEnabled: boolean;
  outlineToolbarEnd?: ReactNode;
  outlinePanel: ReactNode;
  aiPanel: ReactNode;
  linkRecPanel: ReactNode;
  onAfterSelectAiTab?: () => void;
};

export function RightPanelShell({
  tab,
  onViewChange,
  tauriDragExclude,
  outlineTabEnabled,
  linkRecTabEnabled,
  outlineToolbarEnd,
  outlinePanel,
  aiPanel,
  linkRecPanel,
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
            id="right-panel-tab-linkRec"
            aria-selected={tab === "linkRec"}
            aria-controls="right-panel-panel-linkRec"
            className={`right-panel-shell__segment${tab === "linkRec" ? " is-active" : ""}`}
            aria-label={t("rightPanel.linkRecTabTitle")}
            title={t("rightPanel.linkRecTabTitle")}
            disabled={!linkRecTabEnabled}
            onClick={() => {
              if (linkRecTabEnabled) onViewChange("linkRec");
            }}
          >
            <RightPanelLinkRecIcon />
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
        id="right-panel-panel-linkRec"
        role="tabpanel"
        aria-labelledby="right-panel-tab-linkRec"
        hidden={tab !== "linkRec"}
        inert={tab !== "linkRec" ? true : undefined}
        className="right-panel-shell__panel"
      >
        {linkRecPanel}
      </div>
    </div>
  );
}
