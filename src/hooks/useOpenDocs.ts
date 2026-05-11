import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { ask, message } from "@tauri-apps/plugin-dialog";
import i18n from "../i18n";
import { mergeEditorMarkdownWithStoredFrontmatter } from "../utils/kfPrivateFrontmatterEdit";
import { markdownEquivalentForDirty } from "../utils/markdownDirtyEquivalence";
import { endPerfTrace, startPerfTrace } from "../utils/perfTrace";
import { sha256Utf8Hex } from "../utils/sha256Utf8Hex";
import { isMarkdownRelPath, syncMarkdownHeadingAfterRename } from "../utils/syncMarkdownHeadingOnRename";

/** 防抖自动保存间隔（毫秒） */
const AUTO_SAVE_DEBOUNCE_MS = 1200;
/** 「已保存」提示显示时长 */
const SAVED_FEEDBACK_MS = 2200;
/** 与同路径 in-flight 写入对齐：轮询间隔与上限（与 persist 内等待逻辑一致） */
const PERSIST_IN_FLIGHT_POLL_MS = 25;
const PERSIST_IN_FLIGHT_MAX_POLLS = 80;

/** `read_markdown_file` 在文件不存在时经 sanitize_io_error 返回的文案片段 */
const READ_MARKDOWN_NOT_FOUND_HINT = "File not found";

/** 自动保存写盘失败后指数退避重试上限（次） */
const MAX_AUTO_SAVE_FAILURE_RETRIES = 8;

/**
 * 保存前读盘并计算基线 SHA，与后端当前字节一致才允许覆盖（含外部修改后的磁盘）。
 * 新建尚未落盘的文件读盘失败时返回 null。
 */
async function readDiskBaselineSha256ForPersist(relPath: string): Promise<string | null> {
  try {
    const diskText = await invoke<string>("read_markdown_file", { relPath });
    return await sha256Utf8Hex(diskText);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (msg.includes(READ_MARKDOWN_NOT_FOUND_HINT)) {
      return null;
    }
    throw e;
  }
}

/** 等待指定路径上尚未结束的写入，避免读盘与刚落盘的写交叉 */
async function waitWhilePathPersistInFlight(
  relPath: string,
  inFlightRef: { current: Set<string> },
): Promise<void> {
  for (let i = 0; i < PERSIST_IN_FLIGHT_MAX_POLLS && inFlightRef.current.has(relPath); i++) {
    await new Promise((r) => setTimeout(r, PERSIST_IN_FLIGHT_POLL_MS));
  }
}

export type DocState = {
  content: string;
  savedContent: string;
  loadError: string | null;
  loading: boolean;
};

export type EditorDocumentSession = DocState & {
  relPath: string;
  contentInjectEpoch: number;
  dirty: boolean;
  hasDiskStaleConflict: boolean;
};

type MarkdownFileSignature = {
  sizeBytes: number;
  modifiedNs: string;
};

/** 相对上次保存快照是否有未落盘的实质编辑（AST 语义比较，兼容列表/HTML 等往返差异） */
function docIsDirty(doc: DocState): boolean {
  return !markdownEquivalentForDirty(doc.content, doc.savedContent);
}

function sameMarkdownFileSignature(
  a: MarkdownFileSignature | undefined,
  b: MarkdownFileSignature | undefined,
): boolean {
  return !!a && !!b && a.sizeBytes === b.sizeBytes && a.modifiedNs === b.modifiedNs;
}

function emptyDoc(): DocState {
  return { content: "", savedContent: "", loadError: null, loading: true };
}

export function useOpenDocs(workspaceReady: boolean) {
  const [tabPaths, setTabPaths] = useState<string[]>([]);
  const [docByPath, setDocByPath] = useState<Record<string, DocState>>({});
  const [activePath, setActivePath] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  /** 磁盘重载等非换标签场景下递增，驱动编辑器 replaceAll 注入新正文 */
  const [contentInjectEpochByPath, setContentInjectEpochByPath] = useState<Record<string, number>>({});
  /** 磁盘内容与 savedContent 不一致且当前有未保存编辑时的路径（需提示重新加载或忽略） */
  const [diskStaleDirtyPaths, setDiskStaleDirtyPaths] = useState<string[]>([]);
  /** 工作区尚未就绪时调用 openOrFocusTab 会暂存于此，就绪后由 effect 补打开（避免恢复路径被静默丢弃） */
  const pendingOpenRelPathRef = useRef<string | null>(null);
  /** 各路径自动保存定时器 */
  const autoSaveTimersRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  /** 避免同一文件并发写入 */
  const persistInFlightRef = useRef<Set<string>>(new Set());
  /** 自动保存在 in-flight 期间被跳过：当前写入结束后补一次防抖调度 */
  const pendingAutoSaveAfterPersistRef = useRef<Set<string>>(new Set());
  /** 自动保存连续失败次数（成功落盘后清零） */
  const autoSaveFailureRetryCountRef = useRef<Record<string, number>>({});
  /** 最近一次已确认磁盘快照；切回 Tab 时先比元数据，未变则跳过全文读盘 */
  const diskSignatureByPathRef = useRef<Record<string, MarkdownFileSignature>>({});
  const diskStaleDirtyPathsRef = useRef<string[]>([]);
  const savedFeedbackClearRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tabPathsRef = useRef<string[]>([]);
  tabPathsRef.current = tabPaths;
  /** 供 handleMarkdownChange 在 persist 定义前触发防抖保存 */
  const scheduleAutoSaveRef = useRef<(relPath: string, editedContent: string) => void>(() => {});

  /** 当前活动文档的保存状态反馈（手动 / 自动 共用） */
  const [saveFeedback, setSaveFeedback] = useState<
    "idle" | "pending_auto" | "saving" | "saved"
  >("idle");

  const sessionsByPath = useMemo<Record<string, EditorDocumentSession>>(() => {
    const stalePaths = new Set(diskStaleDirtyPaths);
    const sessions: Record<string, EditorDocumentSession> = {};
    for (const [relPath, doc] of Object.entries(docByPath)) {
      sessions[relPath] = {
        ...doc,
        relPath,
        contentInjectEpoch: contentInjectEpochByPath[relPath] ?? 0,
        dirty: !doc.loading && !doc.loadError && docIsDirty(doc),
        hasDiskStaleConflict: stalePaths.has(relPath),
      };
    }
    return sessions;
  }, [contentInjectEpochByPath, diskStaleDirtyPaths, docByPath]);

  const activeSession = activePath ? sessionsByPath[activePath] : undefined;
  const activeDoc = activeSession;
  const dirty = !!activeSession && activeSession.dirty;

  const docByPathRef = useRef(docByPath);
  docByPathRef.current = docByPath;

  const activePathRef = useRef(activePath);
  activePathRef.current = activePath;

  const workspaceReadyRef = useRef(workspaceReady);
  workspaceReadyRef.current = workspaceReady;

  const savingRef = useRef(saving);
  savingRef.current = saving;

  useEffect(() => {
    diskStaleDirtyPathsRef.current = diskStaleDirtyPaths;
  }, [diskStaleDirtyPaths]);

  const clearAutoSaveTimer = useCallback((relPath: string) => {
    const t = autoSaveTimersRef.current[relPath];
    if (t != null) {
      clearTimeout(t);
      delete autoSaveTimersRef.current[relPath];
    }
  }, []);

  const clearAllAutoSaveTimers = useCallback(() => {
    for (const t of Object.values(autoSaveTimersRef.current)) {
      clearTimeout(t);
    }
    autoSaveTimersRef.current = {};
  }, []);

  const bumpSavedFeedback = useCallback(() => {
    if (savedFeedbackClearRef.current != null) {
      clearTimeout(savedFeedbackClearRef.current);
    }
    setSaveFeedback("saved");
    savedFeedbackClearRef.current = setTimeout(() => {
      savedFeedbackClearRef.current = null;
      setSaveFeedback("idle");
    }, SAVED_FEEDBACK_MS);
  }, []);

  const isDirty = useCallback(
    (relPath: string) => {
      const doc = docByPath[relPath];
      return !!doc && !doc.loading && docIsDirty(doc);
    },
    [docByPath],
  );

  const hasAnyDirtyTab = useCallback(() => {
    return tabPaths.some((path) => isDirty(path));
  }, [isDirty, tabPaths]);

  const confirmDiscardAllTabs = useCallback(async () => {
    if (!hasAnyDirtyTab()) {
      return true;
    }

    return ask(i18n.t("dialogs.discardUnsavedTabs"), {
      title: i18n.t("dialogs.discard"),
      kind: "warning",
    });
  }, [hasAnyDirtyTab]);

  const resetOpenDocs = useCallback(() => {
    pendingOpenRelPathRef.current = null;
    persistInFlightRef.current.clear();
    pendingAutoSaveAfterPersistRef.current.clear();
    autoSaveFailureRetryCountRef.current = {};
    clearAllAutoSaveTimers();
    if (savedFeedbackClearRef.current != null) {
      clearTimeout(savedFeedbackClearRef.current);
      savedFeedbackClearRef.current = null;
    }
    setSaveFeedback("idle");
    setTabPaths([]);
    setDocByPath({});
    setActivePath(null);
    setSaveError(null);
    setSaving(false);
    setContentInjectEpochByPath({});
    setDiskStaleDirtyPaths([]);
    diskSignatureByPathRef.current = {};
  }, [clearAllAutoSaveTimers]);

  const openOrFocusTab = useCallback(async (relPath: string): Promise<string | null> => {
    if (!workspaceReadyRef.current) {
      pendingOpenRelPathRef.current = relPath;
      return null;
    }

    pendingOpenRelPathRef.current = null;
    setSaveError(null);
    if (docByPathRef.current[relPath]) {
      setActivePath(relPath);
      return docByPathRef.current[relPath]?.content ?? null;
    }

    setTabPaths((paths) => (paths.includes(relPath) ? paths : [...paths, relPath]));
    setDocByPath((prev) => ({ ...prev, [relPath]: emptyDoc() }));
    setActivePath(relPath);

    const loadTrace = startPerfTrace("markdown.open.read_file", { relPath });
    try {
      const [text, signature] = await Promise.all([
        invoke<string>("read_markdown_file", { relPath }),
        invoke<MarkdownFileSignature>("get_markdown_file_signature", { relPath }),
      ]);
      diskSignatureByPathRef.current[relPath] = signature;
      endPerfTrace(loadTrace, { status: "ok", bytes: text.length });
      setDocByPath((prev) => {
        if (!prev[relPath]) {
          return prev;
        }
        return {
          ...prev,
          [relPath]: {
            content: text,
            savedContent: text,
            loadError: null,
            loading: false,
          },
        };
      });
      return text;
    } catch (e) {
      endPerfTrace(loadTrace, { status: "error" });
      setDocByPath((prev) => {
        if (!prev[relPath]) {
          return prev;
        }
        return {
          ...prev,
          [relPath]: {
            content: "",
            savedContent: "",
            loadError: e instanceof Error ? e.message : String(e),
            loading: false,
          },
        };
      });
      return null;
    }
  }, []);

  useEffect(() => {
    if (!workspaceReady) {
      return;
    }
    const pending = pendingOpenRelPathRef.current;
    if (pending == null) {
      return;
    }
    pendingOpenRelPathRef.current = null;
    void openOrFocusTab(pending);
  }, [workspaceReady, openOrFocusTab]);

  const closeTab = useCallback(
    async (relPath: string) => {
      const doc = docByPathRef.current[relPath];
      if (doc && !doc.loading && docIsDirty(doc)) {
        const name = relPath.includes("/") ? relPath.slice(relPath.lastIndexOf("/") + 1) : relPath;
        const ok = await ask(i18n.t("dialogs.closeTab", { name }), {
          title: i18n.t("dialogs.close"),
          kind: "warning",
        });
        if (!ok) {
          return;
        }
      }

      clearAutoSaveTimer(relPath);
      delete diskSignatureByPathRef.current[relPath];

      setTabPaths((prevTabs) => {
        const idx = prevTabs.indexOf(relPath);
        if (idx < 0) {
          return prevTabs;
        }
        const nextTabs = prevTabs.filter((path) => path !== relPath);
        setActivePath((currentPath) => {
          if (currentPath !== relPath) {
            return currentPath;
          }
          // 关闭最后一项时 idx === nextTabs.length，无 nextTabs[idx]；仅一项时 nextTabs 为空
          if (nextTabs.length === 0) {
            return null;
          }
          if (idx < nextTabs.length) {
            return nextTabs[idx];
          }
          return nextTabs[idx - 1];
        });
        return nextTabs;
      });

      setDocByPath((prev) => {
        const next = { ...prev };
        delete next[relPath];
        return next;
      });
    },
    [clearAutoSaveTimer],
  );

  const closeAllTabs = useCallback(async () => {
    if (!await confirmDiscardAllTabs()) {
      return;
    }
    persistInFlightRef.current.clear();
    pendingAutoSaveAfterPersistRef.current.clear();
    autoSaveFailureRetryCountRef.current = {};
    clearAllAutoSaveTimers();
    setSaveFeedback("idle");
    setTabPaths([]);
    setDocByPath({});
    setActivePath(null);
    setSaveError(null);
    setDiskStaleDirtyPaths([]);
    diskSignatureByPathRef.current = {};
  }, [clearAllAutoSaveTimers, confirmDiscardAllTabs]);

  const handleMarkdownChange = useCallback(
    (
      relPath: string,
      markdown: string,
      meta?: { baseline?: boolean; /** 调用方已给出含 frontmatter 的完整全文，禁止再与旧 YAML 拼接 */ fullDocument?: boolean },
    ) => {
      let mergedForAutosave: string | null = null;
      setDocByPath((prev) => {
        const current = prev[relPath];
        if (!current) {
          return prev;
        }
        const merged = meta?.fullDocument
          ? markdown
          : mergeEditorMarkdownWithStoredFrontmatter(current.content, markdown);
        if (!meta?.baseline) {
          mergedForAutosave = merged;
        }
        if (meta?.baseline) {
          // 换标签回来时 replaceAll 会走 baseline；若此时仍有未保存修改，不得把 saved 抬成当前正文，否则会误清脏状态
          const clean = markdownEquivalentForDirty(current.content, current.savedContent);
          return {
            ...prev,
            [relPath]: {
              ...current,
              content: merged,
              savedContent: clean ? merged : current.savedContent,
            },
          };
        }
        return { ...prev, [relPath]: { ...current, content: merged } };
      });
      if (!meta?.baseline && mergedForAutosave !== null) {
        const payload = mergedForAutosave;
        queueMicrotask(() => scheduleAutoSaveRef.current(relPath, payload));
      }
    },
    [],
  );

  /** 程序化改写缓冲区正文时递增，驱动 Crepe replaceAll 注入（仅靠 handleMarkdownChange 不会触发 contentSyncKey） */
  const bumpContentInjectEpochForPath = useCallback((relPath: string) => {
    setContentInjectEpochByPath((prev) => ({
      ...prev,
      [relPath]: (prev[relPath] ?? 0) + 1,
    }));
  }, []);

  const reloadFromDisk = useCallback(async (relPath: string) => {
    if (!workspaceReadyRef.current) {
      return;
    }
    try {
      setSaveError(null);
      clearAutoSaveTimer(relPath);
      await waitWhilePathPersistInFlight(relPath, persistInFlightRef);
      const [text, signature] = await Promise.all([
        invoke<string>("read_markdown_file", { relPath }),
        invoke<MarkdownFileSignature>("get_markdown_file_signature", { relPath }),
      ]);
      diskSignatureByPathRef.current[relPath] = signature;
      setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
      setDocByPath((prev) => {
        const cur = prev[relPath];
        if (!cur) {
          return prev;
        }
        return {
          ...prev,
          [relPath]: {
            ...cur,
            content: text,
            savedContent: text,
            loadError: null,
            loading: false,
          },
        };
      });
      setContentInjectEpochByPath((prev) => ({
        ...prev,
        [relPath]: (prev[relPath] ?? 0) + 1,
      }));
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [clearAutoSaveTimer]);

  const persistDocument = useCallback(
    async (
      relPath: string,
      source: "manual" | "auto" | "visibility" | "exit",
      options?: { exactMarkdown?: string },
    ): Promise<boolean> => {
      if (!workspaceReadyRef.current || !isTauri()) {
        return false;
      }
      const exact = options?.exactMarkdown;
      if (exact != null) {
        clearAutoSaveTimer(relPath);
      }

      let doc = docByPathRef.current[relPath];
      if (!doc || doc.loading || doc.loadError) {
        return false;
      }
      let contentToSave = exact ?? doc.content;
      if (markdownEquivalentForDirty(contentToSave, doc.savedContent)) {
        return true;
      }

      if (persistInFlightRef.current.has(relPath)) {
        if (source === "auto") {
          pendingAutoSaveAfterPersistRef.current.add(relPath);
          return true;
        }
        await waitWhilePathPersistInFlight(relPath, persistInFlightRef);
        doc = docByPathRef.current[relPath];
        if (!doc || doc.loading) {
          return false;
        }
        contentToSave = exact ?? doc.content;
        if (markdownEquivalentForDirty(contentToSave, doc.savedContent)) {
          return true;
        }
      }

      persistInFlightRef.current.add(relPath);
      const ap = activePathRef.current;
      if (ap === relPath && source !== "exit") {
        setSaveFeedback("saving");
      }

      try {
        const diskBaselineSha256 = await readDiskBaselineSha256ForPersist(relPath);
        await invoke("write_markdown_file", {
          relPath,
          content: contentToSave,
          diskBaselineSha256,
        });
        void invoke<MarkdownFileSignature>("get_markdown_file_signature", { relPath })
          .then((signature) => {
            diskSignatureByPathRef.current[relPath] = signature;
          })
          .catch(() => {
            delete diskSignatureByPathRef.current[relPath];
          });
        delete autoSaveFailureRetryCountRef.current[relPath];
        const latest = docByPathRef.current[relPath];
        const diverged = latest != null && latest.content !== contentToSave;
        setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
        setSaveError(null);
        setDocByPath((prev) => {
          const cur = prev[relPath];
          if (!cur) {
            return prev;
          }
          if (cur.content !== contentToSave) {
            return prev;
          }
          return {
            ...prev,
            [relPath]: { ...cur, savedContent: contentToSave },
          };
        });
        if (ap === relPath && source !== "exit") {
          bumpSavedFeedback();
        }
        if (
          diverged &&
          latest &&
          !diskStaleDirtyPathsRef.current.includes(relPath) &&
          isTauri() &&
          workspaceReadyRef.current
        ) {
          scheduleAutoSaveRef.current(relPath, latest.content);
        }
        return true;
      } catch (e) {
        const raw = e instanceof Error ? e.message : String(e);
        if (raw.includes("DISK_CONFLICT")) {
          try {
            await message(i18n.t("errors.diskConflictSave"), {
              title: i18n.t("errors.diskConflictTitle"),
              kind: "warning",
            });
          } catch {
            /* 弹窗失败时仍保留顶栏错误文案 */
          }
          setSaveError(i18n.t("errors.diskConflictSave"));
          setDiskStaleDirtyPaths((prev) => (prev.includes(relPath) ? prev : [...prev, relPath]));
          // 语义等价时曾只清 stale 不更新 savedContent，导致 sha256 仍按旧串算、永久 DISK_CONFLICT；读盘把 baseline 对齐真实字节
          try {
            const fromDisk = await invoke<string>("read_markdown_file", { relPath });
            setDocByPath((prev) => {
              const cur = prev[relPath];
              if (!cur) {
                return prev;
              }
              return { ...prev, [relPath]: { ...cur, savedContent: fromDisk } };
            });
            setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
            setSaveError(null);
          } catch {
            /* 读盘失败则保留 stale，由用户手动「从磁盘重新加载」 */
          }
        } else {
          setSaveError(raw);
        }
        if (ap === relPath) {
          setSaveFeedback("idle");
        }
        if (source === "auto") {
          const n = (autoSaveFailureRetryCountRef.current[relPath] ?? 0) + 1;
          if (n <= MAX_AUTO_SAVE_FAILURE_RETRIES) {
            autoSaveFailureRetryCountRef.current[relPath] = n;
            const delay = Math.min(2000 * n, 12000);
            window.setTimeout(() => {
              const d = docByPathRef.current[relPath];
              if (!d || d.loading || d.loadError) {
                return;
              }
              if (markdownEquivalentForDirty(d.content, d.savedContent)) {
                delete autoSaveFailureRetryCountRef.current[relPath];
                return;
              }
              scheduleAutoSaveRef.current(relPath, d.content);
            }, delay);
          }
        }
        return false;
      } finally {
        persistInFlightRef.current.delete(relPath);
        if (pendingAutoSaveAfterPersistRef.current.has(relPath)) {
          pendingAutoSaveAfterPersistRef.current.delete(relPath);
          queueMicrotask(() => {
            const d = docByPathRef.current[relPath];
            if (!d || d.loading || d.loadError) {
              return;
            }
            if (!markdownEquivalentForDirty(d.content, d.savedContent)) {
              scheduleAutoSaveRef.current(relPath, d.content);
            }
          });
        }
      }
    },
    [bumpSavedFeedback, clearAutoSaveTimer],
  );

  const scheduleAutoSave = useCallback(
    (relPath: string, editedContent: string) => {
      if (!isTauri() || !workspaceReadyRef.current) {
        return;
      }
      const saved = docByPathRef.current[relPath]?.savedContent;
      if (saved === undefined) {
        return;
      }
      if (markdownEquivalentForDirty(editedContent, saved)) {
        clearAutoSaveTimer(relPath);
        if (relPath === activePathRef.current) {
          setSaveFeedback((prev) => (prev === "saving" ? prev : "idle"));
        }
        return;
      }

      clearAutoSaveTimer(relPath);
      if (relPath === activePathRef.current) {
        setSaveFeedback((prev) => (prev === "saving" ? prev : "pending_auto"));
      }

      autoSaveTimersRef.current[relPath] = setTimeout(() => {
        delete autoSaveTimersRef.current[relPath];
        void persistDocument(relPath, "auto");
      }, AUTO_SAVE_DEBOUNCE_MS);
    },
    [clearAutoSaveTimer, persistDocument],
  );

  scheduleAutoSaveRef.current = scheduleAutoSave;

  /** notify 或切回标签时：磁盘相对 savedContent 变化时的处理 */
  const applyExternalDiskChange = useCallback(
    (relPath: string) => {
      const doc = docByPathRef.current[relPath];
      if (!doc || doc.loading) {
        return;
      }
      void (async () => {
        let text: string;
        let signature: MarkdownFileSignature | null = null;
        try {
          [text, signature] = await Promise.all([
            invoke<string>("read_markdown_file", { relPath }),
            invoke<MarkdownFileSignature>("get_markdown_file_signature", { relPath }),
          ]);
        } catch {
          return;
        }
        if (signature) {
          diskSignatureByPathRef.current[relPath] = signature;
        }
        const latest = docByPathRef.current[relPath];
        if (!latest || latest.loading) {
          return;
        }
        // 磁盘与缓冲区语义一致：必须把 savedContent 同步为读盘原文（语义等价 ≠ 字节/sha256 一致）
        if (markdownEquivalentForDirty(text, latest.content)) {
          setDocByPath((prev) => {
            const cur = prev[relPath];
            if (!cur || cur.savedContent === text) {
              return prev;
            }
            return { ...prev, [relPath]: { ...cur, savedContent: text } };
          });
          setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
          return;
        }
        const dirty = docIsDirty(latest);
        if (!dirty) {
          void reloadFromDisk(relPath);
        } else {
          setDiskStaleDirtyPaths((prev) => (prev.includes(relPath) ? prev : [...prev, relPath]));
        }
      })();
    },
    [reloadFromDisk],
  );

  const dismissDiskStaleForPath = useCallback((relPath: string) => {
    setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
  }, []);

  const hasDiskStaleConflict = useCallback(
    (relPath: string) => diskStaleDirtyPaths.includes(relPath),
    [diskStaleDirtyPaths],
  );

  /** 切换活动标签或文档加载完成后，与磁盘比对 savedContent（补全 notify 遗漏） */
  const activeLoadState =
    activePath == null
      ? "none"
      : !docByPath[activePath]
        ? "none"
        : docByPath[activePath].loading
          ? "loading"
          : docByPath[activePath].loadError
            ? "error"
            : "ready";

  useEffect(() => {
    if (!isTauri() || !workspaceReady || !activePath || activeLoadState !== "ready") {
      return;
    }

    let cancelled = false;
    const diskTrace = startPerfTrace("markdown.active_disk_check", { relPath: activePath });
    void (async () => {
      let signature: MarkdownFileSignature;
      try {
        signature = await invoke<MarkdownFileSignature>("get_markdown_file_signature", {
          relPath: activePath,
        });
      } catch {
        endPerfTrace(diskTrace, { status: "signature_error" });
        return;
      }
      if (cancelled) {
        endPerfTrace(diskTrace, { status: "cancelled_after_signature" });
        return;
      }

      const knownSignature = diskSignatureByPathRef.current[activePath];
      if (sameMarkdownFileSignature(knownSignature, signature)) {
        endPerfTrace(diskTrace, {
          status: "signature_unchanged",
          sizeBytes: signature.sizeBytes,
        });
        return;
      }

      try {
        const text = await invoke<string>("read_markdown_file", { relPath: activePath });
        if (cancelled) {
          endPerfTrace(diskTrace, { status: "cancelled", bytes: text.length });
          return;
        }
        diskSignatureByPathRef.current[activePath] = signature;
        const d = docByPathRef.current[activePath];
        if (!d || d.loading) {
          endPerfTrace(diskTrace, { status: "missing_doc", bytes: text.length });
          return;
        }
        // 与 applyExternalDiskChange 一致：语义对齐时仍须把 baseline 写成磁盘原文，避免 sha256 与真实文件不一致
        if (markdownEquivalentForDirty(text, d.content)) {
          setDocByPath((prev) => {
            const cur = prev[activePath];
            if (!cur || cur.savedContent === text) {
              return prev;
            }
            return { ...prev, [activePath]: { ...cur, savedContent: text } };
          });
          setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== activePath));
          endPerfTrace(diskTrace, { status: "matches_buffer", bytes: text.length });
          return;
        }
        if (markdownEquivalentForDirty(text, d.savedContent)) {
          setDocByPath((prev) => {
            const cur = prev[activePath];
            if (!cur || cur.savedContent === text) {
              return prev;
            }
            return { ...prev, [activePath]: { ...cur, savedContent: text } };
          });
          setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== activePath));
          endPerfTrace(diskTrace, { status: "matches_saved", bytes: text.length });
          return;
        }
        const dirty = docIsDirty(d);
        if (!dirty) {
          void reloadFromDisk(activePath);
          endPerfTrace(diskTrace, { status: "reload_from_disk", bytes: text.length });
        } else {
          setDiskStaleDirtyPaths((prev) =>
            prev.includes(activePath) ? prev : [...prev, activePath],
          );
          endPerfTrace(diskTrace, { status: "disk_stale_dirty", bytes: text.length });
        }
      } catch {
        endPerfTrace(diskTrace, { status: "error" });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [workspaceReady, activePath, activeLoadState, reloadFromDisk]);

  const flushDirtyDocumentsBeforeExit = useCallback(async (): Promise<{
    conflictDirtyPaths: string[];
    saveFailed: boolean;
  }> => {
    clearAllAutoSaveTimers();
    if (!isTauri() || !workspaceReadyRef.current) {
      return { conflictDirtyPaths: [], saveFailed: false };
    }
    const conflicts: string[] = [];
    for (const p of tabPathsRef.current) {
      const d = docByPathRef.current[p];
      if (!d || d.loading || d.loadError || !docIsDirty(d)) {
        continue;
      }
      const ok = await persistDocument(p, "exit");
      if (!ok) {
        conflicts.push(p);
        return { conflictDirtyPaths: conflicts, saveFailed: true };
      }
    }
    return { conflictDirtyPaths: conflicts, saveFailed: false };
  }, [clearAllAutoSaveTimers, persistDocument]);

  const handleSave = useCallback(async () => {
    const currentPath = activePathRef.current;
    if (!workspaceReadyRef.current || !currentPath || savingRef.current) {
      return;
    }

    const doc = docByPathRef.current[currentPath];
    if (!doc || doc.loading || !docIsDirty(doc)) {
      return;
    }

    clearAutoSaveTimer(currentPath);
    setSaveError(null);
    setSaving(true);
    try {
      await persistDocument(currentPath, "manual");
    } finally {
      setSaving(false);
    }
  }, [clearAutoSaveTimer, persistDocument]);

  /** 切到后台时尽快落盘，降低异常退出丢稿概率 */
  useEffect(() => {
    if (!isTauri()) {
      return;
    }
    const onVis = () => {
      if (document.visibilityState !== "hidden") {
        return;
      }
      const keys = Object.keys(autoSaveTimersRef.current);
      for (const relPath of keys) {
        clearAutoSaveTimer(relPath);
        void persistDocument(relPath, "visibility");
      }
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, [clearAutoSaveTimer, persistDocument]);

  /** 切换活动文档时重置顶栏保存提示（避免误显示上一标签状态） */
  useEffect(() => {
    if (!activePath) {
      setSaveFeedback("idle");
      return;
    }
    if (autoSaveTimersRef.current[activePath] != null) {
      setSaveFeedback("pending_auto");
    } else {
      setSaveFeedback("idle");
    }
  }, [activePath]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
        const currentPath = activePathRef.current;
        const doc = currentPath ? docByPathRef.current[currentPath] : undefined;
        if (
          workspaceReadyRef.current &&
          currentPath &&
          doc &&
          !doc.loading &&
          docIsDirty(doc) &&
          !savingRef.current
        ) {
          event.preventDefault();
          void handleSave();
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleSave]);

  /** 重命名磁盘上的文件夹后，批量更新该目录下已打开 Tab 的相对路径（不读写磁盘） */
  const renameDirectoryInOpenDocs = useCallback((fromDirRel: string, toDirRel: string) => {
    const fromNorm = fromDirRel.trim().replace(/\/+$/, "");
    const toNorm = toDirRel.trim().replace(/\/+$/, "");
    if (fromNorm === toNorm) {
      return;
    }
    const childPrefix = `${fromNorm}/`;
    const mapPath = (p: string): string => {
      if (p === fromNorm) {
        return toNorm;
      }
      if (p.startsWith(childPrefix)) {
        return toNorm + p.slice(fromNorm.length);
      }
      return p;
    };

    setTabPaths((paths) => paths.map(mapPath));
    setDocByPath((prev) => {
      const next: Record<string, DocState> = {};
      for (const [k, v] of Object.entries(prev)) {
        next[mapPath(k)] = v;
      }
      return next;
    });
    setActivePath((cur) => (cur == null ? cur : mapPath(cur)));
    setContentInjectEpochByPath((prev) => {
      const next: Record<string, number> = {};
      for (const [k, v] of Object.entries(prev)) {
        next[mapPath(k)] = v;
      }
      return next;
    });
    setDiskStaleDirtyPaths((prev) =>
      prev.map(mapPath).filter((p, i, arr) => arr.indexOf(p) === i),
    );
    const nextSignatures: Record<string, MarkdownFileSignature> = {};
    for (const [k, v] of Object.entries(diskSignatureByPathRef.current)) {
      nextSignatures[mapPath(k)] = v;
    }
    diskSignatureByPathRef.current = nextSignatures;

    const nextTimers: Record<string, ReturnType<typeof setTimeout>> = {};
    for (const [k, t] of Object.entries(autoSaveTimersRef.current)) {
      const nk = mapPath(k);
      nextTimers[nk] = t;
    }
    autoSaveTimersRef.current = nextTimers;

    const nextFlight = new Set<string>();
    for (const p of persistInFlightRef.current) {
      nextFlight.add(mapPath(p));
    }
    persistInFlightRef.current = nextFlight;
  }, []);

  /** 磁盘重命名后同步 Tab 与文档状态（不读写磁盘）；可选将首行 H1 与新旧文件名 stem 对齐 */
  const renameTabPath = useCallback(
    (
      fromRel: string,
      toRel: string,
      options?: { markdownHeadingRename?: { oldBasename: string; newBasename: string } },
    ) => {
      if (fromRel === toRel) {
        return;
      }
      const hdr = options?.markdownHeadingRename;
      setTabPaths((paths) => paths.map((p) => (p === fromRel ? toRel : p)));
      setDocByPath((prev) => {
        const doc = prev[fromRel];
        if (!doc) {
          return prev;
        }
        let nextDoc = doc;
        if (hdr && isMarkdownRelPath(toRel) && !doc.loading && doc.loadError == null) {
          const nextContent = syncMarkdownHeadingAfterRename(
            doc.content,
            hdr.oldBasename,
            hdr.newBasename,
          );
          if (nextContent !== doc.content) {
            nextDoc = { ...doc, content: nextContent };
          }
        }
        const next = { ...prev };
        delete next[fromRel];
        next[toRel] = nextDoc;
        return next;
      });
      setActivePath((cur) => (cur === fromRel ? toRel : cur));
      setContentInjectEpochByPath((prev) => {
        const next = { ...prev };
        const prevEpoch = next[fromRel] ?? 0;
        delete next[fromRel];
        next[toRel] = prevEpoch + 1;
        return next;
      });
      setDiskStaleDirtyPaths((prev) =>
        prev.map((p) => (p === fromRel ? toRel : p)).filter((p, i, arr) => arr.indexOf(p) === i),
      );
      const signature = diskSignatureByPathRef.current[fromRel];
      delete diskSignatureByPathRef.current[fromRel];
      if (signature) {
        diskSignatureByPathRef.current[toRel] = signature;
      }
      const pendingTimer = autoSaveTimersRef.current[fromRel];
      if (pendingTimer != null) {
        delete autoSaveTimersRef.current[fromRel];
        autoSaveTimersRef.current[toRel] = pendingTimer;
      }
    },
    [],
  );

  /** 删除磁盘文件后关闭对应 Tab，不弹确认（由调用方负责） */
  const removeTabByPath = useCallback((relPath: string) => {
    setTabPaths((prevTabs) => {
      const idx = prevTabs.indexOf(relPath);
      if (idx < 0) {
        return prevTabs;
      }
      const nextTabs = prevTabs.filter((path) => path !== relPath);
      setActivePath((currentPath) => {
        if (currentPath !== relPath) {
          return currentPath;
        }
        if (nextTabs.length === 0) {
          return null;
        }
        if (idx < nextTabs.length) {
          return nextTabs[idx];
        }
        return nextTabs[idx - 1];
      });
      return nextTabs;
    });
    setDocByPath((prev) => {
      const next = { ...prev };
      delete next[relPath];
      return next;
    });
    setContentInjectEpochByPath((prev) => {
      if (prev[relPath] == null) {
        return prev;
      }
      const next = { ...prev };
      delete next[relPath];
      return next;
    });
    setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== relPath));
    delete diskSignatureByPathRef.current[relPath];
    clearAutoSaveTimer(relPath);
  }, [clearAutoSaveTimer]);

  /** 删除目录后关闭其下已打开 Tab（不弹确认；由调用方在磁盘删除成功后使用） */
  const removeTabsUnderDirectory = useCallback(
    (dirRel: string) => {
      const norm = dirRel.trim().replace(/\/+$/, "");
      const prevTabs = tabPathsRef.current;
      const tr = new Set(prevTabs.filter((p) => p === norm || p.startsWith(`${norm}/`)));
      if (tr.size === 0) {
        return;
      }
      for (const p of tr) {
        clearAutoSaveTimer(p);
      }
      const nextTabs = prevTabs.filter((p) => !tr.has(p));
      const currentPath = activePathRef.current;
      if (currentPath !== null && tr.has(currentPath)) {
        const idx = prevTabs.indexOf(currentPath);
        if (nextTabs.length === 0) {
          setActivePath(null);
        } else if (idx < nextTabs.length) {
          setActivePath(nextTabs[idx]);
        } else {
          setActivePath(nextTabs[idx - 1]);
        }
      }
      setTabPaths(nextTabs);
      setDocByPath((prev) => {
        const next = { ...prev };
        for (const p of tr) {
          delete next[p];
        }
        return next;
      });
      setContentInjectEpochByPath((prev) => {
        const next = { ...prev };
        for (const p of tr) {
          if (next[p] != null) {
            delete next[p];
          }
        }
        return next;
      });
      setDiskStaleDirtyPaths((prev) => prev.filter((p) => p !== norm && !p.startsWith(`${norm}/`)));
      for (const p of tr) {
        delete diskSignatureByPathRef.current[p];
      }
    },
    [clearAutoSaveTimer],
  );

  return {
    activeDoc,
    activePath,
    activeSession,
    applyExternalDiskChange,
    bumpContentInjectEpochForPath,
    closeAllTabs,
    closeTab,
    confirmDiscardAllTabs,
    contentInjectEpochByPath,
    dirty,
    dismissDiskStaleForPath,
    diskStaleDirtyPaths,
    docByPath,
    flushDirtyDocumentsBeforeExit,
    handleMarkdownChange,
    handleSave,
    hasAnyDirtyTab,
    hasDiskStaleConflict,
    openOrFocusTab,
    persistDocument,
    reloadFromDisk,
    removeTabByPath,
    removeTabsUnderDirectory,
    renameDirectoryInOpenDocs,
    renameTabPath,
    resetOpenDocs,
    saveError,
    saveFeedback,
    saving,
    sessionsByPath,
    setActivePath,
    setSaveError,
    tabPaths,
    isDirty,
  };
}
