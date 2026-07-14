
import { useTranslation } from "react-i18next";
import { EditorTabBar } from "./EditorTabBar";
import type { useOpenDocs } from "../hooks/useOpenDocs";

export interface AppTopToolbarProps {
  // Tab bar
  tabPaths: string[];
  activePath: string | null;
  isDirty: (path: string) => boolean;
  hasDiskStaleConflict: (path: string) => boolean;
  isKfPrivate: (relPath: string) => boolean;
  onSelectTab: (path: string) => void;
  onCloseTab: (path: string) => void;
  onCloseAllTabs: () => void;
  onRenameTab?: ((path: string) => void) | undefined;
  tauriDragExclude: boolean;

  // Toolbar buttons
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  rightPanelOpen: boolean;
  onToggleRightPanel: () => void;
  workspaceReady: boolean;
  editorUsable: boolean;
  saveDisabled: boolean;
  saving: boolean;
  saveFeedback: ReturnType<typeof useOpenDocs>["saveFeedback"];
  onSave: () => void;
  onOpenWorkspaceSearch: () => void;

  // Platform
  tauriRuntime: boolean;
  isMacPlatform: boolean;
  tauriDragExcludeProps: Record<string, unknown>;
  tauriWindowDragProps: Record<string, unknown>;

  // Titlebar
  onTitlebarMouseDown: (e: React.MouseEvent<HTMLDivElement>) => void;
  onTitlebarDoubleClick: (e: React.MouseEvent<HTMLDivElement>) => void;
  // Window controls
  appWindow: { minimize: () => Promise<void>; toggleMaximize: () => Promise<void>; close: () => Promise<void> } | null;
}

export function AppTopToolbar(props: AppTopToolbarProps) {
  const { t } = useTranslation();
  const {
    tabPaths,
    activePath,
    isDirty,
    hasDiskStaleConflict,
    isKfPrivate,
    onSelectTab,
    onCloseTab,
    onCloseAllTabs,
    onRenameTab,
    tauriDragExclude,
    sidebarOpen,
    onToggleSidebar,
    rightPanelOpen,
    onToggleRightPanel,
    workspaceReady,
    editorUsable,
    saveDisabled,
    saving,
    saveFeedback,
    onSave,
    onOpenWorkspaceSearch,
    tauriRuntime,
    isMacPlatform,
    tauriDragExcludeProps,
    onTitlebarMouseDown,
    onTitlebarDoubleClick,
    appWindow,
  } = props;

  const renderWindowControls = (placement: "leading" | "trailing") => {
    if (!tauriRuntime) {
      return null;
    }

    const renderMacControls = isMacPlatform && placement === "leading";
    const renderDesktopControls = !isMacPlatform && placement === "trailing";
    if (!renderMacControls && !renderDesktopControls) {
      return null;
    }

    /* macOS: system traffic lights handled by Tauri (Transparent + decorations) */
    if (renderMacControls) {
      return null;
    }

    return (
      <div
        className="app-top-toolbar__window-controls app-top-toolbar__window-controls--desktop"
        {...tauriDragExcludeProps}
        aria-label={t("toolbar.windowControls")}
      >
        <button
          type="button"
          className="app-top-toolbar__window-control"
          onClick={() => void appWindow?.minimize()}
          aria-label={t("toolbar.minimize")}
          title={t("toolbar.minimize")}
        >
          <svg
            className="app-top-toolbar__window-control-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden={true}
          >
            <path d="M5 12h14" />
          </svg>
        </button>
        <button
          type="button"
          className="app-top-toolbar__window-control"
          onClick={() => void appWindow?.toggleMaximize()}
          aria-label={t("toolbar.maximize")}
          title={t("toolbar.maximize")}
        >
          <svg
            className="app-top-toolbar__window-control-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden={true}
          >
            <rect x="5" y="5" width="14" height="14" rx="1.5" />
          </svg>
        </button>
        <button
          type="button"
          className="app-top-toolbar__window-control app-top-toolbar__window-control--close"
          onClick={() => void appWindow?.close()}
          aria-label={t("toolbar.close")}
          title={t("toolbar.close")}
        >
          <svg
            className="app-top-toolbar__window-control-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden={true}
          >
            <path d="M6 6l12 12" />
            <path d="M18 6L6 18" />
          </svg>
        </button>
      </div>
    );
  };

  return (
    <div
      className="app-top-toolbar__banner"
      onMouseDown={onTitlebarMouseDown}
      onDoubleClick={onTitlebarDoubleClick}
    >
      <div className="app-top-toolbar__start" role="toolbar" aria-label={t("toolbar.files")}>
        {renderWindowControls("leading")}
        <button
          type="button"
          className={`app-top-toolbar__sidebar-toggle${sidebarOpen ? " is-active" : ""}`}
          {...tauriDragExcludeProps}
          onClick={onToggleSidebar}
          aria-pressed={sidebarOpen}
          aria-label={sidebarOpen ? t("toolbar.hideTree") : t("toolbar.showTree")}
          title={sidebarOpen ? t("toolbar.hideTree") : t("toolbar.showTree")}
        >
          <svg
            className="app-top-toolbar__sidebar-toggle__icon"
            width="19"
            height="19"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden={true}
          >
            <rect x="3" y="3" width="18" height="18" rx="2.5" />
            <path d="M9 5.5v13" />
          </svg>
        </button>
        {tauriRuntime && workspaceReady ? (
          <button
            type="button"
            className="app-top-toolbar__thought-hub"
            {...tauriDragExcludeProps}
            onClick={onOpenWorkspaceSearch}
            aria-label={t("toolbar.workspaceSearch")}
            title={t("toolbar.workspaceSearchTitle")}
          >
            <svg
              className="app-top-toolbar__thought-hub-icon"
              width="19"
              height="19"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden={true}
              focusable="false"
            >
              <circle cx="11" cy="11" r="8" />
              <path d="m21 21-4.35-4.35" />
            </svg>
          </button>
        ) : null}
      </div>
      <div className="app-top-toolbar__editor">
        <div className="app-top-toolbar__tabs">
          <EditorTabBar
            tabs={tabPaths}
            activePath={activePath}
            isDirty={isDirty}
            hasDiskStaleConflict={hasDiskStaleConflict}
            tauriDragExclude={tauriDragExclude}
            isKfPrivate={isKfPrivate}
            onRenameTab={onRenameTab}
            onSelect={onSelectTab}
            onClose={onCloseTab}
            onCloseAll={onCloseAllTabs}
          />
        </div>
        <div className="app-top-toolbar__end">
          {tauriRuntime && workspaceReady && editorUsable && saveFeedback !== "idle" ? (
            <span className="app-top-toolbar__save-status" aria-live="polite">
              {saveFeedback === "pending_auto"
                ? t("toolbar.autoSavePending")
                : saveFeedback === "saving"
                  ? t("toolbar.saving")
                  : saveFeedback === "saved"
                    ? t("toolbar.saved")
                    : null}
            </span>
          ) : null}
          <button
            type="button"
            className="app-top-toolbar__save"
            {...tauriDragExcludeProps}
            disabled={saveDisabled}
            aria-busy={saving || saveFeedback === "saving"}
            aria-label={
              saving || saveFeedback === "saving"
                ? t("toolbar.saving")
                : t("toolbar.save")
            }
            title={
              saving || saveFeedback === "saving"
                ? t("toolbar.saving")
                : isMacPlatform
                  ? t("toolbar.saveMac")
                  : t("toolbar.saveWin")
            }
            onClick={onSave}
          >
            <svg
              className="app-top-toolbar__save-icon"
              width="19"
              height="19"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden={true}
            >
              <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" />
              <polyline points="17 21 17 13 7 13 7 21" />
            </svg>
          </button>
          <button
            type="button"
            className={`main__right-panel-toggle${rightPanelOpen ? " is-active" : ""}`}
            {...tauriDragExcludeProps}
            onClick={onToggleRightPanel}
            disabled={!workspaceReady}
            aria-pressed={rightPanelOpen}
            aria-label={rightPanelOpen ? t("toolbar.hideSidePanel") : t("toolbar.showSidePanel")}
            title={
              rightPanelOpen ? t("toolbar.hideSidePanelTitle") : t("toolbar.showSidePanelTitle")
            }
          >
            <svg
              className="main__right-panel-toggle__icon"
              width="19"
              height="19"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden={true}
              focusable="false"
            >
              <rect x="3" y="3" width="18" height="18" rx="2.5" />
              <path d="M15 5.5v13" />
            </svg>
          </button>
          {renderWindowControls("trailing")}
        </div>
      </div>
    </div>
  );
}
