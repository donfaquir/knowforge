import { lazy, Suspense, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { EditorFindBar } from "./EditorFindBar";
import { EditorWritingCoachHost, type EditorWritingCoachHostHandle } from "./EditorWritingCoachHost";
import { KfPrivateLockIcon } from "./KfPrivateLockIcon";
import { MARKDOWN_TAB_PANEL_ID, editorTabDomId } from "./EditorTabBar";
import { KF_PRIVATE_LOCK_ICON_DOC_BAR_PX } from "../constants/kfPrivateUi";
import type { CrepeMarkdownEditorApi } from "./CrepeMarkdownEditor";
import type { useOpenDocs } from "../hooks/useOpenDocs";
import type { WikiSuggestFileRow } from "../utils/flattenMarkdownTreeForWikiSuggest";

const CrepeMarkdownEditor = lazy(() => import("./CrepeMarkdownEditor"));

export interface EditorViewProps {
  docState: ReturnType<typeof useOpenDocs>;
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
  workspaceReady: boolean;
  tauriRuntime: boolean;
  tauriDragExcludeProps: Record<string, unknown>;
  tauriWindowDragProps: Record<string, unknown>;
  rootPath: string | null;
}

export function EditorView(props: EditorViewProps) {
  const { t } = useTranslation();
  const {
    docState,
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
    workspaceReady,
    tauriDragExcludeProps,
    tauriWindowDragProps,
    rootPath,
  } = props;

  const current = docState.activeSession;
  const loadingDoc = !!current?.loading;
  const loadError = current?.loadError ?? null;
  const editorUsable = !!docState.activePath && !loadingDoc && !loadError && !!current;

  return (
    <main className="main">
      {docState.activePath != null &&
        docState.hasDiskStaleConflict(docState.activePath) &&
        editorUsable && (
          <div className="main__disk-notice" role="alert">
            <span className="main__disk-notice__text">{t("diskNotice.text")}</span>
            <div className="main__disk-notice__actions">
              <button
                type="button"
                className="main__disk-notice__btn main__disk-notice__btn--primary"
                onClick={() => {
                  const p = docState.activePath;
                  if (p) {
                    void docState.reloadFromDisk(p);
                  }
                }}
              >
                {t("diskNotice.reload")}
              </button>
              <button
                type="button"
                className="main__disk-notice__btn"
                onClick={() => {
                  const p = docState.activePath;
                  if (p) {
                    docState.dismissDiskStaleForPath(p);
                  }
                }}
              >
                {t("diskNotice.dismiss")}
              </button>
            </div>
          </div>
        )}
      {docState.saveError ? (
        <div className="main__save-error" role="alert">
          <span className="main__save-error__text">{docState.saveError}</span>
          <button
            type="button"
            className="main__save-error__dismiss"
            onClick={() => docState.setSaveError(null)}
            aria-label={t("main.dismiss")}
          >
            {t("main.dismiss")}
          </button>
        </div>
      ) : null}
      <div className="main__content">
        {docState.activePath ? (
          <div
            id={MARKDOWN_TAB_PANEL_ID}
            role="tabpanel"
            aria-labelledby={editorTabDomId(docState.activePath)}
            className="main__tab-panel"
          >
            {loadingDoc && <p className="main__placeholder">{t("main.loading")}</p>}
            {!loadingDoc && loadError && (
              <p className="main__placeholder">{t("main.loadError", { details: loadError })}</p>
            )}
            {!loadingDoc && !loadError && current && (
              <>
                <div
                  className="main__doc-bar"
                  aria-live="polite"
                  aria-atomic="true"
                  {...tauriWindowDragProps}
                >
                  <button
                    type="button"
                    className="file-tree__new-md main__doc-bar__privacy-icon-btn"
                    aria-pressed={isPathKfPrivate(docState.activePath)}
                    title={
                      isPathKfPrivate(docState.activePath)
                        ? t("kfPrivate.tooltipDocPrivate")
                        : t("kfPrivate.tooltipDocPublic")
                    }
                    aria-label={
                      isPathKfPrivate(docState.activePath)
                        ? t("kfPrivate.ariaRemovePrivate")
                        : t("kfPrivate.ariaMarkPrivate")
                    }
                    onClick={onKfPrivateBarToggle}
                    {...tauriDragExcludeProps}
                  >
                    {isPathKfPrivate(docState.activePath) ? (
                      <KfPrivateLockIcon
                        className="file-tree__new-md-icon"
                        size={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                      />
                    ) : (
                      <svg
                        className="file-tree__new-md-icon"
                        width={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                        height={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        aria-hidden={true}
                      >
                        <circle cx="12" cy="12" r="10" />
                        <line x1="2" y1="12" x2="22" y2="12" />
                        <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
                      </svg>
                    )}
                  </button>
                  <span className="main__doc-bar__path" dir="ltr" title={docState.activePath}>
                    {docState.activePath}
                  </span>
                  <div className="main__doc-bar__end">
                    <button
                      type="button"
                      className="file-tree__new-md main__doc-bar__source-toggle"
                      onClick={onToggleMarkdownSource}
                      aria-pressed={showMarkdownSource}
                      aria-label={
                        showMarkdownSource ? t("main.backToPreviewTitle") : t("main.viewRawSourceTitle")
                      }
                      title={
                        showMarkdownSource ? t("main.backToPreviewTitle") : t("main.viewRawSourceTitle")
                      }
                      {...tauriDragExcludeProps}
                    >
                      {showMarkdownSource ? (
                        <svg
                          className="file-tree__new-md-icon"
                          width={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                          height={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          aria-hidden={true}
                        >
                          <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                          <circle cx="12" cy="12" r="3" />
                        </svg>
                      ) : (
                        <svg
                          className="file-tree__new-md-icon"
                          width={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                          height={KF_PRIVATE_LOCK_ICON_DOC_BAR_PX}
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          aria-hidden={true}
                        >
                          <polyline points="16 18 22 12 16 6" />
                          <polyline points="8 6 2 12 8 18" />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>
                <div
                  className="editor-scroll"
                  ref={editorScrollRef}
                  aria-label={t("main.markdownEditor", { path: docState.activePath })}
                >
                  <div
                    className={`editor-scroll__body${showMarkdownSource ? " editor-scroll__body--raw" : ""}`}
                  >
                    <div className="editor-scroll__crepe-wrap">
                      <Suspense
                        fallback={
                          <div
                            className="editor-scroll__crepe-suspense-fallback"
                            aria-busy={true}
                          >
                            {t("settings.loading")}
                          </div>
                        }
                      >
                        <CrepeMarkdownEditor
                          docKey={docState.activePath}
                          initialMarkdown={activeMarkdownBodyForEditor ?? ""}
                          contentSyncKey={current.contentInjectEpoch}
                          onMarkdownChange={docState.handleMarkdownChange}
                          wikiSuggestFiles={wikiSuggestFiles}
                          onOpenInternalMarkdownLink={onOpenCoachMarkdownPath}
                          onEditorReady={(api) => {
                            crepeEditorApiRef.current = api;
                          }}
                          onEditorDispose={() => {
                            crepeEditorApiRef.current = null;
                          }}
                          onSaveAsThought={onSaveAsThought}
                        />
                      </Suspense>
                    </div>
                    {showMarkdownSource ? (
                      <textarea
                        ref={rawSourceTextareaRef}
                        className="main__raw-doc-source"
                        value={current.content}
                        onChange={(e) =>
                          docState.handleMarkdownChange(docState.activePath!, e.target.value, {
                            fullDocument: true,
                          })
                        }
                        spellCheck={false}
                        autoCapitalize="off"
                        autoCorrect="off"
                        aria-label={t("main.rawSourceAria")}
                        {...tauriDragExcludeProps}
                      />
                    ) : null}
                    <EditorWritingCoachHost
                      ref={writingCoachRef}
                      editorApiRef={crepeEditorApiRef}
                      activePath={docState.activePath}
                      workspaceReady={workspaceReady}
                      showMarkdownSource={showMarkdownSource}
                      onOpenMarkdownPath={onOpenCoachMarkdownPath}
                    />
                    <EditorFindBar
                      open={editorFindOpen}
                      onClose={onEditorFindClose}
                      previewMode={!showMarkdownSource}
                      rawFullMarkdown={current.content}
                      rawTextareaRef={rawSourceTextareaRef}
                      crepeApiRef={crepeEditorApiRef}
                      docKey={docState.activePath}
                      workspaceSearchJumpSeed={editorFindWorkspaceSeed}
                      onWorkspaceSearchJumpSeedConsumed={onEditorFindSeedConsumed}
                    />
                  </div>
                </div>
              </>
            )}
          </div>
        ) : null}
        {!docState.activePath && docState.tabPaths.length === 0 && rootPath && (
          <p className="main__placeholder">{t("main.selectFile")}</p>
        )}
        {!docState.activePath && docState.tabPaths.length === 0 && !rootPath && (
          <p className="main__placeholder">{t("main.openFolderFirst")}</p>
        )}
      </div>
    </main>
  );
}
