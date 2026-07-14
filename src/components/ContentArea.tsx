import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { AiConversationSessionProvider } from "../contexts/AiConversationSessionContext";
import { ThoughtMgmtAiConversationSessionProvider } from "../contexts/ThoughtMgmtAiConversationSessionContext";
import type { DepthMode } from "../types/cognitiveTypes";
import { AiConversationPanel } from "./AiConversationPanel";
import { AiConversationToolbar } from "./AiConversationToolbar";
import type { CrepeMarkdownEditorApi } from "./CrepeMarkdownEditor";
import { EditorThoughtsPanel } from "./EditorThoughtsPanel";
import { EditorView } from "./EditorView";
import type { EditorWritingCoachHostHandle } from "./EditorWritingCoachHost";
import { GraphTabShell } from "./GraphTabShell";
import { PracticeMode } from "./practice/PracticeMode";
import { LinkRecommendationPanel } from "./LinkRecommendationPanel";
import { OutlineBulkToolbar } from "./OutlineBulkToolbar";
import { OutlinePanel } from "./OutlinePanel";
import { RightPanelReviewTab } from "./RightPanelReviewTab";
import { RightPanelShell, type RightPanelTab } from "./RightPanelShell";
import { ThoughtManagementPanel } from "./ThoughtManagementPanel";
import type { LeftPanelView } from "./ActivityBar";
import type { useOpenDocs } from "../hooks/useOpenDocs";
import type { useOutline } from "../hooks/useOutline";
import type { OutlineFoldModel } from "../hooks/useOutlineFoldModel";
import type { WikiSuggestFileRow } from "../utils/flattenMarkdownTreeForWikiSuggest";

export interface ContentAreaProps {
  leftPanelView: LeftPanelView;
  sidebarOpen: boolean;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  rootPath: string | null;

  // Editor mode
  docState: ReturnType<typeof useOpenDocs>;
  editorReady: boolean;
  showMarkdownSource: boolean;
  activeMarkdownBodyForEditor: string | undefined;
  editorScrollRef: RefObject<HTMLDivElement | null>;
  crepeEditorApiRef: React.MutableRefObject<CrepeMarkdownEditorApi | null>;
  rawSourceTextareaRef: RefObject<HTMLTextAreaElement | null>;
  writingCoachRef: RefObject<EditorWritingCoachHostHandle | null>;
  wikiSuggestFiles: WikiSuggestFileRow[];
  isPathKfPrivate: (relPath: string) => boolean;
  onOpenCoachMarkdownPath: (relPath: string, meta?: { headingFragment?: string | null }) => Promise<void>;
  onToggleMarkdownSource: () => void;
  onKfPrivateBarToggle: () => void;
  onSaveAsThought: (text: string | null) => void;
  editorFindOpen: boolean;
  editorFindWorkspaceSeed: { query: string; caseSensitive: boolean; nonce: number } | null;
  onEditorFindClose: () => void;
  onEditorFindSeedConsumed: () => void;

  // Right panel
  showRightColumn: boolean;
  rightPanelOpen: boolean;
  rightPanelTab: RightPanelTab;
  onRightPanelTabChange: (tab: RightPanelTab) => void;
  onAfterSelectAiTab: () => void;
  rightResizableIsDragging: boolean;
  rightResizableHandleMouseDown: (e: React.MouseEvent) => void;
  initialDepthMode: DepthMode | undefined;
  reviewTabBadgeCount: number | null;

  // Outline
  outlineState: ReturnType<typeof useOutline>;
  outlineFold: OutlineFoldModel;
  navigateToHeading: (slug: string) => void;

  // Thought management
  thoughtManagementSessionActive: boolean;
  onThoughtMgmtDirtyChange: (dirty: boolean) => void;
  onSetLeftPanelView: (view: LeftPanelView) => void;

  // Tauri platform
  tauriDragExcludeProps: Record<string, unknown>;
  tauriWindowDragProps: Record<string, unknown>;
}

export function ContentArea(props: ContentAreaProps) {
  const { t } = useTranslation();
  const {
    leftPanelView,
    sidebarOpen,
    workspaceReady,
    tauriRuntime,
    rootPath,
    docState,
    editorReady,
    showMarkdownSource,
    activeMarkdownBodyForEditor,
    editorScrollRef,
    crepeEditorApiRef,
    rawSourceTextareaRef,
    writingCoachRef,
    wikiSuggestFiles,
    isPathKfPrivate,
    onOpenCoachMarkdownPath,
    onToggleMarkdownSource,
    onKfPrivateBarToggle,
    onSaveAsThought,
    editorFindOpen,
    editorFindWorkspaceSeed,
    onEditorFindClose,
    onEditorFindSeedConsumed,
    showRightColumn,
    rightPanelOpen,
    rightPanelTab,
    onRightPanelTabChange,
    onAfterSelectAiTab,
    rightResizableIsDragging,
    rightResizableHandleMouseDown,
    initialDepthMode,
    reviewTabBadgeCount,
    outlineState,
    outlineFold,
    navigateToHeading,
    thoughtManagementSessionActive,
    onThoughtMgmtDirtyChange,
    onSetLeftPanelView,
    tauriDragExcludeProps,
    tauriWindowDragProps,
  } = props;

  const current = docState.activeSession;

  return (
    <div
      className={`content-area${showRightColumn && leftPanelView === "files" ? " content-area--with-right-panel" : ""}${!sidebarOpen ? " content-area--sidebar-collapsed" : ""}`}
    >
      {leftPanelView === "graph" ? (
        <main className="main main--full-view">
          <GraphTabShell
            workspaceReady={workspaceReady}
            workspaceRoot={rootPath}
            tauriRuntime={tauriRuntime}
            onOpenNote={(relPath) => {
              onSetLeftPanelView("files");
              void onOpenCoachMarkdownPath(relPath);
            }}
          />
        </main>
      ) : leftPanelView === "thoughts" ? (
        thoughtManagementSessionActive ? (
          <main className="main main--full-view app-thought-management-route">
            <ThoughtMgmtAiConversationSessionProvider
              workspaceReady={workspaceReady}
              workspaceRoot={rootPath}
              tauriRuntime={tauriRuntime}
            >
              <ThoughtManagementPanel
                workspaceReady={workspaceReady}
                tauriRuntime={tauriRuntime}
                onThoughtDetailDirtyChange={onThoughtMgmtDirtyChange}
                onOpenNote={(relPath) => {
                  onSetLeftPanelView("files");
                  void onOpenCoachMarkdownPath(relPath);
                }}
                isPathKfPrivate={isPathKfPrivate}
              />
            </ThoughtMgmtAiConversationSessionProvider>
          </main>
        ) : null
      ) : leftPanelView === "practice" ? (
        <main className="main main--full-view">
          <PracticeMode
            workspaceReady={workspaceReady}
            tauriRuntime={tauriRuntime}
            workspaceRoot={rootPath}
          />
        </main>
      ) : (
        <EditorView
          docState={docState}
          showMarkdownSource={showMarkdownSource}
          activeMarkdownBodyForEditor={activeMarkdownBodyForEditor}
          editorScrollRef={editorScrollRef}
          crepeEditorApiRef={crepeEditorApiRef}
          rawSourceTextareaRef={rawSourceTextareaRef}
          writingCoachRef={writingCoachRef}
          wikiSuggestFiles={wikiSuggestFiles}
          isPathKfPrivate={isPathKfPrivate}
          onOpenCoachMarkdownPath={onOpenCoachMarkdownPath}
          onToggleMarkdownSource={onToggleMarkdownSource}
          onKfPrivateBarToggle={onKfPrivateBarToggle}
          onSaveAsThought={onSaveAsThought}
          editorFindOpen={editorFindOpen}
          editorFindWorkspaceSeed={editorFindWorkspaceSeed}
          onEditorFindClose={onEditorFindClose}
          onEditorFindSeedConsumed={onEditorFindSeedConsumed}
          workspaceReady={workspaceReady}
          tauriRuntime={tauriRuntime}
          tauriDragExcludeProps={tauriDragExcludeProps}
          tauriWindowDragProps={tauriWindowDragProps}
          rootPath={rootPath}
        />
      )}
      {showRightColumn && leftPanelView === "files" && (
        <div
          className={`panel-resizer panel-resizer--right${rightResizableIsDragging ? " panel-resizer--active" : ""}`}
          onMouseDown={rightResizableHandleMouseDown}
          role="separator"
          aria-label={t("toolbar.resizeRight")}
        />
      )}
      {showRightColumn && leftPanelView === "files" && (
        <AiConversationSessionProvider
          workspaceReady={workspaceReady}
          workspaceRoot={rootPath}
          tauriRuntime={tauriRuntime}
          initialDepthMode={initialDepthMode}
        >
          <RightPanelShell
            tab={rightPanelTab}
            onViewChange={onRightPanelTabChange}
            onAfterSelectAiTab={onAfterSelectAiTab}
            outlineTabEnabled={editorReady}
            linkRecTabEnabled={docState.activePath != null}
            tauriDragExclude={tauriRuntime}
            outlineToolbarEnd={
              rightPanelTab === "outline" && editorReady ? (
                <OutlineBulkToolbar
                  outlineHasBranches={outlineFold.outlineHasBranches}
                  allOutlineBranchesExpanded={outlineFold.allOutlineBranchesExpanded}
                  onToggleBulk={outlineFold.toggleAllOutlineBranchesBulk}
                />
              ) : rightPanelTab === "ai" ? (
                <AiConversationToolbar />
              ) : null
            }
            outlinePanel={
              <>
                <EditorThoughtsPanel
                  activeRelPath={docState.activePath}
                  outlinePanelActive={rightPanelOpen && rightPanelTab === "outline"}
                  savedMarkdownSnapshot={
                    docState.activePath &&
                    current &&
                    !current.loading &&
                    !current.loadError
                      ? current.savedContent
                      : undefined
                  }
                  editorContentInjectEpoch={
                    docState.activePath
                      ? (current?.contentInjectEpoch ?? 0)
                      : 0
                  }
                  workspaceReady={workspaceReady}
                  tauriRuntime={tauriRuntime}
                  onOpenNote={(relPath) => {
                    void onOpenCoachMarkdownPath(relPath);
                  }}
                />
                <OutlinePanel
                  fold={outlineFold}
                  onNavigate={navigateToHeading}
                  characterCount={outlineState.characterCount}
                  tauriDragRegion={tauriRuntime}
                />
              </>
            }
            aiPanel={<AiConversationPanel />}
            linkRecPanel={
              <LinkRecommendationPanel
                workspaceRoot={rootPath}
                activeRelPath={docState.activePath}
                panelActive={rightPanelOpen && rightPanelTab === "linkRec"}
                savedMarkdownSnapshot={
                  docState.activePath &&
                  current &&
                  !current.loading &&
                  !current.loadError
                    ? current.savedContent
                    : undefined
                }
                editorContentInjectEpoch={
                  docState.activePath ? (current?.contentInjectEpoch ?? 0) : 0
                }
                workspaceReady={workspaceReady}
                tauriRuntime={tauriRuntime}
                crepeApiRef={crepeEditorApiRef}
              />
            }
            reviewPanel={<RightPanelReviewTab onClose={() => onRightPanelTabChange("ai")} />}
            reviewTabBadgeCount={reviewTabBadgeCount}
          />
        </AiConversationSessionProvider>
      )}
    </div>
  );
}
