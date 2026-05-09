import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import i18n from "../i18n";
import type { TreeNode } from "../components/FileTree";

const LAST_WORKSPACE_KEY = "knowforge:lastWorkspace";

type UseWorkspaceOptions = {
  canChangeWorkspace: () => Promise<boolean>;
  onWorkspacePicked: () => void;
  onWorkspaceOpened: () => void;
  onWorkspaceOpenFailed: () => void;
};

export function useWorkspace({
  canChangeWorkspace,
  onWorkspacePicked,
  onWorkspaceOpened,
  onWorkspaceOpenFailed,
}: UseWorkspaceOptions) {
  const [rootPath, setRootPath] = useState<string | null>(null);
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [folderLoadError, setFolderLoadError] = useState<string | null>(null);

  /** 启动时从 localStorage 恢复上次工作区（跨重启保留） */
  useEffect(() => {
    const saved = localStorage.getItem(LAST_WORKSPACE_KEY);
    if (!saved) {
      return;
    }
    try {
      invoke<TreeNode[]>("open_workspace", { root: saved })
        .then((nodes) => {
          setRootPath(saved);
          setTree(nodes);
          setFolderLoadError(null);
          onWorkspaceOpened();
        })
        .catch(() => {
          setFolderLoadError(i18n.t("errors.workspaceGone"));
          onWorkspaceOpenFailed();
        });
    } catch {
      onWorkspaceOpenFailed();
    }
  }, [onWorkspaceOpened, onWorkspaceOpenFailed]);

  const pickFolder = useCallback(async () => {
    if (!(await canChangeWorkspace())) {
      return;
    }

    setFolderLoadError(null);
    const choice = await open({ directory: true, multiple: false });
    if (choice == null || Array.isArray(choice)) {
      return;
    }

    try {
      const nodes = await invoke<TreeNode[]>("open_workspace", { root: choice });
      // 仅在后端确认打开成功后再重置文档区，避免 invoke 失败时已清空标签却未换新根路径
      onWorkspacePicked();
      setRootPath(choice);
      setTree(nodes);
      setFolderLoadError(null);
      localStorage.setItem(LAST_WORKSPACE_KEY, choice);
      onWorkspaceOpened();
    } catch (e) {
      setFolderLoadError(e instanceof Error ? e.message : String(e));
      onWorkspaceOpenFailed();
    }
  }, [canChangeWorkspace, onWorkspaceOpenFailed, onWorkspaceOpened, onWorkspacePicked]);

  /** 重新扫描当前工作区 Markdown 树；成功返回最新节点供调用方立即使用（不必等 React 重渲染） */
  const refreshTree = useCallback(async (): Promise<TreeNode[] | null> => {
    try {
      const nodes = await invoke<TreeNode[]>("refresh_md_tree");
      setTree(nodes);
      setFolderLoadError(null);
      return nodes;
    } catch (e) {
      setFolderLoadError(e instanceof Error ? e.message : String(e));
      return null;
    }
  }, []);

  return {
    folderLoadError,
    pickFolder,
    refreshTree,
    rootPath,
    tree,
  };
}
