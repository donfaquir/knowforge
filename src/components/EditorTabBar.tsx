import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { KF_PRIVATE_LOCK_ICON_TAB_PX } from "../constants/kfPrivateUi";
import { KfPrivateLockIcon } from "./KfPrivateLockIcon";

/** 与下方 Markdown 面板的 id 一致，标签用 aria-controls 指向此处 */
export const MARKDOWN_TAB_PANEL_ID = "markdown-tabpanel";

/** 右键菜单贴边时与视口保留的间隙（仅用于定位钳位，与 CSS padding 无关） */
const TAB_CONTEXT_MENU_VIEWPORT_MARGIN_PX = 4;

/** 每个标签的稳定 DOM id，面板用 aria-labelledby 指向当前激活标签 */
export function editorTabDomId(relPath: string): string {
  return `editor-tab-${encodeURIComponent(relPath).replace(/%/g, "_")}`;
}

type ContextMenuState = {
  x: number;
  y: number;
  relPath: string;
} | null;

type Props = {
  tabs: string[];
  activePath: string | null;
  isDirty: (relPath: string) => boolean;
  /** 磁盘已变且该标签有未保存编辑时（与主栏横幅一致） */
  hasDiskStaleConflict?: (relPath: string) => boolean;
  onSelect: (relPath: string) => void;
  onClose: (relPath: string) => void;
  onCloseAll: () => void;
  /** 标签右键 / 双指轻点菜单：重命名（与文件树一致，打开应用层重命名对话框） */
  onRenameTab?: (relPath: string) => void;
  /** macOS 顶栏拖拽区：避免标签按钮触发拖动窗口 */
  tauriDragExclude?: boolean;
  /** `kf-private`：磁盘树或当前缓冲区判定为私密时显示锁 */
  isKfPrivate?: (relPath: string) => boolean;
};

function tabTitle(relPath: string): string {
  const i = relPath.lastIndexOf("/");
  return i >= 0 ? relPath.slice(i + 1) : relPath;
}

export function EditorTabBar({
  tabs,
  activePath,
  isDirty,
  hasDiskStaleConflict,
  onSelect,
  onClose,
  onCloseAll,
  onRenameTab,
  tauriDragExclude,
  isKfPrivate,
}: Props) {
  const { t } = useTranslation();
  const excludeDrag = tauriDragExclude
    ? ({ "data-tauri-drag-region-exclude": true } as const)
    : {};
  const tabButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  /** 待执行的 focus rAF，卸载或再次调度时取消，避免已卸载节点 focus 或陈旧回调 */
  const focusTabRafRef = useRef<number | null>(null);
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const closeMenu = useCallback(() => setCtxMenu(null), []);

  useEffect(
    () => () => {
      if (focusTabRafRef.current != null) {
        cancelAnimationFrame(focusTabRafRef.current);
        focusTabRafRef.current = null;
      }
    },
    [],
  );

  // 点击菜单外部关闭
  useEffect(() => {
    if (!ctxMenu) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    // 用 capture 保证先于其他 handler
    document.addEventListener("mousedown", onDown, true);
    return () => document.removeEventListener("mousedown", onDown, true);
  }, [ctxMenu]);

  // Escape 关闭
  useEffect(() => {
    if (!ctxMenu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCtxMenu(null);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [ctxMenu]);

  // 菜单实际渲染后按真实宽高钳位到视口，避免硬编码高度/宽度在缩放或样式变更后失效
  useLayoutEffect(() => {
    if (!ctxMenu || !menuRef.current) {
      return;
    }
    const el = menuRef.current;
    const rect = el.getBoundingClientRect();
    const m = TAB_CONTEXT_MENU_VIEWPORT_MARGIN_PX;
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

  const openCtxMenu = useCallback(
    (e: { clientX: number; clientY: number; preventDefault: () => void; stopPropagation: () => void }, relPath: string) => {
      e.preventDefault();
      e.stopPropagation();
      setCtxMenu({ x: e.clientX, y: e.clientY, relPath });
    },
    [],
  );

  const focusTab = useCallback((relPath: string) => {
    if (focusTabRafRef.current != null) {
      cancelAnimationFrame(focusTabRafRef.current);
    }
    focusTabRafRef.current = requestAnimationFrame(() => {
      focusTabRafRef.current = null;
      tabButtonRefs.current[relPath]?.focus();
    });
  }, []);

  if (tabs.length === 0) {
    return <div className="editor-tab-bar editor-tab-bar--empty" aria-hidden={true} />;
  }

  return (
    <div className="editor-tab-bar" role="tablist" aria-label={t("tabs.ariaLabel")}>
      <div className="editor-tab-bar__scroll">
        {tabs.map((relPath, tabIndex) => {
          const active = relPath === activePath;
          const dirty = isDirty(relPath);
          const diskStale = hasDiskStaleConflict?.(relPath) ?? false;
          const tabId = editorTabDomId(relPath);
          return (
            <div
              key={relPath}
              className={`editor-tab${active ? " editor-tab--active" : ""}${dirty ? " editor-tab--dirty" : ""}${diskStale ? " editor-tab--disk-stale" : ""}`}
            >
              <button
                type="button"
                className="editor-tab__label"
                id={tabId}
                role="tab"
                aria-selected={active}
                aria-controls={MARKDOWN_TAB_PANEL_ID}
                tabIndex={active ? 0 : -1}
                {...excludeDrag}
                ref={(node) => {
                  tabButtonRefs.current[relPath] = node;
                }}
                onClick={() => onSelect(relPath)}
                onDoubleClick={(e) => openCtxMenu(e, relPath)}
                onContextMenu={(e) => openCtxMenu(e, relPath)}
                onKeyDown={(event) => {
                  const currentIndex = tabIndex;

                  if (event.key === "ArrowRight") {
                    event.preventDefault();
                    const nextPath = tabs[(currentIndex + 1) % tabs.length];
                    onSelect(nextPath);
                    focusTab(nextPath);
                  } else if (event.key === "ArrowLeft") {
                    event.preventDefault();
                    const nextPath = tabs[(currentIndex - 1 + tabs.length) % tabs.length];
                    onSelect(nextPath);
                    focusTab(nextPath);
                  } else if (event.key === "Home") {
                    event.preventDefault();
                    onSelect(tabs[0]);
                    focusTab(tabs[0]);
                  } else if (event.key === "End") {
                    event.preventDefault();
                    onSelect(tabs[tabs.length - 1]);
                    focusTab(tabs[tabs.length - 1]);
                  }
                }}
                title={
                  diskStale ? t("tabs.diskStaleTitle", { path: relPath }) : relPath
                }
              >
                <span className="editor-tab__name">{tabTitle(relPath)}</span>
                {isKfPrivate?.(relPath) ? (
                  <span
                    className="editor-tab__lock"
                    title={t("kfPrivate.tooltipTab")}
                    aria-label={t("fileTree.privateNote")}
                  >
                    <KfPrivateLockIcon size={KF_PRIVATE_LOCK_ICON_TAB_PX} />
                  </span>
                ) : null}
                {diskStale ? <span className="editor-tab__disk-dot" aria-hidden /> : null}
                {dirty ? <span className="editor-tab__dot" aria-hidden /> : null}
              </button>
              <button
                type="button"
                className="editor-tab__close"
                {...excludeDrag}
                onClick={(e) => {
                  e.stopPropagation();
                  onClose(relPath);
                }}
                aria-label={t("tabs.closeNamed", { name: tabTitle(relPath) })}
                title={t("toolbar.close")}
              >
                ×
              </button>
            </div>
          );
        })}
      </div>
      {ctxMenu && (
        <div
          ref={menuRef}
          className="tab-context-menu"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          role="menu"
        >
          {onRenameTab ? (
            <button
              type="button"
              className="tab-context-menu__item"
              role="menuitem"
              onClick={() => {
                const p = ctxMenu.relPath;
                closeMenu();
                onRenameTab(p);
              }}
            >
              {t("tabs.rename")}
            </button>
          ) : null}
          <button
            type="button"
            className="tab-context-menu__item"
            role="menuitem"
            onClick={() => {
              closeMenu();
              onClose(ctxMenu.relPath);
            }}
          >
            {t("tabs.closeTab")}
          </button>
          <button
            type="button"
            className="tab-context-menu__item"
            role="menuitem"
            onClick={() => {
              closeMenu();
              onCloseAll();
            }}
          >
            {t("tabs.closeAll")}
          </button>
        </div>
      )}
    </div>
  );
}
