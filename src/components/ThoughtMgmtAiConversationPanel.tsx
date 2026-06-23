/**
 * 想法管理页右侧专用 AI 面板：会话存于 conversations_thought_mgmt，与文档侧栏隔离；
 * 发送时固定深度为「深」，并携带想法正文、关联笔记全文与工作区关键词检索。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { useThoughtMgmtAiConversationSession } from "../contexts/ThoughtMgmtAiConversationSessionContext";
import type { ThoughtFocusContext } from "../types/aiConversation";
import type { SearchWorkspaceContextResponse } from "../types/vaultContextSearch";
import type { ProviderProfileForUi } from "../types/vaultAiConfig";
import { markdownTreatAsKfPrivateForUi } from "../utils/kfPrivateMarkdown";
import { useAiNoteContext } from "../contexts/AiNoteContext";
import type { ReplyContextSources } from "../types/replyContextSources";
import { hasReplyContextSourcesToShow } from "../types/replyContextSources";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import { AiReplyContextSources } from "./AiReplyContextSources";
import { StreamingTimer } from "./StreamingTimer";
import { ThoughtSavePopover } from "./ThoughtSavePopover";
import type { ThoughtMgmtChatMessage } from "../hooks/useThoughtMgmtAiConversations";
import "./AiConversationPanel.css";

const DEPTH_MODE = "deep" as const;

type StartStreamResponse = {
  sessionId: string;
  resolvedDepth?: typeof DEPTH_MODE;
  replyContextSources: ReplyContextSources;
  providerLabel: string;
  modelName: string;
};

type VaultCfgForSend = {
  ai?: {
    activeProviderId?: string;
    providers?: ProviderProfileForUi[];
    privacy?: { allowPrivateContentInLocalLlm?: boolean };
  };
};

function retractInterruptedTurn(prev: ThoughtMgmtChatMessage[]): ThoughtMgmtChatMessage[] {
  const next = [...prev];
  const last = next[next.length - 1];
  if (last?.role === "assistant" && last.streaming) {
    next.pop();
  }
  const u = next[next.length - 1];
  if (u?.role === "user") {
    next.pop();
  }
  return next;
}

function finalizeStreamingAssistant(prev: ThoughtMgmtChatMessage[]): ThoughtMgmtChatMessage[] {
  const next = [...prev];
  const last = next[next.length - 1];
  if (last?.role === "assistant" && last.streaming) {
    next[next.length - 1] = { ...last, streaming: false };
  }
  return next;
}

/**
 * 合并流式增量，兼容重复监听或全量片段误入增量通道的场景。
 * 规则与主会话面板保持一致，避免出现重复叠词。
 */
function mergeStreamDelta(current: string, delta: string): string {
  if (!delta) return current;
  if (!current) return delta;
  if (current.endsWith(delta)) return current;
  if (delta.startsWith(current)) return delta;

  const maxOverlap = Math.min(current.length, delta.length);
  for (let overlap = maxOverlap; overlap > 0; overlap -= 1) {
    if (current.endsWith(delta.slice(0, overlap))) {
      return current + delta.slice(overlap);
    }
  }
  return current + delta;
}

function IconSendPlane() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M22 2 11 13" />
      <path d="M22 2 15 22 11 13 2 9 22 2z" />
    </svg>
  );
}

function IconStopSquare() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden={true}>
      <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />
    </svg>
  );
}

function IconCopyClipboard() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
      <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
    </svg>
  );
}

function IconSaveThought() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
    </svg>
  );
}

export type ThoughtMgmtAiConversationPanelProps = {
  /** 来自详情编辑区：含未保存草稿 */
  thoughtFocusFromDetail: ThoughtFocusContext | null;
  linkedNoteRelPath: string | null;
};

export function ThoughtMgmtAiConversationPanel({
  thoughtFocusFromDetail,
  linkedNoteRelPath,
}: ThoughtMgmtAiConversationPanelProps) {
  const { t } = useTranslation();
  const { openMarkdownTab } = useAiNoteContext();
  const {
    conversationId,
    messages,
    setMessages,
    sessionReady,
    markNeedPersist,
    isStreaming,
    setIsStreaming,
    workspaceReady,
    tauriRuntime,
    isVaultSearching,
    setIsVaultSearching,
    vaultSearchEpochRef,
    setThoughtFocusContext,
  } = useThoughtMgmtAiConversationSession();

  useEffect(() => {
    setThoughtFocusContext(thoughtFocusFromDetail);
  }, [thoughtFocusFromDetail, setThoughtFocusContext]);

  const dragExcludeProps = tauriRuntime
    ? ({ "data-tauri-drag-region-exclude": true } as const)
    : {};
  const dragProps = tauriRuntime ? ({ "data-tauri-drag-region": true } as const) : {};

  const [input, setInput] = useState("");
  const composerInputRef = useRef(input);
  composerInputRef.current = input;

  const [errorBanner, setErrorBanner] = useState<string | null>(null);
  const [copyToast, setCopyToast] = useState<"copied" | "failed" | null>(null);
  const [privacyHint, setPrivacyHint] = useState<string | null>(null);
  const [vaultSearchSummary, setVaultSearchSummary] = useState<string | null>(null);
  /** 工作区「包含上下文」提示条：默认单行，可展开查看全文 */
  const [vaultContextReminderExpanded, setVaultContextReminderExpanded] = useState(false);

  const activeSessionRef = useRef<string | null>(null);
  const listEndRef = useRef<HTMLDivElement>(null);

  const [savePopoverMsgId, setSavePopoverMsgId] = useState<string | null>(null);
  const [savePopoverContent, setSavePopoverContent] = useState<string>("");
  const [thoughtSaveToast, setThoughtSaveToast] = useState<"saved" | "failed" | null>(null);

  useEffect(() => {
    setVaultSearchSummary(null);
    setSavePopoverMsgId(null);
    setVaultContextReminderExpanded(false);
  }, [conversationId]);

  useEffect(() => {
    setVaultContextReminderExpanded(false);
  }, [vaultSearchSummary]);

  const scrollToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      listEndRef.current?.scrollIntoView({ block: "end", behavior: "smooth" });
    });
  }, []);

  useEffect(() => {
    scrollToBottom();
  }, [messages, scrollToBottom]);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }

    let disposed = false;
    const pending: UnlistenFn[] = [];

    void Promise.all([
      listen<{ sessionId: string; delta: string }>("llm:stream-chunk", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant" && last.streaming) {
            const isFirstToken = last.content === "";
            next[next.length - 1] = {
              ...last,
              content: mergeStreamDelta(last.content, p.delta),
              meta: isFirstToken && last.meta?.timing
                ? { ...last.meta, timing: { ...last.meta.timing, firstTokenMs: Date.now() } }
                : last.meta,
            };
          }
          return next;
        });
      }),
      listen<{ sessionId: string }>("llm:stream-done", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
        markNeedPersist();
        activeSessionRef.current = null;
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant" && last.streaming) {
            next[next.length - 1] = {
              ...last,
              streaming: false,
              meta: last.meta?.timing
                ? { ...last.meta, timing: { ...last.meta.timing, endMs: Date.now() } }
                : last.meta,
            };
          }
          return next;
        });
      }),
      listen<{ sessionId: string; code?: string; message: string }>("llm:stream-error", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
        markNeedPersist();
        activeSessionRef.current = null;
        setIsStreaming(false);
        if (p.code === "cancelled") {
          setMessages((prev) => {
            if (composerInputRef.current.trim().length > 0) {
              return finalizeStreamingAssistant(prev);
            }
            let lastUser = "";
            for (let i = prev.length - 1; i >= 0; i--) {
              if (prev[i].role === "user") {
                lastUser = prev[i].content;
                break;
              }
            }
            const next = retractInterruptedTurn(prev);
            if (lastUser.length > 0) {
              queueMicrotask(() => setInput(lastUser));
            }
            return next;
          });
          return;
        }
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant" && last.streaming) {
            next[next.length - 1] = { ...last, streaming: false };
          }
          return next;
        });
        setErrorBanner(p.message);
      }),
    ]).then((unlisteners) => {
      if (disposed) {
        unlisteners.forEach((u) => void u());
        return;
      }
      pending.push(...unlisteners);
    });

    return () => {
      disposed = true;
      pending.forEach((u) => void u());
    };
  }, [markNeedPersist, setMessages, setIsStreaming]);

  const handleStop = useCallback(async () => {
    if (isVaultSearching) {
      vaultSearchEpochRef.current += 1;
      setIsVaultSearching(false);
      setVaultSearchSummary(null);
      return;
    }

    const sid = activeSessionRef.current;
    if (!sid || !isTauri()) {
      return;
    }

    const hasComposerDraft = input.trim().length > 0;

    markNeedPersist();
    activeSessionRef.current = null;
    setIsStreaming(false);
    setErrorBanner(null);
    if (hasComposerDraft) {
      setMessages((prev) => finalizeStreamingAssistant(prev));
    } else {
      for (let i = messages.length - 1; i >= 0; i--) {
        if (messages[i].role === "user") {
          const lastUser = messages[i].content;
          if (lastUser.length > 0) {
            setInput(lastUser);
          }
          break;
        }
      }
      setMessages((prev) => retractInterruptedTurn(prev));
    }
    try {
      await invoke("abort_llm_stream", { sessionId: sid });
    } catch {
      /* 忽略 */
    }
  }, [input, messages, markNeedPersist, setMessages, setIsStreaming, isVaultSearching, setIsVaultSearching]);

  const handleSend = useCallback(async () => {
    setCopyToast(null);
    const trimmed = input.trim();
    if (!trimmed || isStreaming || isVaultSearching) {
      return;
    }
    if (!isTauri()) {
      setErrorBanner(t("aiPanel.onlyDesktop"));
      return;
    }
    if (!workspaceReady || !sessionReady || !conversationId) {
      setErrorBanner(t("aiPanel.openFolder"));
      return;
    }

    let noteContext: { relPath: string; markdownForGate: string } | null = null;
    if (linkedNoteRelPath && linkedNoteRelPath.trim()) {
      try {
        const md = await invoke<string>("read_markdown_file", { relPath: linkedNoteRelPath.trim() });
        noteContext = { relPath: linkedNoteRelPath.trim(), markdownForGate: md };
      } catch {
        noteContext = null;
      }
    }

    const userMsg: ThoughtMgmtChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: trimmed,
    };
    const nextChat = [...messages, userMsg];
    const chatTurns = nextChat.map((m) => ({ role: m.role, content: m.content }));

    let modelName = "";
    let allowPrivateLocal = false;
    try {
      const cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
      const p = cfg.ai?.providers?.find((x) => x.id === cfg.ai?.activeProviderId);
      modelName = (p?.lastUsedModel?.trim() || p?.defaultModel?.trim()) ?? "";
      allowPrivateLocal = cfg.ai?.privacy?.allowPrivateContentInLocalLlm === true;
      if (!modelName) {
        setErrorBanner(t("aiPanel.noModel"));
        return;
      }
    } catch {
      /* 仍尝试发流 */
    }

    if (
      noteContext &&
      markdownTreatAsKfPrivateForUi(noteContext.markdownForGate) &&
      !allowPrivateLocal
    ) {
      setPrivacyHint(t("aiPanel.privacyPlaceholder"));
    } else {
      setPrivacyHint(null);
    }

    vaultSearchEpochRef.current += 1;
    const searchEpoch = vaultSearchEpochRef.current;

    let vaultSearchResult: SearchWorkspaceContextResponse | null = null;
    if (workspaceReady) {
      setIsVaultSearching(true);
      setVaultSearchSummary(null);
      try {
        const excludeRelPaths =
          noteContext != null ? [noteContext.relPath] : ([] as string[]);
        vaultSearchResult = await invoke<SearchWorkspaceContextResponse>("search_workspace_context", {
          args: {
            query: trimmed,
            excludeRelPaths,
          },
        });
      } catch (e) {
        console.error(e);
        vaultSearchResult = null;
      } finally {
        if (vaultSearchEpochRef.current === searchEpoch) {
          setIsVaultSearching(false);
        }
      }
    }

    if (vaultSearchEpochRef.current !== searchEpoch) {
      return;
    }

    if (vaultSearchResult != null && vaultSearchResult.snippets.length > 0) {
      const paths = vaultSearchResult.snippets.map((s) => s.relPath).join(", ");
      const priv = vaultSearchResult.snippets.filter((s) => s.kind === "privateOmitted").length;
      const m = vaultSearchResult.meta;
      let line = t("aiPanel.vaultLine", {
        paths,
        scannedFiles: m.scannedFiles,
        elapsedMs: m.elapsedMs,
      });
      if (priv > 0) {
        line += ` ${t("aiPanel.vaultPrivateOmitted", { count: priv })}`;
      }
      if (m.stoppedEarly) {
        line += ` ${t("aiPanel.vaultStoppedEarly")}`;
      }
      setVaultSearchSummary(line);
    } else {
      setVaultSearchSummary(null);
    }

    setErrorBanner(null);
    setInput("");
    setMessages(nextChat);

    const tf = thoughtFocusFromDetail;

    try {
      const streamArgs: {
        messages: { role: string; content: string }[];
        noteContext?: { relPath: string; markdownForGate: string };
        vaultContext?: { snippets: SearchWorkspaceContextResponse["snippets"] };
        depthMode: typeof DEPTH_MODE;
        thoughtFocusContext?: ThoughtFocusContext;
      } = { messages: chatTurns, depthMode: DEPTH_MODE };
      if (noteContext) {
        streamArgs.noteContext = noteContext;
      }
      if (vaultSearchResult != null && vaultSearchResult.snippets.length > 0) {
        streamArgs.vaultContext = { snippets: vaultSearchResult.snippets };
      }
      if (tf != null && tf.thoughtId.trim() !== "" && tf.thoughtBody.trim() !== "") {
        streamArgs.thoughtFocusContext = tf;
      }
      const res = await invoke<StartStreamResponse>("start_chat_stream", {
        args: streamArgs,
      });
      if (vaultSearchEpochRef.current !== searchEpoch) {
        void invoke("abort_llm_stream", { sessionId: res.sessionId }).catch(() => {});
        return;
      }
      activeSessionRef.current = res.sessionId;
      setIsStreaming(true);
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: "",
          streaming: true,
          meta: {
            timing: { startMs: Date.now() },
            replyContextSources: res.replyContextSources,
            providerLabel: res.providerLabel,
            modelName: res.modelName,
          },
        },
      ]);
    } catch (e) {
      setPrivacyHint(null);
      setVaultSearchSummary(null);
      setErrorBanner(e instanceof Error ? e.message : String(e));
    }
  }, [
    conversationId,
    input,
    isStreaming,
    isVaultSearching,
    linkedNoteRelPath,
    messages,
    sessionReady,
    setMessages,
    setIsStreaming,
    setIsVaultSearching,
    thoughtFocusFromDetail,
    t,
    workspaceReady,
  ]);

  const onComposerKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key !== "Enter" || e.shiftKey) {
        return;
      }
      if (isStreaming || isVaultSearching) {
        return;
      }
      e.preventDefault();
      void handleSend();
    },
    [handleSend, isStreaming, isVaultSearching],
  );

  const copyAssistant = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopyToast("copied");
      window.setTimeout(() => setCopyToast(null), 1600);
    } catch {
      setCopyToast("failed");
      window.setTimeout(() => setCopyToast(null), 2200);
    }
  }, []);

  const handleThoughtSaved = useCallback(() => {
    setSavePopoverMsgId(null);
    setThoughtSaveToast("saved");
    window.setTimeout(() => setThoughtSaveToast(null), 1600);
  }, []);

  const canSend =
    isTauri() &&
    workspaceReady &&
    sessionReady &&
    !!conversationId &&
    input.trim().length > 0 &&
    !isStreaming &&
    !isVaultSearching;

  return (
    <section
      className="ai-chat thought-mgmt-ai-chat"
      aria-label={t("thoughtManagement.thoughtAiSectionAria")}
      data-ai-conversation={conversationId ?? ""}
    >
      {errorBanner ? (
        <div className="ai-chat__banner ai-chat__banner--error" role="alert" {...dragExcludeProps}>
          {errorBanner}
        </div>
      ) : null}

      {privacyHint ? (
        <div className="ai-chat__banner ai-chat__banner--hint" aria-live="polite" {...dragExcludeProps}>
          {privacyHint}
        </div>
      ) : null}

      {vaultSearchSummary ? (
        <div
          className={`ai-chat__banner ai-chat__banner--hint thought-mgmt-ai-chat__context-reminder${
            vaultContextReminderExpanded ? " is-expanded" : ""
          }`}
          role="region"
          aria-label={t("thoughtManagement.contextIncludedReminderAria")}
          aria-live="polite"
          {...dragExcludeProps}
        >
          <button
            type="button"
            className={`thought-mgmt-ai-chat__context-reminder-toggle${
              vaultContextReminderExpanded ? " is-expanded" : ""
            }`}
            aria-expanded={vaultContextReminderExpanded}
            title={
              vaultContextReminderExpanded
                ? t("thoughtManagement.contextIncludedReminderCollapse")
                : t("thoughtManagement.contextIncludedReminderExpand")
            }
            aria-label={
              vaultContextReminderExpanded
                ? t("thoughtManagement.contextIncludedReminderCollapse")
                : t("thoughtManagement.contextIncludedReminderExpand")
            }
            onClick={() => setVaultContextReminderExpanded((v) => !v)}
            {...dragExcludeProps}
          >
            <svg
              className="thought-mgmt-ai-chat__context-reminder-chevron"
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
              <path d="m9 18 6-6-6-6" />
            </svg>
          </button>
          <div
            className="thought-mgmt-ai-chat__context-reminder-body ai-chat__vault-refs"
            title={!vaultContextReminderExpanded ? vaultSearchSummary : undefined}
          >
            {vaultSearchSummary}
          </div>
        </div>
      ) : null}

      {isVaultSearching ? (
        <div className="ai-chat__banner ai-chat__banner--hint" aria-live="polite" {...dragExcludeProps}>
          {t("aiPanel.searchingVault")}
        </div>
      ) : null}

      <div className="ai-chat__messages" role="log" aria-label={t("aiPanel.messages")} aria-live="polite" {...dragProps}>
        {messages.length === 0 ? (
          <p className="ai-chat__empty" {...dragProps}>
            {t("thoughtManagement.thoughtAiEmptyMessages")}
          </p>
        ) : (
          messages.map((m) => (
            <div key={m.id} className={`ai-chat__row ai-chat__row--${m.role}`} {...dragExcludeProps}>
              <div className={`ai-chat__bubble ai-chat__bubble--${m.role}`}>
                {m.role === "assistant" ? (
                  <>
                    <AiAssistantMarkdown content={m.content} />
                    {!m.streaming && m.meta?.replyContextSources && hasReplyContextSourcesToShow(m.meta.replyContextSources) ? (
                      <AiReplyContextSources sources={m.meta.replyContextSources} onOpenMarkdown={openMarkdownTab} />
                    ) : null}
                    {m.streaming ? (
                      <span className="ai-chat__typing" aria-hidden={true}>
                        ▌
                      </span>
                    ) : null}
                    {!m.streaming && m.content.trim().length > 0 ? (
                      <>
                        <button
                          type="button"
                          className="ai-chat__copy"
                          onClick={() => void copyAssistant(m.content)}
                          aria-label={t("aiPanel.copyAria")}
                          title={t("aiPanel.copyMd")}
                          {...dragExcludeProps}
                        >
                          <IconCopyClipboard />
                        </button>
                        <button
                          type="button"
                          className="ai-chat__copy"
                          onClick={() => {
                            setSavePopoverContent(m.content);
                            setSavePopoverMsgId(m.id);
                          }}
                          aria-label={t("thoughtSave.buttonTitle")}
                          title={t("thoughtSave.buttonTitle")}
                          {...dragExcludeProps}
                        >
                          <IconSaveThought />
                        </button>
                      </>
                    ) : null}
                    {m.meta?.timing ? (
                      <StreamingTimer timing={m.meta.timing} streaming={!!m.streaming} modelName={m.meta.modelName} />
                    ) : null}
                  </>
                ) : (
                  <div className="ai-chat__user-stack">
                    <p className="ai-chat__user-text">{m.content}</p>
                  </div>
                )}
              </div>
            </div>
          ))
        )}

        {savePopoverMsgId ? (
          <ThoughtSavePopover
            content={savePopoverContent}
            defaultRelPath={linkedNoteRelPath}
            isSelection={true}
            onSaved={handleThoughtSaved}
            onCancel={() => setSavePopoverMsgId(null)}
          />
        ) : null}

        <div ref={listEndRef} />
      </div>

      <div className="ai-chat__composer" {...dragExcludeProps}>
        <div className="ai-chat__composer-field">
          <div className="ai-chat__composer-stack">
            <textarea
              className="ai-chat__input"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onComposerKeyDown}
              placeholder={
                !isTauri()
                  ? t("aiPanel.placeholderDesktop")
                  : !workspaceReady
                    ? t("aiPanel.placeholderOpenFolder")
                    : isStreaming || isVaultSearching
                      ? t("aiPanel.placeholderStreaming")
                      : t("thoughtManagement.thoughtAiPlaceholder")
              }
              disabled={!isTauri() || !workspaceReady}
              rows={3}
              aria-label={t("aiPanel.messageLabel")}
            />
            <div className="ai-chat__composer-toolbar" {...dragExcludeProps}>
              <div className="ai-chat__composer-actions">
                <button
                  type="button"
                  className={
                    isStreaming || isVaultSearching
                      ? "ai-chat__submit ai-chat__submit--streaming"
                      : "ai-chat__submit ai-chat__submit--send"
                  }
                  disabled={!isStreaming && !isVaultSearching && !canSend}
                  onClick={() =>
                    isStreaming || isVaultSearching ? void handleStop() : void handleSend()
                  }
                  aria-label={
                    isStreaming
                      ? t("aiPanel.stopGen")
                      : isVaultSearching
                        ? t("aiPanel.cancelSearch")
                        : t("aiPanel.send")
                  }
                  title={
                    isStreaming
                      ? t("aiPanel.stopTitle")
                      : isVaultSearching
                        ? t("aiPanel.cancelSearchTitle")
                        : t("aiPanel.sendTitle")
                  }
                  {...dragExcludeProps}
                >
                  {isStreaming ? <IconStopSquare /> : <IconSendPlane />}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {copyToast ? (
        <div
          className={
            copyToast === "failed"
              ? "ai-chat__copy-toast ai-chat__copy-toast--error"
              : "ai-chat__copy-toast ai-chat__copy-toast--success"
          }
          role="status"
          aria-live="polite"
          {...dragExcludeProps}
        >
          {copyToast === "failed" ? t("aiPanel.copyFailed") : t("aiPanel.copied")}
        </div>
      ) : null}

      {thoughtSaveToast ? (
        <div
          className={
            thoughtSaveToast === "failed"
              ? "ai-chat__copy-toast ai-chat__copy-toast--error"
              : "ai-chat__copy-toast ai-chat__copy-toast--success"
          }
          role="status"
          aria-live="polite"
          {...dragExcludeProps}
        >
          {thoughtSaveToast === "saved" ? t("thoughtSave.saved") : t("thoughtSave.saveFailed")}
        </div>
      ) : null}
    </section>
  );
}
