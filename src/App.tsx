import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ButtonHTMLAttributes,
} from "react";
import { useTranslation } from "react-i18next";
import { AiNoteContextProvider } from "./contexts/AiNoteContext";
import type { DepthMode } from "./types/cognitiveTypes";
const AiLlmSettingsModal = lazy(() => import("./components/AiLlmSettingsModal"));

import { FileTree, collectKfPrivateRelPaths } from "./components/FileTree";
import type { CrepeMarkdownEditorApi } from "./components/CrepeMarkdownEditor";
import { CognitiveReportPanel } from "./components/cognitive-report/CognitiveReportPanel";
import { CommandPalette } from "./components/CommandPalette";
import { AppTopToolbar } from "./components/AppTopToolbar";
import { ContentArea } from "./components/ContentArea";
import { type EditorWritingCoachHostHandle } from "./components/EditorWritingCoachHost";
import { type RightPanelTab } from "./components/RightPanelShell";
import { ThoughtMaturityToastHost } from "./components/ThoughtMaturityToastHost";
import { ThoughtSavePopover } from "./components/ThoughtSavePopover";
import { ThoughtVaultHubModal } from "./components/ThoughtVaultHubModal";
import { WorkspaceSearchModal } from "./components/WorkspaceSearchModal";
import { ActivityBar, type LeftPanelView } from "./components/ActivityBar";
import { OnboardingOverlay } from "./components/OnboardingOverlay";

import { useGlobalShortcuts } from "./hooks/useGlobalShortcuts";
import { useHeadingNavigation } from "./hooks/useHeadingNavigation";
import { useWindowLifecycle } from "./hooks/useWindowLifecycle";
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
  const [onboardingOpen, setOnboardingOpen] = useState(false);
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

  const rawSourceTextareaRef = useRef<HTMLTextAreaElement>(null);
  const crepeEditorApiRef = useRef<CrepeMarkdownEditorApi | null>(null);
  const writingCoachRef = useRef<EditorWritingCoachHostHandle>(null);
  /** 全文搜索打开笔记后：待预览态滚到近似命中行 */
  const pendingWorkspaceSearchGotoRef = useRef<{ relPath: string; line: number } | null>(null);
  const docState = useOpenDocs(workspaceReady);
  const activePathSwitchTraceRef = useRef<PerfTrace | null>(null);
  const markdownBodyCacheRef = useRef<Map<string, string>>(new Map());

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

  /** Navigate to Practice Mode (used by keyboard shortcut and onboarding) */
  const requestOpenPracticeMode = useCallback(() => {
    setLeftPanelView("practice");
  }, []);

  useEffect(() => {
    if (!workspaceReady || !tauriRuntime) return;
    if (localStorage.getItem("knowforge:onboardingCompleted")) return;
    invoke("seed_onboarding_content").catch(() => {});
    setOnboardingOpen(true);
  }, [workspaceReady, tauriRuntime]);

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
    onStartChallengeReview: requestOpenPracticeMode,
  });

  resetWorkspaceFileCommandsRef.current = resetWorkspaceFileCommands;

  useEffect(() => {
    const onOpenAiSettings = () => setAiSettingsOpen(true);
    const onGoToPractice = () => setLeftPanelView("practice");
    const onOpenNoteInEditor = (e: Event) => {
      const relPath = (e as CustomEvent<{ relPath: string }>).detail?.relPath;
      if (relPath) {
        setLeftPanelView("files");
        void openOrFocusRef.current(relPath);
      }
    };
    window.addEventListener(OPEN_AI_SETTINGS_EVENT, onOpenAiSettings);
    window.addEventListener("knowforge:goToPractice", onGoToPractice);
    window.addEventListener("knowforge:openNoteInEditor", onOpenNoteInEditor);
    return () => {
      window.removeEventListener(OPEN_AI_SETTINGS_EVENT, onOpenAiSettings);
      window.removeEventListener("knowforge:goToPractice", onGoToPractice);
      window.removeEventListener("knowforge:openNoteInEditor", onOpenNoteInEditor);
    };
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
    if (view === "files") setSidebarOpen(true);
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

  useGlobalShortcuts(
    {
      openAiPanel: () => { setRightPanelOpen(true); setRightPanelTab("ai"); },
      toggleCommandPalette: () => { setCognitiveReportOpen(false); setCommandPaletteOpen((o) => !o); },
      triggerWritingCoach: () => { writingCoachRef.current?.triggerManually(); },
      openEditorFind: () => { setEditorFindOpen(true); },
    },
    { workspaceReady, editorUsable },
  );

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

  const { navigateToHeading, navigateToHeadingWithRetry } = useHeadingNavigation(editorScrollRef);

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

  const handleToolbarTabSelect = useCallback((p: string) => {
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
  }, [docState.activePath, leftPanelView, changeView, docState.setSaveError, docState.setActivePath]);

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

  useWindowLifecycle({
    tauriRuntime,
    appWindow,
    flushDirtyBeforeExit: docState.flushDirtyDocumentsBeforeExit,
    hasAnyDirtyTab: docState.hasAnyDirtyTab,
  });

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
      <AppTopToolbar
        tabPaths={docState.tabPaths}
        activePath={docState.activePath}
        isDirty={docState.isDirty}
        hasDiskStaleConflict={docState.hasDiskStaleConflict}
        isKfPrivate={isPathKfPrivate}
        onSelectTab={handleToolbarTabSelect}
        onCloseTab={(p) => void docState.closeTab(p)}
        onCloseAllTabs={() => void docState.closeAllTabs()}
        onRenameTab={onRenameTabFromBar}
        tauriDragExclude={tauriRuntime}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((o) => !o)}
        rightPanelOpen={rightPanelOpen}
        onToggleRightPanel={() => setRightPanelOpen((o) => !o)}
        workspaceReady={workspaceReady}
        editorUsable={editorUsable}
        saveDisabled={saveDisabled}
        saving={docState.saving}
        saveFeedback={docState.saveFeedback}
        onSave={() => void docState.handleSave()}
        onOpenWorkspaceSearch={() => setWorkspaceSearchOpen(true)}
        tauriRuntime={tauriRuntime}
        isMacPlatform={isMacPlatform}
        tauriDragExcludeProps={tauriDragExcludeProps}
        tauriWindowDragProps={tauriWindowDragProps}
        onTitlebarMouseDown={handleTitlebarMouseDown}
        onTitlebarDoubleClick={handleTitlebarDoubleClick}
        appWindow={appWindow}
      />
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
          reviewDueCount={reviewDueTabCount}
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
      <ContentArea
        leftPanelView={leftPanelView}
        sidebarOpen={sidebarOpen}
        workspaceReady={workspaceReady}
        tauriRuntime={tauriRuntime}
        rootPath={rootPath}
        docState={docState}
        editorReady={editorReady}
        showMarkdownSource={showMarkdownSource}
        activeMarkdownBodyForEditor={activeMarkdownBodyForEditor}
        editorScrollRef={editorScrollRef}
        crepeEditorApiRef={crepeEditorApiRef}
        rawSourceTextareaRef={rawSourceTextareaRef}
        writingCoachRef={writingCoachRef}
        wikiSuggestFiles={wikiSuggestFiles}
        isPathKfPrivate={isPathKfPrivate}
        onOpenCoachMarkdownPath={onOpenCoachMarkdownPath}
        onToggleMarkdownSource={toggleMarkdownSourceView}
        onKfPrivateBarToggle={handleKfPrivateBarToggle}
        onSaveAsThought={setEditorSaveThoughtText}
        editorFindOpen={editorFindOpen}
        editorFindWorkspaceSeed={editorFindWorkspaceSeed}
        onEditorFindClose={() => setEditorFindOpen(false)}
        onEditorFindSeedConsumed={clearEditorFindWorkspaceSeed}
        showRightColumn={showRightColumn}
        rightPanelOpen={rightPanelOpen}
        rightPanelTab={rightPanelTab}
        onRightPanelTabChange={setRightPanelTab}
        onAfterSelectAiTab={() => setSidebarOpen(false)}
        rightResizableIsDragging={rightResizable.isDragging}
        rightResizableHandleMouseDown={rightResizable.handleMouseDown}
        initialDepthMode={initialDepthMode}
        outlineState={outlineState}
        outlineFold={outlineFold}
        navigateToHeading={navigateToHeading}
        thoughtManagementSessionActive={thoughtManagementSessionActive}
        onThoughtMgmtDirtyChange={setThoughtMgmtBodyDirty}
        onSetLeftPanelView={setLeftPanelView}
        tauriDragExcludeProps={tauriDragExcludeProps}
        tauriWindowDragProps={tauriWindowDragProps}
      />
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
        onStartOnboarding={() => setOnboardingOpen(true)}
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
      <OnboardingOverlay
        open={onboardingOpen}
        onClose={() => setOnboardingOpen(false)}
        onStartChallenge={() => {
          setOnboardingOpen(false);
          localStorage.setItem("knowforge:onboardingCompleted", "true");
          requestOpenPracticeMode();
        }}
        tauriRuntime={tauriRuntime}
      />
    </AiNoteContextProvider>
  );
}

export default App;
