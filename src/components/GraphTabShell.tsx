import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { UnderstandingGraphPanel } from "./UnderstandingGraphPanel";
import { TopicNetworkPanel } from "./TopicNetworkPanel";
import "./GraphTabShell.css";

export type GraphSubTab = "wikilink" | "topic";

type Props = {
  workspaceReady: boolean;
  workspaceRoot: string | null;
  tauriRuntime: boolean;
  onOpenNote: (relPath: string) => void;
  onTogglePanelWide?: () => void;
  graphPanelWideExpanded?: boolean;
};

/** Graph 右栏：Wiki 链接图与主题二部图切换；两子面板均保持挂载以保留力导向状态（visibility 切换） */
export function GraphTabShell({
  workspaceReady,
  workspaceRoot,
  tauriRuntime,
  onOpenNote,
  onTogglePanelWide,
  graphPanelWideExpanded = false,
}: Props) {
  const { t } = useTranslation();
  const [subTab, setSubTab] = useState<GraphSubTab>("wikilink");

  const onWikilink = useCallback(() => setSubTab("wikilink"), []);
  const onTopic = useCallback(() => setSubTab("topic"), []);

  return (
    <div className="graph-tab-shell">
      <div className="graph-tab-shell__subtabs" role="tablist" aria-label={t("graphTabShell.subtabsAria")}>
        <button
          type="button"
          role="tab"
          aria-selected={subTab === "wikilink"}
          className={`graph-tab-shell__subtab${subTab === "wikilink" ? " is-active" : ""}`}
          onClick={onWikilink}
        >
          {t("graphTabShell.wikilink")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={subTab === "topic"}
          className={`graph-tab-shell__subtab${subTab === "topic" ? " is-active" : ""}`}
          onClick={onTopic}
        >
          {t("graphTabShell.topic")}
        </button>
      </div>
      <div
        className="graph-tab-shell__panel-wrap"
        role="tabpanel"
        hidden={subTab !== "wikilink"}
        inert={subTab !== "wikilink" ? true : undefined}
        aria-hidden={subTab !== "wikilink"}
      >
        <UnderstandingGraphPanel
          workspaceReady={workspaceReady}
          workspaceRoot={workspaceRoot}
          tauriRuntime={tauriRuntime}
          onOpenNote={onOpenNote}
          onTogglePanelWide={onTogglePanelWide}
          graphPanelWideExpanded={graphPanelWideExpanded}
        />
      </div>
      <div
        className="graph-tab-shell__panel-wrap"
        role="tabpanel"
        hidden={subTab !== "topic"}
        inert={subTab !== "topic" ? true : undefined}
        aria-hidden={subTab !== "topic"}
      >
        <TopicNetworkPanel
          workspaceReady={workspaceReady}
          workspaceRoot={workspaceRoot}
          tauriRuntime={tauriRuntime}
          onOpenNote={onOpenNote}
          onTogglePanelWide={onTogglePanelWide}
          graphPanelWideExpanded={graphPanelWideExpanded}
        />
      </div>
    </div>
  );
}
