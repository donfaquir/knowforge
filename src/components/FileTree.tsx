import type { ButtonHTMLAttributes } from "react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { KF_PRIVATE_LOCK_ICON_TAB_PX } from "../constants/kfPrivateUi";
import { DirChevronIcon, FolderBulkToggleIcon } from "./treeCollapseIcons";
import { KfPrivateLockIcon } from "./KfPrivateLockIcon";

/** 与 Rust `TreeNode` 序列化字段一致 */
export type TreeNode = {
  name: string;
  rel_path: string;
  children?: TreeNode[];
  /** 仅叶子：Rust `build_md_tree` 由文件头解析 */
  kfPrivate?: boolean;
  /** 若后端以 snake 序列化（旧构建），兼容读取 */
  kf_private?: boolean;
};

/** 从树节点载荷读取是否私密（磁盘快照）。 */
export function treeNodeKfPrivateFromPayload(n: TreeNode): boolean {
  if (n.kfPrivate === true) {
    return true;
  }
  return n.kf_private === true;
}

/** 与 file-tree__bulk 内「全部展开/折叠」同排的新建入口 */
export type FileTreeNewMarkdownAction = {
  onClick: () => void;
  disabled: boolean;
  busy: boolean;
  dragExcludeProps?: ButtonHTMLAttributes<HTMLButtonElement>;
  /** 覆盖工具栏新建按钮的 title */
  tooltipTitle?: string;
};

export type FileTreeNewFolderAction = {
  onClick: () => void;
  disabled: boolean;
  busy: boolean;
  dragExcludeProps?: ButtonHTMLAttributes<HTMLButtonElement>;
  tooltipTitle?: string;
};

/** 右键：在目录新建 / 重命名、删除文件与文件夹 */
/** 收集树中 `kfPrivate === true` 的 Markdown 路径（与 Rust `TreeNode.kfPrivate` 一致）。 */
export function collectKfPrivateRelPaths(nodes: TreeNode[]): Set<string> {
  const out = new Set<string>();
  const walk = (list: TreeNode[]) => {
    for (const n of list) {
      if (n.children != null) {
        walk(n.children);
      } else if (treeNodeKfPrivateFromPayload(n)) {
        out.add(n.rel_path);
      }
    }
  };
  walk(nodes);
  return out;
}

export type FileTreeFileOps = {
  onNewInDirectory: (dirRel: string) => void;
  onNewFolderInDirectory: (dirRel: string) => void;
  onRenameFile: (relPath: string) => void;
  onDeleteFile: (relPath: string) => void;
  /** 仅当目录树中无任何普通文件时由后端允许删除 */
  onDeleteFolder: (dirRel: string) => void;
  onRenameFolder: (dirRel: string) => void;
};

/** 右键菜单定位时与视口边缘的间隙 */
const FILE_TREE_CTX_MENU_VIEWPORT_MARGIN_PX = 4;

type Props = {
  nodes: TreeNode[];
  selectedPath: string | null;
  onSelectFile: (relPath: string) => void;
  newMarkdownAction?: FileTreeNewMarkdownAction;
  newFolderAction?: FileTreeNewFolderAction;
  fileOps?: FileTreeFileOps | null;
  /** 与顶栏标签一致：合并当前缓冲区 frontmatter + 树快照；传入后侧栏锁不依赖单独 refresh 树 */
  isKfPrivate?: (relPath: string) => boolean;
  /** Tauri：避免拖整块窗口时误触「定位当前文件」按钮 */
  revealActiveDragExcludeProps?: ButtonHTMLAttributes<HTMLButtonElement>;
};

type ContextMenuState = {
  x: number;
  y: number;
  payload: { kind: "root" } | { kind: "dir"; relPath: string } | { kind: "file"; relPath: string };
};

/** 收集树中所有目录的 rel_path */
function collectDirPaths(nodes: TreeNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    if (n.children != null) {
      out.push(n.rel_path);
      collectDirPaths(n.children, out);
    }
  }
  return out;
}

function normRelPath(p: string): string {
  return p.replace(/\\/g, "/").replace(/^\/+/, "");
}

/** 当前打开文件相对路径在树中是否存在（叶子 .md） */
function isLeafRelPathInTree(nodes: TreeNode[], relPath: string): boolean {
  const want = normRelPath(relPath);
  for (const n of nodes) {
    if (n.children != null) {
      if (isLeafRelPathInTree(n.children, relPath)) {
        return true;
      }
    } else if (normRelPath(n.rel_path) === want) {
      return true;
    }
  }
  return false;
}

/** 与树节点一致的 rel_path（用于 data 属性与选择器） */
function findTreeRelPathForFile(nodes: TreeNode[], relPath: string): string | null {
  const want = normRelPath(relPath);
  for (const n of nodes) {
    if (n.children != null) {
      const hit = findTreeRelPathForFile(n.children, relPath);
      if (hit) {
        return hit;
      }
    } else if (normRelPath(n.rel_path) === want) {
      return n.rel_path;
    }
  }
  return null;
}

/** 从文件 rel_path 得到需展开的祖先目录 rel_path（不含文件本身） */
function ancestorDirRelPathsForFile(fileRelPath: string): string[] {
  const norm = normRelPath(fileRelPath);
  const parts = norm.split("/").filter((p) => p.length > 0);
  if (parts.length <= 1) {
    return [];
  }
  const out: string[] = [];
  for (let i = 0; i < parts.length - 1; i++) {
    out.push(parts.slice(0, i + 1).join("/"));
  }
  return out;
}

/**
 * 将值嵌入 `[data-file-tree-path="…"]` 双引号属性选择器（CSS 字符串 token 语义）。
 * 转义 `\`、`"` 及控制字符（十六进制 `\` + 码点 + 空格 终止）；`.#:[]` 在引号内无需转义即可匹配。
 * 不使用 `CSS.escape`：其面向「标识符」，用于属性值会误转义常见路径字符。
 */
function escapeCssDoubleQuotedAttributeValue(s: string): string {
  let out = "";
  for (const ch of s) {
    if (ch === "\\") {
      out += "\\\\";
      continue;
    }
    if (ch === '"') {
      out += '\\"';
      continue;
    }
    const cp = ch.codePointAt(0)!;
    if (cp < 0x20 || cp === 0x7f) {
      out += `\\${cp.toString(16)} `;
      continue;
    }
    out += ch;
  }
  return out;
}

function escapeFileTreePathForSelector(relPath: string): string {
  return escapeCssDoubleQuotedAttributeValue(relPath);
}

const REVEAL_IN_TREE_ICON_STROKE = 1.65;

function IconRevealInTree() {
  return (
    <svg
      className="file-tree__reveal-active-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={REVEAL_IN_TREE_ICON_STROKE}
      strokeLinecap="round"
      aria-hidden={true}
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2v3 M12 19v3 M2 12h3 M19 12h3" />
    </svg>
  );
}

function TreeItem({
  node,
  depth,
  selectedPath,
  onSelectFile,
  expandedDirs,
  onToggleDir,
  onDirContextMenu,
  onDirLabelDoubleClick,
  onFileContextMenu,
  isKfPrivate,
}: {
  node: TreeNode;
  depth: number;
  selectedPath: string | null;
  onSelectFile: (relPath: string) => void;
  expandedDirs: Record<string, boolean>;
  onToggleDir: (relPath: string) => void;
  onDirContextMenu?: (e: MouseEvent, relPath: string) => void;
  onDirLabelDoubleClick?: (e: MouseEvent, relPath: string) => void;
  onFileContextMenu?: (e: MouseEvent, relPath: string) => void;
  isKfPrivate?: (relPath: string) => boolean;
}) {
  const { t } = useTranslation();
  const isDir = node.children != null;

  if (isDir) {
    const expanded = expandedDirs[node.rel_path] !== false;

    return (
      <li className="file-tree__item">
        <div
          className="file-tree__row file-tree__row--dir"
          style={{ paddingLeft: `${6 + depth * 14}px` }}
          onContextMenu={
            onDirContextMenu
              ? (e) => {
                  onDirContextMenu(e, node.rel_path);
                }
              : undefined
          }
        >
          <button
            type="button"
            className="file-tree__twisty"
            onClick={() => onToggleDir(node.rel_path)}
            aria-expanded={expanded}
            aria-label={
              expanded ? `${t("fileTree.collapse")} ${node.name}` : `${t("fileTree.expand")} ${node.name}`
            }
            title={expanded ? t("fileTree.collapse") : t("fileTree.expand")}
          >
            <DirChevronIcon expanded={expanded} />
          </button>
          <button
            type="button"
            className="file-tree__row-label"
            onClick={() => onToggleDir(node.rel_path)}
            onDoubleClick={
              onDirLabelDoubleClick
                ? (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    onDirLabelDoubleClick(e, node.rel_path);
                  }
                : undefined
            }
            aria-expanded={expanded}
            title={node.rel_path}
          >
            <span className="file-tree__name">{node.name}</span>
          </button>
        </div>
        {expanded && node.children && (
          <ul className="file-tree__list file-tree__list--nested">
            {node.children.map((child) => (
              <TreeItem
                key={child.rel_path}
                node={child}
                depth={depth + 1}
                selectedPath={selectedPath}
                onSelectFile={onSelectFile}
                expandedDirs={expandedDirs}
                onToggleDir={onToggleDir}
                onDirContextMenu={onDirContextMenu}
                onDirLabelDoubleClick={onDirLabelDoubleClick}
                onFileContextMenu={onFileContextMenu}
                isKfPrivate={isKfPrivate}
              />
            ))}
          </ul>
        )}
      </li>
    );
  }

  const showPrivateLock = isKfPrivate ? isKfPrivate(node.rel_path) : treeNodeKfPrivateFromPayload(node);

  const selected = selectedPath === node.rel_path;
  return (
    <li className="file-tree__item">
      <button
        type="button"
        className={`file-tree__row file-tree__row--file${selected ? " file-tree__row--selected" : ""}`}
        data-file-tree-path={node.rel_path}
        style={{ paddingLeft: `${6 + depth * 14}px` }}
        onClick={() => onSelectFile(node.rel_path)}
        title={node.rel_path}
        onContextMenu={
          onFileContextMenu
            ? (e) => {
                onFileContextMenu(e, node.rel_path);
              }
            : undefined
        }
      >
        <span className="file-tree__twisty-spacer" aria-hidden />
        {showPrivateLock ? (
          <span
            className="file-tree__private-lock"
            title={t("kfPrivate.tooltipTree")}
            aria-label={t("fileTree.privateNote")}
          >
            <KfPrivateLockIcon size={KF_PRIVATE_LOCK_ICON_TAB_PX} />
          </span>
        ) : null}
        <span className="file-tree__name">{node.name}</span>
      </button>
    </li>
  );
}

export function FileTree({
  nodes,
  selectedPath,
  onSelectFile,
  newMarkdownAction,
  newFolderAction,
  fileOps,
  isKfPrivate,
  revealActiveDragExcludeProps,
}: Props) {
  const { t } = useTranslation();
  /** rel_path -> false 表示折叠；缺省为展开 */
  const [expandedDirs, setExpandedDirs] = useState<Record<string, boolean>>({});
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);
  const ctxMenuRef = useRef<HTMLUListElement>(null);

  const dirPaths = useMemo(() => collectDirPaths(nodes), [nodes]);
  const dirCount = dirPaths.length;

  useEffect(() => {
    if (!ctxMenu) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setCtxMenu(null);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [ctxMenu]);

  useLayoutEffect(() => {
    if (!ctxMenu || !ctxMenuRef.current) {
      return;
    }
    const el = ctxMenuRef.current;
    const rect = el.getBoundingClientRect();
    const m = FILE_TREE_CTX_MENU_VIEWPORT_MARGIN_PX;
    setCtxMenu((prev) => {
      if (!prev) {
        return prev;
      }
      let x = Math.min(prev.x, window.innerWidth - rect.width - m);
      let y = Math.min(prev.y, window.innerHeight - rect.height - m);
      x = Math.max(m, x);
      y = Math.max(m, y);
      if (x === prev.x && y === prev.y) {
        return prev;
      }
      return { ...prev, x, y };
    });
  }, [ctxMenu]);

  const toggleDir = useCallback((relPath: string) => {
    setExpandedDirs((prev) => {
      const cur = prev[relPath] !== false;
      return { ...prev, [relPath]: !cur };
    });
  }, []);

  const collapseAllDirs = useCallback(() => {
    const paths = collectDirPaths(nodes);
    setExpandedDirs((prev) => {
      const next = { ...prev };
      for (const p of paths) {
        next[p] = false;
      }
      return next;
    });
  }, [nodes]);

  const expandAllDirs = useCallback(() => {
    setExpandedDirs({});
  }, []);

  const allFoldersExpanded = useMemo(() => {
    if (dirPaths.length === 0) {
      return true;
    }
    return dirPaths.every((p) => expandedDirs[p] !== false);
  }, [dirPaths, expandedDirs]);

  const toggleAllFolders = useCallback(() => {
    if (allFoldersExpanded) {
      collapseAllDirs();
    } else {
      expandAllDirs();
    }
  }, [allFoldersExpanded, collapseAllDirs, expandAllDirs]);

  const revealActiveInTree = useCallback(() => {
    if (!selectedPath || !isLeafRelPathInTree(nodes, selectedPath)) {
      return;
    }
    const treePath = findTreeRelPathForFile(nodes, selectedPath) ?? selectedPath;
    const ancestors = ancestorDirRelPathsForFile(treePath);
    setExpandedDirs((prev) => {
      const next = { ...prev };
      for (const d of ancestors) {
        next[d] = true;
      }
      return next;
    });
    const scrollToRow = () => {
      const el = document.querySelector(`[data-file-tree-path="${escapeFileTreePathForSelector(treePath)}"]`);
      el?.scrollIntoView({ block: "nearest", inline: "nearest" });
    };
    requestAnimationFrame(() => {
      requestAnimationFrame(scrollToRow);
    });
  }, [selectedPath, nodes]);

  const openDirMenu = useCallback(
    (e: MouseEvent, relPath: string) => {
      if (!fileOps) {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      setCtxMenu({ x: e.clientX, y: e.clientY, payload: { kind: "dir", relPath } });
    },
    [fileOps],
  );

  const openFileMenu = useCallback(
    (e: MouseEvent, relPath: string) => {
      if (!fileOps) {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      setCtxMenu({ x: e.clientX, y: e.clientY, payload: { kind: "file", relPath } });
    },
    [fileOps],
  );

  const onTreeBackgroundContextMenu = useCallback(
    (e: MouseEvent<HTMLDivElement>) => {
      if (!fileOps) {
        return;
      }
      const el = e.target as HTMLElement;
      if (
        el.closest(
          ".file-tree__row, .file-tree__bulk, .file-tree__new-md, .file-tree__new-folder, .file-tree__reveal-active",
        )
      ) {
        return;
      }
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY, payload: { kind: "root" } });
    },
    [fileOps],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  const showRevealInTree = nodes.length > 0;
  const revealDisabled = !selectedPath || !isLeafRelPathInTree(nodes, selectedPath);
  const showBulkToolbar =
    newMarkdownAction != null || newFolderAction != null || dirCount > 0 || showRevealInTree;

  const bulkToolbar = showBulkToolbar ? (
    <div className="file-tree__bulk">
      {newMarkdownAction ? (
        <button
          type="button"
          className="file-tree__new-md"
          {...newMarkdownAction.dragExcludeProps}
          disabled={newMarkdownAction.disabled}
          aria-busy={newMarkdownAction.busy}
          aria-label={t("fileTree.newMarkdown")}
          title={newMarkdownAction.tooltipTitle ?? t("fileTree.newMarkdown")}
          onClick={newMarkdownAction.onClick}
        >
          <svg
            className="file-tree__new-md-icon"
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
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
            <line x1="12" y1="18" x2="12" y2="12" />
            <line x1="9" y1="15" x2="15" y2="15" />
          </svg>
        </button>
      ) : null}
      {newFolderAction ? (
        <button
          type="button"
          className="file-tree__new-folder"
          {...newFolderAction.dragExcludeProps}
          disabled={newFolderAction.disabled}
          aria-busy={newFolderAction.busy}
          aria-label={t("fileTree.newFolder")}
          title={newFolderAction.tooltipTitle ?? t("fileTree.newFolder")}
          onClick={newFolderAction.onClick}
        >
          <svg
            className="file-tree__new-folder-icon"
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
            <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 2H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2Z" />
            <line x1="12" y1="9" x2="12" y2="15" />
            <line x1="9" y1="12" x2="15" y2="12" />
          </svg>
        </button>
      ) : null}
      {showRevealInTree ? (
        <button
          type="button"
          className="file-tree__reveal-active"
          {...revealActiveDragExcludeProps}
          disabled={revealDisabled}
          aria-label={t("fileTree.revealActiveInTree")}
          title={t("fileTree.revealActiveInTreeTitle")}
          onClick={revealActiveInTree}
        >
          <IconRevealInTree />
        </button>
      ) : null}
      {dirCount > 0 ? (
        <button
          type="button"
          className="file-tree__bulk-toggle"
          onClick={toggleAllFolders}
          aria-label={allFoldersExpanded ? t("fileTree.collapseAll") : t("fileTree.expandAll")}
          title={allFoldersExpanded ? t("fileTree.collapseAll") : t("fileTree.expandAll")}
        >
          <FolderBulkToggleIcon allExpanded={allFoldersExpanded} />
        </button>
      ) : null}
    </div>
  ) : null;

  const ctxMenuUi =
    ctxMenu && fileOps ? (
      <>
        <div
          className="file-tree-ctx-scrim"
          aria-hidden
          onClick={closeCtxMenu}
          onContextMenu={(e) => {
            e.preventDefault();
            closeCtxMenu();
          }}
        />
        <ul
          ref={ctxMenuRef}
          className="file-tree-ctx-menu"
          role="menu"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onMouseDown={(e) => e.stopPropagation()}
        >
          {ctxMenu.payload.kind === "root" ? (
            <>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    fileOps.onNewInDirectory("");
                  }}
                >
                  {t("fileTree.ctxNewMarkdown")}
                </button>
              </li>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    fileOps.onNewFolderInDirectory("");
                  }}
                >
                  {t("fileTree.ctxNewFolder")}
                </button>
              </li>
            </>
          ) : null}
          {ctxMenu.payload.kind === "dir" ? (
            <>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "dir") {
                      fileOps.onNewInDirectory(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxNewMarkdown")}
                </button>
              </li>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "dir") {
                      fileOps.onNewFolderInDirectory(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxNewFolder")}
                </button>
              </li>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "dir") {
                      void fileOps.onRenameFolder(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxRename")}
                </button>
              </li>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item file-tree-ctx-menu__item--danger"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "dir") {
                      void fileOps.onDeleteFolder(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxDelete")}
                </button>
              </li>
            </>
          ) : null}
          {ctxMenu.payload.kind === "file" ? (
            <>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "file") {
                      fileOps.onRenameFile(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxRename")}
                </button>
              </li>
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="file-tree-ctx-menu__item file-tree-ctx-menu__item--danger"
                  onClick={() => {
                    closeCtxMenu();
                    const p = ctxMenu.payload;
                    if (p.kind === "file") {
                      fileOps.onDeleteFile(p.relPath);
                    }
                  }}
                >
                  {t("fileTree.ctxDelete")}
                </button>
              </li>
            </>
          ) : null}
        </ul>
      </>
    ) : null;

  if (nodes.length === 0) {
    return (
      <div className="file-tree" onContextMenu={onTreeBackgroundContextMenu}>
        {bulkToolbar}
        <p className="file-tree__empty">{t("fileTree.empty")}</p>
        {ctxMenuUi}
      </div>
    );
  }

  return (
    <div className="file-tree" onContextMenu={onTreeBackgroundContextMenu}>
      {bulkToolbar}
      <ul className="file-tree__list">
        {nodes.map((node) => (
          <TreeItem
            key={node.rel_path}
            node={node}
            depth={0}
            selectedPath={selectedPath}
            onSelectFile={onSelectFile}
            expandedDirs={expandedDirs}
            onToggleDir={toggleDir}
            onDirContextMenu={fileOps ? openDirMenu : undefined}
            onDirLabelDoubleClick={fileOps ? openDirMenu : undefined}
            onFileContextMenu={fileOps ? openFileMenu : undefined}
            isKfPrivate={isKfPrivate}
          />
        ))}
      </ul>
      {ctxMenuUi}
    </div>
  );
}
