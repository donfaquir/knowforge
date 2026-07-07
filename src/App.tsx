import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import GithubSlugger from "github-slugger";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
} from "react";
import { useTranslation } from "react-i18next";
import { AiNoteContextProvider } from "./contexts/AiNoteContext";
import { AiConversationSessionProvider } from "./contexts/AiConversationSessionContext";
import { ThoughtMgmtAiConversationSessionProvider } from "./contexts/ThoughtMgmtAiConversationSessionContext";
import type { DepthMode } from "./types/cognitiveTypes";
import { AiConversationPanel } from "./components/AiConversationPanel";
import { AiConversationToolbar } from "./components/AiConversationToolbar";
const CrepeMarkdownEditor = lazy(() => import("./components/CrepeMarkdownEditor"));
const AiLlmSettingsModal = lazy(() => import("./components/AiLlmSettingsModal"));
import { EditorTabBar, MARKDOWN_TAB_PANEL_ID, editorTabDomId } from "./components/EditorTabBar";
import { FileTree, collectKfPrivateRelPaths } from "./components/FileTree";
import { KfPrivateLockIcon } from "./components/KfPrivateLockIcon";
import { OutlineBulkToolbar } from "./components/OutlineBulkToolbar";
import { OutlinePanel } from "./components/OutlinePanel";
import type { CrepeMarkdownEditorApi } from "./components/CrepeMarkdownEditor";
import { CognitiveReportPanel } from "./components/CognitiveReportPanel";
import { CommandPalette } from "./components/CommandPalette";
import { EditorThoughtsPanel } from "./components/EditorThoughtsPanel";
import { EditorWritingCoachHost, type EditorWritingCoachHostHandle } from "./components/EditorWritingCoachHost";
import { RightPanelReviewTab } from "./components/RightPanelReviewTab";
import { LinkRecommendationPanel } from "./components/LinkRecommendationPanel";
import { RightPanelShell, type RightPanelTab } from "./components/RightPanelShell";
import { ThoughtMaturityToastHost } from "./components/ThoughtMaturityToastHost";
import { ThoughtManagementPanel } from "./components/ThoughtManagementPanel";
import { ThoughtSavePopover } from "./components/ThoughtSavePopover";
import { ThoughtVaultHubModal } from "./components/ThoughtVaultHubModal";
import { WorkspaceSearchModal } from "./components/WorkspaceSearchModal";
import { EditorFindBar } from "./components/EditorFindBar";
import { GraphTabShell } from "./components/GraphTabShell";
import { ActivityBar, type LeftPanelView } from "./components/ActivityBar";
import { KF_PRIVATE_LOCK_ICON_DOC_BAR_PX } from "./constants/kfPrivateUi";
import { useKfPrivateForPath } from "./hooks/useKfPrivateForPath";
import { useOpenDocs } from "./hooks/useOpenDocs";
import { useOutline } from "./hooks/useOutline";
import { useOutlineFoldModel } from "./hooks/useOutlineFoldModel";
import { useResizable } from "./hooks/useResizable";
import { useRightPanelMaxWidthPx } from "./hooks/useRightPanelMaxWidth";
import { useWorkspace } from "./hooks/useWorkspace";
import { flattenMarkdownTreeForWikiSuggest } from "./utils/flattenMarkdownTreeForWikiSuggest";
import { useReviewDueTabBadge } from "./hooks/useReviewDueTabBadge";
import { useWorkspaceFileCommands } from "./hooks/useWorkspaceFileCommands";
import { getMarkdownBodyForEditor, setMarkdownKfPrivate } from "./utils/kfPrivateFrontmatterEdit";
import { resolveWikiHeadingSlug } from "./utils/resolveWikiHeadingSlug";
import { markdownTreatAsKfPrivateForUi } from "./utils/kfPrivateMarkdown";
import { parentDirOfRelPath } from "./utils/newUntitledMarkdownPath";
import { endPerfTrace, logPerfMark, startPerfTrace, type PerfTrace } from "./utils/perfTrace";
import { OPEN_AI_SETTINGS_EVENT } from "./utils/vaultConfigBroadcast";
import "./App.css";

/** 与会话恢复配套：须与 knowforge:lastWorkspace 指向同一工作区根路径 */
const LAST_SESSION_KEY = "knowforge:lastSession";

/** Wikilink `#` 标题定位：等 ProseMirror 挂载的最长等待（毫秒）；弱设备/大文档下避免无限 rAF */
const WIKI_HEADING_NAV_RETRY_BUDGET_MS = 2500;

const PM_HEADING_SELECTOR =
  ".ProseMirror h1, .ProseMirror h2, .ProseMirror h3, .ProseMirror h4, .ProseMirror h5, .ProseMirror h6";

/** 在 Milkdown 滚动容器内按 GitHub slug 查找标题 DOM（与 extractOutline / navigateToHeading 一致） */
function findProseMirrorHeadingBySlug(
  scrollEl: HTMLElement | null,
  slug: string,
): HTMLElement | null {
  if (!scrollEl) {
    return null;
  }
  const slugger = new GithubSlugger();
  const headings = scrollEl.querySelectorAll(PM_HEADING_SELECTOR);
  for (const heading of headings) {
    if (!(heading instanceof HTMLElement)) {
      continue;
    }
    const text = heading.textContent?.trim() ?? "";
    if (slugger.slug(text) === slug) {
      return heading;
    }
  }
  return null;
}

function scrollMilkdownHeadingIntoView(scrollEl: HTMLElement, headingEl: HTMLElement) {
  const pad = 12;
  const cRect = scrollEl.getBoundingClientRect();
  const eRect = headingEl.getBoundingClientRect();
  const top = scrollEl.scrollTop + (eRect.top - cRect.top) - pad;
  scrollEl.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
}

/** 与 src-tauri 中 rel_path_components_ok 规则一致，禁止空段、.、.. */
function isValidStoredRelPath(relPath: string): boolean {
  if (relPath.length === 0) {
    return false;
  }
  for (const part of relPath.split("/")) {
    if (part === "" || part === "." || part === "..") {
      return false;
    }
  }
  return true;
}

/** 校验 localStorage 中 lastSession 的形状与路径，损坏或篡改则返回 null */
function parseStoredEditorSession(raw: string, rootPath: string): { activeRel: string } | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }
  const o = parsed as Record<string, unknown>;
  if (typeof o.workspace !== "string" || typeof o.activeRel !== "string") {
    return null;
  }
  if (o.workspace !== rootPath || !isValidStoredRelPath(o.activeRel)) {
    return null;
  }
  return { activeRel: o.activeRel };
}

/** 标题栏拖拽/双击最大化：与可交互控件及 Tauri exclude 区域一致 */
function isDragExcludedTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) {
    return false;
  }
  return (
    target.closest(
      [
        "button",
        "a",
        "input",
        "textarea",
        "select",
        "summary",
        "[role='button']",
        "[role='tab']",
        "[contenteditable='true']",
        "[data-tauri-drag-region-exclude]",
      ].join(","),
    ) != null
  );
}

function App() {
  const { t } = useTranslation();
  const [rightPanelOpen, setRightPanelOpen] = useState(true);
  const [rightPanelTab, setRightPanelTab] = useState<RightPanelTab>("outline");
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [leftPanelView, setLeftPanelView] = useState<LeftPanelView>("files");
  const [aiSettingsOpen, setAiSettingsOpen] = useState(false);
  const [workspaceReady, setWorkspaceReady] = useState(false);
  const [initialDepthMode, setInitialDepthMode] = useState<DepthMode | undefined>(undefined);
  /** 中间栏：查看/编辑原始 Markdown（整篇含 frontmatter）；切换标签或活动文档时复位为预览 */
  const [showMarkdownSource, setShowMarkdownSource] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [cognitiveReportOpen, setCognitiveReportOpen] = useState(false);
  const [thoughtVaultHubOpen, setThoughtVaultHubOpen] = useState(false);
  const [thoughtMgmtBodyDirty, setThoughtMgmtBodyDirty] = useState(false);
  const [editorSaveThoughtText, setEditorSaveThoughtText] = useState<string | null>(null);
  const [workspaceSearchOpen, setWorkspaceSearchOpen] = useState(false);
  /** 同标签重复点全文搜索结果时仍触发预览滚动 */
  const [workspaceSearchGotoEpoch, setWorkspaceSearchGotoEpoch] = useState(0);
  const [editorFindOpen, setEditorFindOpen] = useState(false);
  /** 全文搜索跳转后注入篇内查找的关键词与大小写 */
  const [editorFindWorkspaceSeed, setEditorFindWorkspaceSeed] = useState<{
    query: string;
    caseSensitive: boolean;
    nonce: number;
  } | null>(null);
  const editorFindWorkspaceSeedNonceRef = useRef(0);
  const leftResizable = useResizable({ side: "left", defaultWidth: 196, minWidth: 150, maxWidth: 500 });
  const rightPanelFillRemainder = rightPanelTab === "ai";
  const rightPanelMaxWidth = useRightPanelMaxWidthPx({
    fillRemainder: rightPanelFillRemainder,
    leftSidebarPx: sidebarOpen ? leftResizable.width : 36,
  });
  const rightResizable = useResizable({
    side: "right",
    defaultWidth: 364,
    minWidth: 280,
    maxWidth: rightPanelMaxWidth,
  });
  const anyDragging = leftResizable.isDragging || rightResizable.isDragging;

  const editorScrollRef = useRef<HTMLDivElement>(null);
  /** 递增代数：新一次 wikilink 标题定位或卸载时作废仍在排队的 rAF */
  const wikiHeadingNavRetryGenerationRef = useRef(0);
  const rawSourceTextareaRef = useRef<HTMLTextAreaElement>(null);
  const crepeEditorApiRef = useRef<CrepeMarkdownEditorApi | null>(null);
  const writingCoachRef = useRef<EditorWritingCoachHostHandle>(null);
  /** 全文搜索打开笔记后：待预览态滚到近似命中行 */
  const pendingWorkspaceSearchGotoRef = useRef<{ relPath: string; line: number } | null>(null);
  const docState = useOpenDocs(workspaceReady);
  const activePathSwitchTraceRef = useRef<PerfTrace | null>(null);
  const markdownBodyCacheRef = useRef<Map<string, string>>(new Map());
  const flushDirtyBeforeExitRef = useRef(docState.flushDirtyDocumentsBeforeExit);
  flushDirtyBeforeExitRef.current = docState.flushDirtyDocumentsBeforeExit;
  const openOrFocusRef = useRef(docState.openOrFocusTab);
  openOrFocusRef.current = docState.openOrFocusTab;

  const getCachedMarkdownBodyForEditor = useCallback((content: string): string => {
    const cache = markdownBodyCacheRef.current;
    if (cache.has(content)) {
      const cached = cache.get(content)!;
      cache.delete(content);
      cache.set(content, cached);
      return cached;
    }
    const body = getMarkdownBodyForEditor(content);
    cache.set(content, body);
    while (cache.size > 24) {
      const oldestKey = cache.keys().next().value;
      if (oldestKey == null) break;
      cache.delete(oldestKey);
    }
    return body;
  }, []);

  useEffect(() => {
    const trace = activePathSwitchTraceRef.current;
    if (!trace || trace.meta?.to !== docState.activePath) {
      return;
    }
    activePathSwitchTraceRef.current = null;
    const frame = window.requestAnimationFrame(() => {
      endPerfTrace(trace, { phase: "next_frame" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [docState.activePath]);

  /** useWorkspace 早于 useWorkspaceFileCommands 创建，通过 ref 调用后者提供的 reset */
  const resetWorkspaceFileCommandsRef = useRef<() => void>(() => {});

  const onWorkspacePicked = useCallback(() => {
    docState.resetOpenDocs();
    setWorkspaceReady(false);
    resetWorkspaceFileCommandsRef.current();
    setSidebarOpen((open) => open || true);
  }, [docState.resetOpenDocs]);

  const onWorkspaceOpened = useCallback(() => setWorkspaceReady(true), []);
  const onWorkspaceOpenFailed = useCallback(() => setWorkspaceReady(false), []);

  const { folderLoadError, pickFolder, refreshTree, rootPath, tree } = useWorkspace({
    canChangeWorkspace: docState.confirmDiscardAllTabs,
    onWorkspacePicked,
    onWorkspaceOpened,
    onWorkspaceOpenFailed,
  });

  const kfPrivatePathsFromTree = useMemo(() => collectKfPrivateRelPaths(tree), [tree]);
  const wikiSuggestFiles = useMemo(() => flattenMarkdownTreeForWikiSuggest(tree), [tree]);

  const isPathKfPrivate = useKfPrivateForPath(docState.docByPath, kfPrivatePathsFromTree);

  const tauriRuntime = isTauri();

  const { dueCount: reviewDueTabCount } = useReviewDueTabBadge({
    workspaceReady,
    tauriRuntime,
    workspaceRoot: rootPath,
  });

  /**
   * 离开「回顾」标签后本会话内不再显示角标（换工作区重置）。
   * 必须在渲染阶段同步置位：若仅用 useEffect 在 tab 已非 review 后才 setState，
   * 会多出一帧 dismissed 仍为 false，角标会按 totalDue 闪一下。
   */
  const reviewBadgeAfterVisitSuppressedRef = useRef(false);
  const lastRootPathForReviewBadgeRef = useRef(rootPath);
  if (lastRootPathForReviewBadgeRef.current !== rootPath) {
    reviewBadgeAfterVisitSuppressedRef.current = false;
    lastRootPathForReviewBadgeRef.current = rootPath;
  }

  const prevRightPanelTabForBadgeRef = useRef<RightPanelTab>(rightPanelTab);
  if (prevRightPanelTabForBadgeRef.current === "review" && rightPanelTab !== "review") {
    reviewBadgeAfterVisitSuppressedRef.current = true;
  }

  useLayoutEffect(() => {
    prevRightPanelTabForBadgeRef.current = rightPanelTab;
  }, [rightPanelTab]);

  const reviewTabBadgeCount = useMemo(() => {
    if (reviewBadgeAfterVisitSuppressedRef.current || rightPanelTab === "review" || reviewDueTabCount <= 0) {
      return null;
    }
    return reviewDueTabCount;
  }, [reviewDueTabCount, rightPanelTab, rootPath]);

  const requestOpenChallengeReview = useCallback(() => {
    setLeftPanelView("files");
    setRightPanelOpen(true);
    setRightPanelTab("review");
  }, []);

  const {
    setPreferredNewMarkdownDir,
    fileTreeFileOps,
    newMarkdownToolbar,
    newFolderToolbar,
    onRenameTabFromBar,
    resetWorkspaceFileCommands,
    fileCommandModals,
  } = useWorkspaceFileCommands({
    workspaceReady,
    rootPath,
    refreshTree,
    docState,
    activePath: docState.activePath,
    tauriRuntime,
    onStartChallengeReview: requestOpenChallengeReview,
  });

  resetWorkspaceFileCommandsRef.current = resetWorkspaceFileCommands;

  useEffect(() => {
    const onOpenAiSettings = () => setAiSettingsOpen(true);
    window.addEventListener(OPEN_AI_SETTINGS_EVENT, onOpenAiSettings);
    return () => window.removeEventListener(OPEN_AI_SETTINGS_EVENT, onOpenAiSettings);
  }, []);

  // 工作区就绪后从 vault config 读取持久化的 depthMode
  useEffect(() => {
    if (!workspaceReady || !isTauri()) return;
    let cancelled = false;
    invoke<{ cognitive?: { depthMode?: DepthMode } }>("get_vault_config_for_ui")
      .then((cfg) => {
        if (!cancelled && cfg.cognitive?.depthMode) {
          setInitialDepthMode(cfg.cognitive.depthMode);
        }
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [workspaceReady]);

  // 工作区就绪后恢复上次在该工作区打开的文档（依赖 pending 队列处理「早于 workspaceReady 的 open」）
  useEffect(() => {
    if (!rootPath || !workspaceReady) {
      return;
    }
    const raw = localStorage.getItem(LAST_SESSION_KEY);
    if (!raw) {
      return;
    }
    const session = parseStoredEditorSession(raw, rootPath);
    if (!session) {
      return;
    }
    void openOrFocusRef.current(session.activeRel);
  }, [rootPath, workspaceReady]);

  // 仅在确有打开文件时写入，避免启动瞬间用 null 覆盖掉可恢复的会话
  useEffect(() => {
    if (!rootPath || !workspaceReady || !docState.activePath) {
      return;
    }
    try {
      localStorage.setItem(
        LAST_SESSION_KEY,
        JSON.stringify({ workspace: rootPath, activeRel: docState.activePath }),
      );
    } catch {
      /* 忽略配额或禁用存储 */
    }
  }, [rootPath, workspaceReady, docState.activePath]);

  // Tauri listen 只应在挂载时注册一次；回调里读 ref，保证始终调用最新的 applyExternalDiskChange（含 Fast Refresh / 依赖变化导致的新函数引用），无需把该方法放进 effect 依赖以免反复 unlisten/listen。
  const externalDiskHandlerRef = useRef<(relPath: string) => void>(() => {});
  externalDiskHandlerRef.current = docState.applyExternalDiskChange;

  const refreshTreeRef = useRef(refreshTree);
  refreshTreeRef.current = refreshTree;
  const treeRefreshDebounceRef = useRef<number | null>(null);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<{ relPath: string }>("markdown-disk-changed", (event) => {
      externalDiskHandlerRef.current(event.payload.relPath);
      // 外部或未打开缓冲区的文件：刷新树以更新 `kfPrivate` 磁盘快照
      if (treeRefreshDebounceRef.current != null) {
        window.clearTimeout(treeRefreshDebounceRef.current);
      }
      treeRefreshDebounceRef.current = window.setTimeout(() => {
        treeRefreshDebounceRef.current = null;
        void refreshTreeRef.current();
      }, 400);
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (treeRefreshDebounceRef.current != null) {
        window.clearTimeout(treeRefreshDebounceRef.current);
        treeRefreshDebounceRef.current = null;
      }
      unlisten?.();
    };
    // 故意为空：订阅生命周期与挂载一致；处理器经 externalDiskHandlerRef 保持最新
  }, []);

  const activeMarkdownBodyForEditor = useMemo(
    () =>
      docState.activeSession?.content != null
        ? getCachedMarkdownBodyForEditor(docState.activeSession.content)
        : undefined,
    [docState.activeSession?.content, getCachedMarkdownBodyForEditor],
  );
  /** 仅在右栏大纲可见时计算 outline，避免切文时无效 AST 解析 */
  const shouldComputeOutline = rightPanelOpen && rightPanelTab === "outline";
  const outlineState = useOutline(
    shouldComputeOutline ? docState.activePath : null,
    shouldComputeOutline ? activeMarkdownBodyForEditor : undefined,
  );
  const outlineFold = useOutlineFoldModel(outlineState.outline, docState.activePath);

  const current = docState.activeSession;
  const loadingDoc = !!current?.loading;
  const loadError = current?.loadError ?? null;

  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    if (!workspaceReady) {
      void invoke("sync_open_markdown_watchers", { relPaths: [] }).catch(() => {});
      return;
    }
    void invoke("sync_open_markdown_watchers", { relPaths: docState.tabPaths }).catch(() => {});
  }, [workspaceReady, docState.tabPaths]);

  /** 浏览器预览：关闭页面前提示未保存 */
  useEffect(() => {
    const onBeforeUnload = (e: BeforeUnloadEvent) => {
      if (docState.hasAnyDirtyTab()) {
        e.preventDefault();
      }
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [docState.hasAnyDirtyTab]);

  useEffect(() => {
    /** 切换活动文档或标签时默认回到 Markdown 预览 */
    setShowMarkdownSource(false);
  }, [docState.activePath]);

  useEffect(() => {
    setEditorFindOpen(false);
  }, [docState.activePath]);

  const toggleMarkdownSourceView = useCallback(() => {
    setShowMarkdownSource((prev) => {
      if (prev && docState.activePath) {
        const rel = docState.activePath;
        queueMicrotask(() => {
          docState.bumpContentInjectEpochForPath(rel);
        });
      }
      return !prev;
    });
  }, [docState.activePath, docState.bumpContentInjectEpochForPath]);

  /** 进入原文模式后聚焦编辑区 */
  useEffect(() => {
    if (!showMarkdownSource) {
      return;
    }
    rawSourceTextareaRef.current?.focus({ preventScroll: true });
  }, [showMarkdownSource, docState.activePath]);

  const editorReady = !!docState.activePath && !loadingDoc && !loadError;
  /** 右栏承载大纲与 AI：无打开文档时也应显示，避免分段按钮与 AI 面板被整栏卸载 */
  const showRightColumn = rightPanelOpen && workspaceReady;
  const thoughtManagementSessionActive =
    leftPanelView === "thoughts" && workspaceReady && tauriRuntime && !!rootPath;

  const changeView = useCallback(async (view: LeftPanelView) => {
    if (leftPanelView === "thoughts" && view !== "thoughts" && thoughtMgmtBodyDirty) {
      const ok = await ask(t("thoughtManagement.exitUnsavedMessage"), {
        title: t("thoughtManagement.exitUnsavedTitle"),
        kind: "warning",
      });
      if (!ok) return false;
    }
    setThoughtMgmtBodyDirty(false);
    setLeftPanelView(view);
    return true;
  }, [leftPanelView, thoughtMgmtBodyDirty, t]);

  /** 选中 Outline 但尚无可用编辑器文档时自动切到 AI，避免右栏空白且无法切换 */
  useEffect(() => {
    if (rightPanelOpen && workspaceReady && rightPanelTab === "outline" && !editorReady) {
      setRightPanelTab("ai");
    }
  }, [rightPanelOpen, workspaceReady, rightPanelTab, editorReady]);

  /** 成熟度 Toast 点击后：原文模式下滚动到指定行 */
  useEffect(() => {
    const onGoto = (ev: Event) => {
      const ce = ev as CustomEvent<{ relPath: string; line: number }>;
      const relPath = ce.detail?.relPath;
      const line = ce.detail?.line;
      if (!relPath || line == null || line < 1) {
        return;
      }
      if (docState.activePath !== relPath || !showMarkdownSource) {
        return;
      }
      const content = current?.content;
      if (!content || !rawSourceTextareaRef.current) {
        return;
      }
      const lines = content.split("\n");
      let off = 0;
      for (let i = 0; i < line - 1 && i < lines.length; i++) {
        off += lines[i].length + 1;
      }
      const ta = rawSourceTextareaRef.current;
      requestAnimationFrame(() => {
        ta.focus({ preventScroll: true });
        ta.setSelectionRange(off, off);
        const lh = 17;
        ta.scrollTop = Math.max(0, (line - 3) * lh);
      });
    };
    window.addEventListener("kf-goto-source-line", onGoto);
    return () => window.removeEventListener("kf-goto-source-line", onGoto);
  }, [docState.activePath, showMarkdownSource, current?.content]);

  /** 全文搜索：预览态下打开文件后滚到近似命中位置 */
  useEffect(() => {
    const p = pendingWorkspaceSearchGotoRef.current;
    if (!p || docState.activePath !== p.relPath) {
      return;
    }
    if (!current?.content || current.loading || loadError) {
      return;
    }
    if (showMarkdownSource) {
      return;
    }

    let cancelled = false;
    /** 始终指向「已排队的下一帧」id，否则递归 rAF 在 cleanup 时无法全部取消 */
    let rafId = 0;
    const full = current.content;
    const line = p.line;
    let attempts = 0;
    const maxAttempts = 45;

    const tick = () => {
      if (cancelled) {
        return;
      }
      const cur = pendingWorkspaceSearchGotoRef.current;
      if (!cur || cur.relPath !== p.relPath || cur.line !== p.line) {
        return;
      }
      attempts += 1;
      const api = crepeEditorApiRef.current;
      if (api?.scrollToSourceLineFromFullMarkdown(full, line)) {
        pendingWorkspaceSearchGotoRef.current = null;
        return;
      }
      if (attempts >= maxAttempts) {
        pendingWorkspaceSearchGotoRef.current = null;
        if (!cancelled) {
          const el = editorScrollRef.current;
          const totalLines = Math.max(1, full.split(/\r?\n/).length);
          if (el) {
            const denom = Math.max(1, totalLines - 1);
            const frac = Math.max(0, Math.min(1, (line - 1) / denom));
            el.scrollTop = frac * Math.max(0, el.scrollHeight - el.clientHeight);
          }
        }
        return;
      }
      rafId = requestAnimationFrame(tick);
    };

    rafId = requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      cancelAnimationFrame(rafId);
    };
  }, [docState.activePath, showMarkdownSource, workspaceSearchGotoEpoch, current?.content, current?.loading, loadError]);

  const clearEditorFindWorkspaceSeed = useCallback(() => {
    setEditorFindWorkspaceSeed(null);
  }, []);

  const handleWorkspaceSearchNavigate = useCallback(
    async (
      relPath: string,
      line: number,
      search: { query: string; caseSensitive: boolean },
    ) => {
      await docState.openOrFocusTab(relPath);
      pendingWorkspaceSearchGotoRef.current = { relPath, line };
      setShowMarkdownSource(false);
      setWorkspaceSearchGotoEpoch((n) => n + 1);
      setEditorFindOpen(true);
      const q = search.query.trim();
      if (q) {
        editorFindWorkspaceSeedNonceRef.current += 1;
        setEditorFindWorkspaceSeed({
          query: q,
          caseSensitive: search.caseSensitive,
          nonce: editorFindWorkspaceSeedNonceRef.current,
        });
      } else {
        setEditorFindWorkspaceSeed(null);
      }
    },
    [docState.openOrFocusTab],
  );

  const editorUsable =
    !!docState.activePath && !loadingDoc && !loadError && !!current;

  const workspaceReadyForShortcutRef = useRef(workspaceReady);
  const editorUsableForShortcutRef = useRef(editorUsable);
  workspaceReadyForShortcutRef.current = workspaceReady;
  editorUsableForShortcutRef.current = editorUsable;

  /**
   * 全局快捷键：单一 window keydown，避免多段 useEffect 在依赖抖动或 StrictMode 下重复注册；
   * ⌘F 条件用 ref 读最新 workspace/editor 状态，监听本身空依赖只挂载一次。
   */
  useEffect(() => {
    const inEditableField = (t: EventTarget | null) =>
      t instanceof HTMLElement && t.closest("input, textarea, select, [contenteditable='true']");

    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        return;
      }

      // ⌘L / Ctrl+L：打开侧栏并切到 AI（输入框内不触发）
      if (!e.shiftKey && (e.key === "l" || e.key === "L")) {
        if (inEditableField(e.target)) {
          return;
        }
        e.preventDefault();
        setRightPanelOpen(true);
        setRightPanelTab("ai");
        return;
      }

      // ⌘⇧P / Ctrl+Shift+P：命令面板（输入框内不触发）
      if (e.shiftKey && (e.key === "p" || e.key === "P")) {
        if (inEditableField(e.target)) {
          return;
        }
        e.preventDefault();
        setCognitiveReportOpen(false);
        setCommandPaletteOpen((o) => !o);
        return;
      }

      // ⌘⇧W / Ctrl+Shift+W：手动触发写作教练（编辑器内也需响应）
      if (e.shiftKey && (e.key === "w" || e.key === "W")) {
        if (!editorUsableForShortcutRef.current) {
          return;
        }
        e.preventDefault();
        writingCoachRef.current?.triggerManually();
        return;
      }

      // ⌘F / Ctrl+F：篇内查找（焦点在正文或原文区时）
      if (!e.shiftKey && (e.key === "f" || e.key === "F")) {
        const el = e.target;
        if (!(el instanceof HTMLElement)) {
          return;
        }
        if (el.closest("[data-editor-find-input]")) {
          return;
        }
        if (!workspaceReadyForShortcutRef.current || !editorUsableForShortcutRef.current) {
          return;
        }
        const inDoc = el.closest("[data-milkdown-root], .main__raw-doc-source, .editor-scroll__body");
        if (!inDoc) {
          return;
        }
        e.preventDefault();
        setEditorFindOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const saveDisabled =
    !workspaceReady || !editorUsable || !docState.dirty || docState.saving;

  /** 文档栏一键切换 kf-private：改缓冲区、立即落盘、刷新树以同步标签与侧栏锁标 */
  const handleKfPrivateBarToggle = useCallback(() => {
    void (async () => {
      const p = docState.activePath;
      const doc = docState.activeSession;
      if (!p || !doc || doc.loading || doc.loadError) {
        return;
      }
      const privateNow = markdownTreatAsKfPrivateForUi(doc.content);
      const wantPrivate = !privateNow;
      const next = setMarkdownKfPrivate(doc.content, wantPrivate);
      if (next === doc.content) {
        return;
      }
      docState.handleMarkdownChange(p, next, { fullDocument: true });
      docState.bumpContentInjectEpochForPath(p);
      if (isTauri() && workspaceReady) {
        const ok = await docState.persistDocument(p, "manual", { exactMarkdown: next });
        if (!ok) {
          return;
        }
      }
      if (wantPrivate) {
        window.dispatchEvent(
          new CustomEvent("kf-private-changed", { detail: { relPath: p } }),
        );
      }
      await refreshTree();
    })();
  }, [
    docState.activePath,
    docState.activeSession,
    docState.handleMarkdownChange,
    docState.bumpContentInjectEpochForPath,
    docState.persistDocument,
    workspaceReady,
    refreshTree,
  ]);

  const navigateToHeading = useCallback((slug: string) => {
    requestAnimationFrame(() => {
      const outer = editorScrollRef.current;
      const scrollEl = outer?.querySelector("[data-milkdown-root]") as HTMLElement | null;
      const el = findProseMirrorHeadingBySlug(scrollEl, slug);
      if (!(el instanceof HTMLElement) || !scrollEl) {
        return;
      }
      scrollMilkdownHeadingIntoView(scrollEl, el);
    });
  }, []);

  /** 换文注入后再定位标题（wikilink 带 # 片段） */
  const navigateToHeadingWithRetry = useCallback((slug: string) => {
    const myGen = (wikiHeadingNavRetryGenerationRef.current += 1);
    const t0 = performance.now();

    const step = () => {
      if (wikiHeadingNavRetryGenerationRef.current !== myGen) {
        return;
      }
      if (performance.now() - t0 > WIKI_HEADING_NAV_RETRY_BUDGET_MS) {
        return;
      }
      const outer = editorScrollRef.current;
      const scrollEl = outer?.querySelector("[data-milkdown-root]") as HTMLElement | null;
      const el = findProseMirrorHeadingBySlug(scrollEl, slug);
      if (el && scrollEl) {
        scrollMilkdownHeadingIntoView(scrollEl, el);
        return;
      }
      requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }, []);

  useEffect(() => {
    return () => {
      wikiHeadingNavRetryGenerationRef.current += 1;
    };
  }, []);

  const onOpenCoachMarkdownPath = useCallback(
    async (relPath: string, meta?: { headingFragment?: string | null }) => {
      const content = await docState.openOrFocusTab(relPath);
      const frag = meta?.headingFragment?.trim();
      if (!frag || content == null) {
        return;
      }
      const body = getCachedMarkdownBodyForEditor(content);
      const slug = resolveWikiHeadingSlug(body, frag);
      if (!slug) {
        return;
      }
      navigateToHeadingWithRetry(slug);
    },
    [docState.openOrFocusTab, getCachedMarkdownBodyForEditor, navigateToHeadingWithRetry],
  );

  const openCognitiveReportFromPalette = useCallback(() => {
    setCommandPaletteOpen(false);
    setCognitiveReportOpen(true);
  }, []);

  /**
   * getCurrentWindow() 每次调用都 new Window；若直接作 effect 依赖，每次 render 都会卸载并重挂
   * onCloseRequested，易与异步关闭流程竞态，导致红绿灯关闭后窗口无法 destroy。
   */
  const appWindow = useMemo(
    () => (tauriRuntime ? getCurrentWindow() : null),
    [tauriRuntime],
  );

  /** 关闭窗口前刷盘；磁盘冲突或未写入失败时拦截或二次确认 */
  useEffect(() => {
    if (!tauriRuntime || !appWindow) {
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void appWindow
      .onCloseRequested(async (event) => {
        event.preventDefault();
        try {
          const { conflictDirtyPaths, saveFailed } = await flushDirtyBeforeExitRef.current();
          if (saveFailed) {
            return;
          }
          if (conflictDirtyPaths.length > 0) {
            const ok = await ask(
              t("dialogs.closeWindowDiskConflict", { count: conflictDirtyPaths.length }),
              {
                title: t("dialogs.close"),
                kind: "warning",
              },
            );
            if (!ok) {
              return;
            }
          }
          await appWindow.destroy();
        } catch (e) {
          // 已 preventDefault：异常时必须尽力 destroy，否则窗口永远无法关闭
          console.error(e);
          try {
            await appWindow.destroy();
          } catch (e2) {
            console.error(e2);
          }
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [appWindow, tauriRuntime, t]);

  const isMacPlatform = /Mac/i.test(navigator.userAgent);
  const isWindowsPlatform = /Windows/i.test(navigator.userAgent);
  const titlebarPlatformClass = isMacPlatform
    ? " layout--platform-mac"
    : isWindowsPlatform
      ? " layout--platform-windows"
      : " layout--platform-linux";

  const tauriWindowDragProps =
    tauriRuntime && !isMacPlatform
      ? ({ "data-tauri-drag-region": true } as const)
      : {};

  const tauriDragExcludeProps =
    tauriRuntime && !isMacPlatform
      ? ({ "data-tauri-drag-region-exclude": true } as const)
      : {};

  const handleTitlebarMouseDown = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (!appWindow || event.button !== 0 || isDragExcludedTarget(event.target)) {
        return;
      }
      void appWindow.startDragging();
    },
    [appWindow],
  );

  const handleTitlebarDoubleClick = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (!appWindow || isDragExcludedTarget(event.nativeEvent.target)) {
        return;
      }
      void appWindow.toggleMaximize();
    },
    [appWindow],
  );

  const renderWindowControls = (placement: "leading" | "trailing") => {
    if (!tauriRuntime) {
      return null;
    }

    const renderMacControls = isMacPlatform && placement === "leading";
    const renderDesktopControls = !isMacPlatform && placement === "trailing";
    if (!renderMacControls && !renderDesktopControls) {
      return null;
    }

    /* macOS：系统交通灯在标题栏内（tauri.macos.conf.json：Transparent + decorations） */
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
    <AiNoteContextProvider
      workspaceReady={workspaceReady}
      activePath={docState.activePath}
      docByPath={docState.docByPath}
      openMarkdownTab={onOpenCoachMarkdownPath}
    >
    <div
      className={`layout${showRightColumn && leftPanelView === "files" ? " layout--with-right-panel" : ""}${tauriRuntime ? " layout--tauri" : ""}${sidebarOpen ? "" : " layout--sidebar-collapsed"}${titlebarPlatformClass}${anyDragging ? " layout--resizing" : ""}`}
      style={
        {
          "--sidebar-width": sidebarOpen ? `${leftResizable.width}px` : "36px",
          "--right-panel-width": showRightColumn && leftPanelView === "files" ? `${rightResizable.width}px` : undefined,
        } as React.CSSProperties
      }
    >
      {tauriRuntime && !isMacPlatform ? (
        <>
          <div
            className="window-drag-edge window-drag-edge--left"
            aria-hidden={true}
            {...tauriWindowDragProps}
          />
          <div
            className="window-drag-edge window-drag-edge--right"
            aria-hidden={true}
            {...tauriWindowDragProps}
          />
          <div
            className="window-drag-edge window-drag-edge--bottom"
            aria-hidden={true}
            {...tauriWindowDragProps}
          />
        </>
      ) : null}
      <div
        className="app-top-toolbar__banner"
        onMouseDown={handleTitlebarMouseDown}
        onDoubleClick={handleTitlebarDoubleClick}
      >
        <div className="app-top-toolbar__start" role="toolbar" aria-label={t("toolbar.files")}>
          {renderWindowControls("leading")}
          <button
            type="button"
            className={`app-top-toolbar__sidebar-toggle${sidebarOpen ? " is-active" : ""}`}
            {...tauriDragExcludeProps}
            onClick={() => setSidebarOpen((o) => !o)}
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
              onClick={() => setWorkspaceSearchOpen(true)}
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
              tabs={docState.tabPaths}
              activePath={docState.activePath}
              isDirty={docState.isDirty}
              hasDiskStaleConflict={docState.hasDiskStaleConflict}
              tauriDragExclude={tauriRuntime}
              isKfPrivate={isPathKfPrivate}
              onRenameTab={onRenameTabFromBar}
              onSelect={(p) => {
                if (p === docState.activePath && leftPanelView === "files") {
                  return;
                }
                void changeView("files").then((ok) => {
                  if (!ok) return;
                  logPerfMark("markdown.tab_switch.select", {
                    from: docState.activePath,
                    to: p,
                  });
                  activePathSwitchTraceRef.current = startPerfTrace("markdown.tab_switch.to_next_frame", {
                    from: docState.activePath,
                    to: p,
                  });
                  docState.setSaveError(null);
                  docState.setActivePath(p);
                });
              }}
              onClose={(p) => void docState.closeTab(p)}
              onCloseAll={() => void docState.closeAllTabs()}
            />
          </div>
          <div className="app-top-toolbar__end">
            {tauriRuntime && workspaceReady && editorUsable && docState.saveFeedback !== "idle" ? (
              <span className="app-top-toolbar__save-status" aria-live="polite">
                {docState.saveFeedback === "pending_auto"
                  ? t("toolbar.autoSavePending")
                  : docState.saveFeedback === "saving"
                    ? t("toolbar.saving")
                    : docState.saveFeedback === "saved"
                      ? t("toolbar.saved")
                      : null}
              </span>
            ) : null}
            <button
              type="button"
              className="app-top-toolbar__save"
              {...tauriDragExcludeProps}
              disabled={saveDisabled}
              aria-busy={docState.saving || docState.saveFeedback === "saving"}
              aria-label={
                docState.saving || docState.saveFeedback === "saving"
                  ? t("toolbar.saving")
                  : t("toolbar.save")
              }
              title={
                docState.saving || docState.saveFeedback === "saving"
                  ? t("toolbar.saving")
                  : isMacPlatform
                    ? t("toolbar.saveMac")
                    : t("toolbar.saveWin")
              }
              onClick={() => void docState.handleSave()}
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
              onClick={() => setRightPanelOpen((o) => !o)}
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
      <div
        id="app-file-directory-pane"
        className="app-sidebar-column"
        {...tauriWindowDragProps}
      >
        <ActivityBar
          activeView={leftPanelView}
          onViewChange={(v) => void changeView(v)}
          onOpenCognitiveReport={() => setCognitiveReportOpen(true)}
          onOpenSettings={() => setAiSettingsOpen(true)}
        />
        <aside className="sidebar">
          <nav className="sidebar__nav" aria-label={t("toolbar.files")}>
            {rootPath ? (
              <FileTree
                key={rootPath}
                nodes={tree}
                isKfPrivate={isPathKfPrivate}
                selectedPath={docState.activePath}
                revealActiveDragExcludeProps={
                  tauriRuntime ? (tauriDragExcludeProps as ButtonHTMLAttributes<HTMLButtonElement>) : undefined
                }
                onSelectFile={(p) => {
                  void changeView("files").then((ok) => {
                    if (!ok) return;
                    setPreferredNewMarkdownDir(parentDirOfRelPath(p));
                    void docState.openOrFocusTab(p);
                  });
                }}
                fileOps={fileTreeFileOps}
                newMarkdownAction={
                  newMarkdownToolbar
                    ? {
                        ...newMarkdownToolbar,
                        dragExcludeProps:
                          tauriDragExcludeProps as ButtonHTMLAttributes<HTMLButtonElement>,
                      }
                    : undefined
                }
                newFolderAction={
                  newFolderToolbar
                    ? {
                        ...newFolderToolbar,
                        dragExcludeProps:
                          tauriDragExcludeProps as ButtonHTMLAttributes<HTMLButtonElement>,
                      }
                    : undefined
                }
              />
            ) : (
              <p className="sidebar__hint">{t("sidebar.chooseFolder")}</p>
            )}
          </nav>
          {folderLoadError && <p className="sidebar__error">{folderLoadError}</p>}
          {rootPath ? (
            <footer className="sidebar__footer">
              <div className="sidebar__footer-bar">
                <button
                  type="button"
                  className="sidebar__root"
                  title={t("toolbar.open")}
                  onClick={() => void pickFolder()}
                >
                  <svg
                    className="sidebar__root-icon"
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
                    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 2H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2Z" />
                  </svg>
                  <span className="sidebar__root-path">
                    <span className="sidebar__root-path__ltr">{rootPath}</span>
                  </span>
                </button>
              </div>
            </footer>
          ) : null}
        </aside>
        {sidebarOpen && (
          <div
            className={`panel-resizer panel-resizer--left${leftResizable.isDragging ? " panel-resizer--active" : ""}`}
            onMouseDown={leftResizable.handleMouseDown}
            role="separator"
            aria-label={t("toolbar.resize")}
          />
        )}
      </div>
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
                setLeftPanelView("files");
                void onOpenCoachMarkdownPath(relPath);
              }}
            />
          </main>
        ) : leftPanelView === "linkRec" ? (
          <main className="main main--full-view">
            <LinkRecommendationPanel
              workspaceRoot={rootPath}
              activeRelPath={docState.activePath}
              panelActive={leftPanelView === "linkRec"}
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
                  onThoughtDetailDirtyChange={setThoughtMgmtBodyDirty}
                  onOpenNote={(relPath) => {
                    setLeftPanelView("files");
                    void onOpenCoachMarkdownPath(relPath);
                  }}
                />
              </ThoughtMgmtAiConversationSessionProvider>
            </main>
          ) : null
        ) : (
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
                        onClick={handleKfPrivateBarToggle}
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
                          onClick={toggleMarkdownSourceView}
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
                              onSaveAsThought={setEditorSaveThoughtText}
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
                          onClose={() => setEditorFindOpen(false)}
                          previewMode={!showMarkdownSource}
                          rawFullMarkdown={current.content}
                          rawTextareaRef={rawSourceTextareaRef}
                          crepeApiRef={crepeEditorApiRef}
                          docKey={docState.activePath}
                          workspaceSearchJumpSeed={editorFindWorkspaceSeed}
                          onWorkspaceSearchJumpSeedConsumed={clearEditorFindWorkspaceSeed}
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
        )}
        {showRightColumn && leftPanelView === "files" && (
          <div
            className={`panel-resizer panel-resizer--right${rightResizable.isDragging ? " panel-resizer--active" : ""}`}
            onMouseDown={rightResizable.handleMouseDown}
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
              onViewChange={setRightPanelTab}
              onAfterSelectAiTab={() => setSidebarOpen(false)}
              outlineTabEnabled={editorReady}
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
              reviewPanel={<RightPanelReviewTab onClose={() => setRightPanelTab("ai")} />}
              reviewTabBadgeCount={reviewTabBadgeCount}
            />
          </AiConversationSessionProvider>
        )}
      </div>
      {aiSettingsOpen ? (
        <Suspense fallback={null}>
          <AiLlmSettingsModal
            open={aiSettingsOpen}
            onClose={() => setAiSettingsOpen(false)}
            workspaceReady={workspaceReady}
            tauriRuntime={tauriRuntime}
            dragExcludeProps={
              tauriRuntime ? { "data-tauri-drag-region-exclude": true } : {}
            }
          />
        </Suspense>
      ) : null}
      {fileCommandModals}
    </div>
      <ThoughtMaturityToastHost />
      <CommandPalette
        open={commandPaletteOpen}
        onClose={() => setCommandPaletteOpen(false)}
        onOpenCognitiveReport={openCognitiveReportFromPalette}
        onOpenThoughtVaultHub={
          workspaceReady && tauriRuntime && rootPath
            ? () => setThoughtVaultHubOpen(true)
            : undefined
        }
        onOpenWorkspaceSearch={
          workspaceReady && tauriRuntime && rootPath
            ? () => setWorkspaceSearchOpen(true)
            : undefined
        }
        onTriggerWritingCoach={
          editorUsable
            ? () => writingCoachRef.current?.triggerManually()
            : undefined
        }
      />
      <WorkspaceSearchModal
        open={workspaceSearchOpen}
        onClose={() => setWorkspaceSearchOpen(false)}
        workspaceReady={workspaceReady}
        tauriRuntime={tauriRuntime}
        onNavigateToHit={handleWorkspaceSearchNavigate}
      />
      <ThoughtVaultHubModal
        open={thoughtVaultHubOpen}
        onClose={() => setThoughtVaultHubOpen(false)}
        workspaceReady={workspaceReady}
        tauriRuntime={tauriRuntime}
        onOpenNote={(relPath) => {
          void onOpenCoachMarkdownPath(relPath);
        }}
      />
      {editorSaveThoughtText != null && (
        <ThoughtSavePopover
          content={editorSaveThoughtText}
          defaultRelPath={docState.activePath}
          isSelection
          onSaved={() => setEditorSaveThoughtText(null)}
          onCancel={() => setEditorSaveThoughtText(null)}
        />
      )}
      <CognitiveReportPanel open={cognitiveReportOpen} onClose={() => setCognitiveReportOpen(false)} />
    </AiNoteContextProvider>
  );
}

export default App;
