import { invoke, isTauri } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState, type ReactElement } from "react";
import { useTranslation } from "react-i18next";
import type { FileTreeFileOps, TreeNode } from "../components/FileTree";
import {
  collectMarkdownFileRelPaths,
  DEFAULT_NEW_MARKDOWN_TEMPLATE,
  joinRelPath,
  nextUntitledRelPathInDir,
  normalizeFolderBasename,
  normalizeMarkdownBasename,
  parentDirOfRelPath,
} from "../utils/newUntitledMarkdownPath";

/** 与 useOpenDocs 配合：仅依赖文件操作所需 API，便于单测与复用 */
export type WorkspaceFileCommandsDocState = {
  setSaveError: (msg: string | null) => void;
  openOrFocusTab: (relPath: string) => Promise<string | null>;
  renameTabPath: (fromRel: string, toRel: string) => void;
  renameDirectoryInOpenDocs: (fromDirRel: string, toDirRel: string) => void;
  removeTabByPath: (relPath: string) => void;
  removeTabsUnderDirectory: (dirRel: string) => void;
  isDirty: (relPath: string) => boolean;
  tabPaths: string[];
};

export type UseWorkspaceFileCommandsOptions = {
  workspaceReady: boolean;
  rootPath: string | null;
  refreshTree: () => Promise<TreeNode[] | null>;
  docState: WorkspaceFileCommandsDocState;
  /** 用于工具栏「新建文件夹」默认落在当前打开文件的父目录 */
  activePath: string | null;
  /** 与 isTauri() 一致；非 Tauri 时不注册文件类 invoke */
  tauriRuntime: boolean;
  /** Ctrl+Shift+Y / ⌘+Shift+Y：打开 AI 栏并开始挑战式回顾（与顶栏 chip 一致） */
  onStartChallengeReview?: () => void;
};

export type NewMarkdownToolbarProps = {
  onClick: () => void;
  disabled: boolean;
  busy: boolean;
  tooltipTitle: string;
};

/** 与新建 Markdown 工具条按钮形态一致 */
export type NewFolderToolbarProps = NewMarkdownToolbarProps;

type NewMdDialogState = { dirRel: string; defaultBasename: string } | null;
type NewFolderDialogState = { dirRel: string } | null;
type RenameDialogState = { relPath: string; value: string } | null;
type RenameFolderDialogState = { dirRel: string } | null;

type ModalsProps = {
  newMdDialog: NewMdDialogState;
  newMdBasenameInput: string;
  setNewMdBasenameInput: (v: string) => void;
  newMarkdownBusy: boolean;
  onDismissNewMd: () => void;
  onConfirmNewMd: () => void | Promise<void>;
  newFolderDialog: NewFolderDialogState;
  newFolderNameInput: string;
  setNewFolderNameInput: (v: string) => void;
  newFolderBusy: boolean;
  onDismissNewFolder: () => void;
  onConfirmNewFolder: () => void | Promise<void>;
  renameDialog: RenameDialogState;
  renameValueInput: string;
  setRenameValueInput: (v: string) => void;
  onDismissRename: () => void;
  onConfirmRename: () => void | Promise<void>;
  renameFolderDialog: RenameFolderDialogState;
  renameFolderNameInput: string;
  setRenameFolderNameInput: (v: string) => void;
  onDismissRenameFolder: () => void;
  onConfirmRenameFolder: () => void | Promise<void>;
};

/** 新建 / 重命名模态框（样式依赖 App.css） */
export function WorkspaceFileCommandModals({
  newMdDialog,
  newMdBasenameInput,
  setNewMdBasenameInput,
  newMarkdownBusy,
  onDismissNewMd,
  onConfirmNewMd,
  newFolderDialog,
  newFolderNameInput,
  setNewFolderNameInput,
  newFolderBusy,
  onDismissNewFolder,
  onConfirmNewFolder,
  renameDialog,
  renameValueInput,
  setRenameValueInput,
  onDismissRename,
  onConfirmRename,
  renameFolderDialog,
  renameFolderNameInput,
  setRenameFolderNameInput,
  onDismissRenameFolder,
  onConfirmRenameFolder,
}: ModalsProps): ReactElement | null {
  const { t } = useTranslation();
  if (!newMdDialog && !newFolderDialog && !renameDialog && !renameFolderDialog) {
    return null;
  }
  return (
    <>
      {newMdDialog ? (
        <div className="app-modal-scrim" role="presentation">
          <div
            className="app-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="new-md-dialog-title"
          >
            <h2 id="new-md-dialog-title" className="app-modal__title">
              {t("modals.newMarkdown")}
            </h2>
            <p className="app-modal__hint">
              {newMdDialog.dirRel === ""
                ? `${t("modals.target")} ${t("modals.workspaceRoot")}`
                : `${t("modals.target")} ${newMdDialog.dirRel}`}
            </p>
            <label className="visually-hidden" htmlFor="new-md-basename">
              {t("modals.fileName")}
            </label>
            <input
              id="new-md-basename"
              className="app-modal__field"
              value={newMdBasenameInput}
              onChange={(e) => setNewMdBasenameInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onConfirmNewMd();
                }
              }}
              autoComplete="off"
            />
            <div className="app-modal__actions">
              <button type="button" className="app-modal__btn" onClick={onDismissNewMd}>
                {t("modals.cancel")}
              </button>
              <button
                type="button"
                className="app-modal__btn app-modal__btn--primary"
                disabled={newMarkdownBusy}
                onClick={() => void onConfirmNewMd()}
              >
                {t("modals.create")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {newFolderDialog ? (
        <div className="app-modal-scrim" role="presentation">
          <div
            className="app-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="new-folder-dialog-title"
          >
            <h2 id="new-folder-dialog-title" className="app-modal__title">
              {t("modals.newFolder")}
            </h2>
            <p className="app-modal__hint">
              {newFolderDialog.dirRel === ""
                ? `${t("modals.target")} ${t("modals.workspaceRoot")}`
                : `${t("modals.target")} ${newFolderDialog.dirRel}`}
            </p>
            <label className="visually-hidden" htmlFor="new-folder-name">
              {t("modals.folderName")}
            </label>
            <input
              id="new-folder-name"
              className="app-modal__field"
              value={newFolderNameInput}
              onChange={(e) => setNewFolderNameInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onConfirmNewFolder();
                }
              }}
              autoComplete="off"
            />
            <div className="app-modal__actions">
              <button type="button" className="app-modal__btn" onClick={onDismissNewFolder}>
                {t("modals.cancel")}
              </button>
              <button
                type="button"
                className="app-modal__btn app-modal__btn--primary"
                disabled={newFolderBusy}
                onClick={() => void onConfirmNewFolder()}
              >
                {t("modals.create")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {renameDialog ? (
        <div className="app-modal-scrim" role="presentation">
          <div
            className="app-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rename-dialog-title"
          >
            <h2 id="rename-dialog-title" className="app-modal__title">
              {t("modals.rename")}
            </h2>
            <p className="app-modal__hint">{renameDialog.relPath}</p>
            <label className="visually-hidden" htmlFor="rename-md-basename">
              {t("modals.newFileName")}
            </label>
            <input
              id="rename-md-basename"
              className="app-modal__field"
              value={renameValueInput}
              onChange={(e) => setRenameValueInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onConfirmRename();
                }
              }}
              autoComplete="off"
            />
            <div className="app-modal__actions">
              <button type="button" className="app-modal__btn" onClick={onDismissRename}>
                {t("modals.cancel")}
              </button>
              <button
                type="button"
                className="app-modal__btn app-modal__btn--primary"
                onClick={() => void onConfirmRename()}
              >
                {t("modals.ok")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {renameFolderDialog ? (
        <div className="app-modal-scrim" role="presentation">
          <div
            className="app-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rename-folder-dialog-title"
          >
            <h2 id="rename-folder-dialog-title" className="app-modal__title">
              {t("modals.rename")}
            </h2>
            <p className="app-modal__hint">{renameFolderDialog.dirRel}</p>
            <label className="visually-hidden" htmlFor="rename-folder-basename">
              {t("modals.newFolderName")}
            </label>
            <input
              id="rename-folder-basename"
              className="app-modal__field"
              value={renameFolderNameInput}
              onChange={(e) => setRenameFolderNameInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void onConfirmRenameFolder();
                }
              }}
              autoComplete="off"
            />
            <div className="app-modal__actions">
              <button type="button" className="app-modal__btn" onClick={onDismissRenameFolder}>
                {t("modals.cancel")}
              </button>
              <button
                type="button"
                className="app-modal__btn app-modal__btn--primary"
                onClick={() => void onConfirmRenameFolder()}
              >
                {t("modals.ok")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

export function useWorkspaceFileCommands({
  workspaceReady,
  rootPath,
  refreshTree,
  docState,
  activePath,
  tauriRuntime,
  onStartChallengeReview,
}: UseWorkspaceFileCommandsOptions) {
  const { t } = useTranslation();
  const {
    setSaveError,
    openOrFocusTab,
    renameTabPath,
    renameDirectoryInOpenDocs,
    removeTabByPath,
    removeTabsUnderDirectory,
    isDirty,
    tabPaths,
  } = docState;

  const [newMarkdownBusy, setNewMarkdownBusy] = useState(false);
  const [preferredNewMarkdownDir, setPreferredNewMarkdownDir] = useState("");
  const [newMdDialog, setNewMdDialog] = useState<NewMdDialogState>(null);
  const [newMdBasenameInput, setNewMdBasenameInput] = useState("");
  const [renameDialog, setRenameDialog] = useState<RenameDialogState>(null);
  const [renameValueInput, setRenameValueInput] = useState("");
  const [renameFolderDialog, setRenameFolderDialog] = useState<RenameFolderDialogState>(null);
  const [renameFolderNameInput, setRenameFolderNameInput] = useState("");
  const [newFolderDialog, setNewFolderDialog] = useState<NewFolderDialogState>(null);
  const [newFolderNameInput, setNewFolderNameInput] = useState("");
  const [newFolderBusy, setNewFolderBusy] = useState(false);

  useEffect(() => {
    if (newMdDialog) {
      setNewMdBasenameInput(newMdDialog.defaultBasename);
    }
  }, [newMdDialog]);

  useEffect(() => {
    if (newFolderDialog) {
      setNewFolderNameInput(t("fileOps.newFolderDefault"));
    }
  }, [newFolderDialog, t]);

  useEffect(() => {
    if (renameDialog) {
      setRenameValueInput(renameDialog.value);
    }
  }, [renameDialog]);

  useEffect(() => {
    if (renameFolderDialog) {
      const dr = renameFolderDialog.dirRel;
      const slash = dr.lastIndexOf("/");
      setRenameFolderNameInput(slash < 0 ? dr : dr.slice(slash + 1));
    }
  }, [renameFolderDialog]);

  useEffect(() => {
    if (!newMdDialog && !newFolderDialog && !renameDialog && !renameFolderDialog) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setNewMdDialog(null);
        setNewFolderDialog(null);
        setRenameDialog(null);
        setRenameFolderDialog(null);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [newMdDialog, newFolderDialog, renameDialog, renameFolderDialog]);

  /** 开始回顾：与 App 顶栏 chip 共用回调，避免重复注册时合并到一处 */
  useEffect(() => {
    if (!tauriRuntime || !workspaceReady || !onStartChallengeReview) return;
    const onKey = (e: KeyboardEvent) => {
      if (!e.shiftKey || !(e.metaKey || e.ctrlKey) || e.code !== "KeyY" || e.repeat) return;
      const t = e.target;
      if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement) {
        return;
      }
      if (t instanceof HTMLElement && t.isContentEditable) return;
      e.preventDefault();
      onStartChallengeReview();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [tauriRuntime, workspaceReady, onStartChallengeReview]);

  const resetWorkspaceFileCommands = useCallback(() => {
    setPreferredNewMarkdownDir("");
    setNewMdDialog(null);
    setNewFolderDialog(null);
    setRenameDialog(null);
    setRenameFolderDialog(null);
  }, []);

  const modalOpen =
    newMdDialog !== null ||
    newFolderDialog !== null ||
    renameDialog !== null ||
    renameFolderDialog !== null;

  const openNewFolderDialog = useCallback(
    (dirRel: string) => {
      if (!isTauri() || !rootPath || !workspaceReady || modalOpen) {
        return;
      }
      setSaveError(null);
      setNewFolderDialog({ dirRel });
    },
    [rootPath, workspaceReady, modalOpen, setSaveError],
  );

  const confirmNewFolder = useCallback(async () => {
    if (!isTauri() || !newFolderDialog) {
      return;
    }
    const name = normalizeFolderBasename(newFolderNameInput);
    if (!name) {
      setSaveError(t("errors.invalidFolderName"));
      return;
    }
    setSaveError(null);
    setNewFolderBusy(true);
    try {
      await invoke("create_workspace_folder", {
        parentRel: newFolderDialog.dirRel,
        folderName: name,
      });
      await refreshTree();
      setNewFolderDialog(null);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setNewFolderBusy(false);
    }
  }, [newFolderDialog, newFolderNameInput, refreshTree, setSaveError, t]);

  const openNewMarkdownDialog = useCallback(
    async (dirRel: string) => {
      if (!isTauri() || !rootPath || !workspaceReady || modalOpen) {
        return;
      }
      setSaveError(null);
      try {
        const nodes = await refreshTree();
        if (!nodes) {
          return;
        }
        const existing = collectMarkdownFileRelPaths(nodes);
        const suggestedRel = nextUntitledRelPathInDir(dirRel, existing);
        const slash = suggestedRel.lastIndexOf("/");
        const defaultBasename = slash < 0 ? suggestedRel : suggestedRel.slice(slash + 1);
        setNewMdDialog({ dirRel, defaultBasename });
      } catch (e) {
        setSaveError(e instanceof Error ? e.message : String(e));
      }
    },
    [rootPath, workspaceReady, modalOpen, refreshTree, setSaveError],
  );

  const handleNewMarkdown = useCallback(() => {
    void openNewMarkdownDialog(preferredNewMarkdownDir);
  }, [openNewMarkdownDialog, preferredNewMarkdownDir]);

  const confirmNewMarkdown = useCallback(async () => {
    if (!isTauri() || !newMdDialog) {
      return;
    }
    const newBase = normalizeMarkdownBasename(newMdBasenameInput);
    if (!newBase) {
      setSaveError(t("errors.invalidFileName"));
      return;
    }
    const relPath = joinRelPath(newMdDialog.dirRel, newBase);
    setSaveError(null);
    setNewMarkdownBusy(true);
    try {
      const nodes = await refreshTree();
      if (!nodes) {
        return;
      }
      const existing = collectMarkdownFileRelPaths(nodes);
      if (existing.has(relPath)) {
        setSaveError(t("errors.fileExists"));
        return;
      }
      await invoke("write_markdown_file", {
        relPath,
        content: DEFAULT_NEW_MARKDOWN_TEMPLATE,
      });
      await refreshTree();
      setPreferredNewMarkdownDir(newMdDialog.dirRel);
      setNewMdDialog(null);
      await openOrFocusTab(relPath);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    } finally {
      setNewMarkdownBusy(false);
    }
  }, [newMdDialog, newMdBasenameInput, refreshTree, setSaveError, openOrFocusTab, t]);

  const handleRenameFile = useCallback(
    (relPath: string) => {
      if (newMdDialog !== null || newFolderDialog !== null || renameFolderDialog !== null) {
        return;
      }
      const slash = relPath.lastIndexOf("/");
      const base = slash < 0 ? relPath : relPath.slice(slash + 1);
      setRenameDialog({ relPath, value: base });
    },
    [newMdDialog, newFolderDialog, renameFolderDialog],
  );

  const handleRenameFolder = useCallback(
    (dirRel: string) => {
      if (!isTauri() || modalOpen) {
        return;
      }
      setSaveError(null);
      setRenameFolderDialog({ dirRel });
    },
    [modalOpen, setSaveError],
  );

  const confirmRename = useCallback(async () => {
    if (!renameDialog || !isTauri()) {
      return;
    }
    const newBase = normalizeMarkdownBasename(renameValueInput);
    if (!newBase) {
      setSaveError(t("errors.invalidFileName"));
      return;
    }
    const nextPath = joinRelPath(parentDirOfRelPath(renameDialog.relPath), newBase);
    if (nextPath === renameDialog.relPath) {
      const rel = renameDialog.relPath;
      if (isDirty(rel)) {
        const slash = rel.lastIndexOf("/");
        const shortName = slash < 0 ? rel : rel.slice(slash + 1);
        const ok = await ask(t("dialogs.renameUnsaved", { name: shortName }), {
          title: t("dialogs.rename"),
          kind: "warning",
        });
        if (!ok) {
          return;
        }
      }
      setRenameDialog(null);
      return;
    }
    setSaveError(null);
    try {
      await invoke("move_workspace_entry", { fromRel: renameDialog.relPath, toRel: nextPath });
      renameTabPath(renameDialog.relPath, nextPath);
      await refreshTree();
      setRenameDialog(null);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [renameDialog, renameValueInput, refreshTree, setSaveError, renameTabPath, isDirty, t]);

  const confirmRenameFolder = useCallback(async () => {
    if (!renameFolderDialog || !isTauri()) {
      return;
    }
    const newName = normalizeFolderBasename(renameFolderNameInput);
    if (!newName) {
      setSaveError(t("errors.invalidFolderName"));
      return;
    }
    const fromNorm = renameFolderDialog.dirRel.trim().replace(/\/+$/, "");
    const nextPath = joinRelPath(parentDirOfRelPath(fromNorm), newName);
    if (nextPath === fromNorm) {
      setRenameFolderDialog(null);
      return;
    }
    const childPrefix = `${fromNorm}/`;
    const anyDirty = tabPaths.some(
      (p) => (p === fromNorm || p.startsWith(childPrefix)) && isDirty(p),
    );
    if (anyDirty) {
      const ok = await ask(t("dialogs.renameFolderDirty"), {
        title: t("dialogs.rename"),
        kind: "warning",
      });
      if (!ok) {
        return;
      }
    }
    setSaveError(null);
    try {
      await invoke("rename_workspace_folder", { fromRel: fromNorm, toRel: nextPath });
      renameDirectoryInOpenDocs(fromNorm, nextPath);
      setPreferredNewMarkdownDir((d) => {
        if (d === fromNorm) {
          return nextPath;
        }
        const pref = `${fromNorm}/`;
        if (d.startsWith(pref)) {
          return nextPath + d.slice(fromNorm.length);
        }
        return d;
      });
      await refreshTree();
      setRenameFolderDialog(null);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [
    renameFolderDialog,
    renameFolderNameInput,
    refreshTree,
    setSaveError,
    renameDirectoryInOpenDocs,
    tabPaths,
    isDirty,
    setPreferredNewMarkdownDir,
    t,
  ]);

  const handleDeleteFile = useCallback(
    async (relPath: string) => {
      if (!isTauri()) {
        return;
      }
      const slash = relPath.lastIndexOf("/");
      const name = slash < 0 ? relPath : relPath.slice(slash + 1);
      const dirty = isDirty(relPath);
      const ok = dirty
        ? await ask(t("dialogs.deleteFileDirty", { name }), {
            title: t("dialogs.delete"),
            kind: "warning",
          })
        : await ask(t("dialogs.deleteFileConfirm", { name }), {
            title: t("dialogs.delete"),
            kind: "warning",
          });
      if (!ok) {
        return;
      }
      setSaveError(null);
      try {
        await invoke("delete_markdown_file", { relPath });
        removeTabByPath(relPath);
        await refreshTree();
      } catch (e) {
        setSaveError(e instanceof Error ? e.message : String(e));
      }
    },
    [refreshTree, isDirty, removeTabByPath, setSaveError, t],
  );

  const handleDeleteFolder = useCallback(
    async (dirRel: string) => {
      if (!isTauri()) {
        return;
      }
      const norm = dirRel.trim().replace(/\/+$/, "");
      const slash = norm.lastIndexOf("/");
      const shortName = slash < 0 ? norm : norm.slice(slash + 1);
      const ok = await ask(t("dialogs.deleteFolder", { name: shortName }), {
        title: t("dialogs.delete"),
        kind: "warning",
      });
      if (!ok) {
        return;
      }
      setSaveError(null);
      try {
        await invoke("delete_workspace_folder", { relPath: norm });
        removeTabsUnderDirectory(norm);
        await refreshTree();
      } catch (e) {
        setSaveError(e instanceof Error ? e.message : String(e));
      }
    },
    [refreshTree, removeTabsUnderDirectory, setSaveError, t],
  );

  const newMarkdownTooltip = t("fileTree.newMarkdown");

  const newFolderTargetDir = activePath ? parentDirOfRelPath(activePath) : preferredNewMarkdownDir;

  const newFolderTooltip = t("fileTree.newFolder");

  const newMarkdownUiBlocked =
    newMarkdownBusy ||
    newFolderBusy ||
    newMdDialog !== null ||
    newFolderDialog !== null ||
    renameDialog !== null ||
    renameFolderDialog !== null;

  const fileTreeFileOps: FileTreeFileOps | null = tauriRuntime
    ? {
        onNewInDirectory: (dirRel) => void openNewMarkdownDialog(dirRel),
        onNewFolderInDirectory: (dirRel) => openNewFolderDialog(dirRel),
        onRenameFile: handleRenameFile,
        onDeleteFile: (relPath) => void handleDeleteFile(relPath),
        onDeleteFolder: (dirRel) => void handleDeleteFolder(dirRel),
        onRenameFolder: (dirRel) => void handleRenameFolder(dirRel),
      }
    : null;

  const newMarkdownToolbar: NewMarkdownToolbarProps | null = tauriRuntime
    ? {
        onClick: () => void handleNewMarkdown(),
        disabled: !workspaceReady || newMarkdownUiBlocked,
        busy: newMarkdownBusy,
        tooltipTitle: newMarkdownTooltip,
      }
    : null;

  const newFolderToolbar: NewFolderToolbarProps | null = tauriRuntime
    ? {
        onClick: () => void openNewFolderDialog(newFolderTargetDir),
        disabled: !workspaceReady || newMarkdownUiBlocked,
        busy: newFolderBusy,
        tooltipTitle: newFolderTooltip,
      }
    : null;

  const onRenameTabFromBar = tauriRuntime ? handleRenameFile : undefined;

  const fileCommandModals = (
    <WorkspaceFileCommandModals
      newMdDialog={newMdDialog}
      newMdBasenameInput={newMdBasenameInput}
      setNewMdBasenameInput={setNewMdBasenameInput}
      newMarkdownBusy={newMarkdownBusy}
      onDismissNewMd={() => setNewMdDialog(null)}
      onConfirmNewMd={confirmNewMarkdown}
      newFolderDialog={newFolderDialog}
      newFolderNameInput={newFolderNameInput}
      setNewFolderNameInput={setNewFolderNameInput}
      newFolderBusy={newFolderBusy}
      onDismissNewFolder={() => setNewFolderDialog(null)}
      onConfirmNewFolder={confirmNewFolder}
      renameDialog={renameDialog}
      renameValueInput={renameValueInput}
      setRenameValueInput={setRenameValueInput}
      onDismissRename={() => setRenameDialog(null)}
      onConfirmRename={confirmRename}
      renameFolderDialog={renameFolderDialog}
      renameFolderNameInput={renameFolderNameInput}
      setRenameFolderNameInput={setRenameFolderNameInput}
      onDismissRenameFolder={() => setRenameFolderDialog(null)}
      onConfirmRenameFolder={confirmRenameFolder}
    />
  );

  return {
    setPreferredNewMarkdownDir,
    fileTreeFileOps,
    newMarkdownToolbar,
    newFolderToolbar,
    onRenameTabFromBar,
    resetWorkspaceFileCommands,
    fileCommandModals,
  };
}
