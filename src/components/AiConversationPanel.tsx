import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { useAiConversationSession } from "../contexts/AiConversationSessionContext";
import type { ThoughtFocusContext } from "../types/aiConversation";
import { useAiNoteContext } from "../contexts/AiNoteContext";
import type { ChatMessage, ToolCallDisplayInfo } from "../hooks/useWorkspaceAiConversations";
import type { ReplyContextSources } from "../types/replyContextSources";
import { hasReplyContextSourcesToShow } from "../types/replyContextSources";
import type { SearchWorkspaceContextResponse } from "../types/vaultContextSearch";
import type {
  AutoResolvedDepth,
  ThoughtRetrievalResult,
  CountVaultThoughtsForReviewResponse,
  GenerateChallengeQuestionResponse,
} from "../types/cognitiveTypes";
import type { DetectPassiveHighlightResponse, PassiveHighlightMarked } from "../types/passiveHighlight";
import type { ActiveProvider, VaultConfigForUi } from "../types/vaultAiConfig";
import { isChallengeInlineLlmReady } from "../utils/isChallengeReviewLlmReady";
import { VAULT_CONFIG_UPDATED_EVENT } from "../utils/vaultConfigBroadcast";
import { markdownTreatAsKfPrivateForUi } from "../utils/kfPrivateMarkdown";
import { PrivacyChangeOverlay } from "./PrivacyChangeOverlay";
import { AiToolApprovalDialog } from "./AiToolApprovalDialog";
import type { ApprovalRequest } from "../types/toolTypes";
import { respondToolApproval } from "../utils/toolInvoke";
import { listSkills, invokeSkill } from "../utils/skillInvoke";
import type { SkillManifestJson } from "../types/skillTypes";
import { parseSlashCommand } from "../utils/parseSlashCommand";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import { StreamingTimer } from "./StreamingTimer";
import { useCognitiveFrequencyControl } from "../hooks/useCognitiveFrequencyControl";
import { DepthSlider } from "./DepthSlider";
import { ChallengeReviewInline } from "./ChallengeReviewInline";
import { InviteAfterAnswer } from "./InviteAfterAnswer";
import { ThoughtSavePopover } from "./ThoughtSavePopover";
import { PassiveHighlightSaveCue } from "./PassiveHighlightSaveCue";
import { AiReplyContextSources } from "./AiReplyContextSources";
import { stripMarkedPassiveHighlightWithCount } from "../utils/passiveHighlightLifecycle";
import { trackKnowforgeEvent } from "../utils/knowforgeAnalytics";
import { getAppLocale } from "../i18n";
import "./AiConversationPanel.css";

/** 用户中止且输入框无新草稿：去掉末尾 streaming 助手及其前一条用户消息 */
function retractInterruptedTurn(prev: ChatMessage[]): ChatMessage[] {
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

/** 用户中止但保留会话：仅将末尾 streaming 助手标为结束，保留已生成片段 */
function finalizeStreamingAssistant(prev: ChatMessage[]): ChatMessage[] {
  const next = [...prev];
  const last = next[next.length - 1];
  if (last?.role === "assistant" && last.streaming) {
    next[next.length - 1] = { ...last, streaming: false };
  }
  return next;
}

/**
 * 合并流式增量，兼容重复监听或上游偶发返回「全量片段」导致的重复拼接。
 * - `delta` 与当前尾部完全重复：忽略；
 * - `delta` 以当前全文开头：视为全量快照，直接替换为 `delta`；
 * - 其余情况按最大前后缀重叠合并，避免出现「您您 在在」这类叠字。
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

type StartStreamResponse = {
  sessionId: string;
  /** 后端在 depthMode=auto 时返回的解析档位 */
  resolvedDepth?: AutoResolvedDepth;
  replyContextSources: ReplyContextSources;
};

type VaultCfgForSend = {
  ai?: {
    activeProvider?: ActiveProvider;
    ollama?: { defaultModel?: string; lastUsedModel?: string };
    privacy?: { allowPrivateContentInLocalLlm?: boolean };
  };
  cognitive?: {
    passiveHighlightEnabled?: boolean;
    passiveHighlightConfidenceMin?: number;
  };
};

type InviteData = { thought: ThoughtRetrievalResult | null; question: string };

type ChallengeInlineData = {
  thought: ThoughtRetrievalResult;
  question: string;
  templateKind: string;
};

/** 纸飞机发送图标 */
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

/** 流式生成中：绿色按钮内白色停止块（点击即停止） */
function IconStopSquare() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden={true}>
      <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" />
    </svg>
  );
}

/** 复制助手消息为 Markdown */
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

/** 保存为想法图标（书签） */
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

/** 横幅提示手动关闭（X） */
function IconDismissBanner() {
  return (
    <svg
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
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

/** Enter 发送、Shift+Enter 换行（默认约定，见 onComposerKeyDown） */

export function AiConversationPanel() {
  const { t } = useTranslation();
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
    includeVaultContext,
    setIncludeVaultContext,
    isVaultSearching,
    setIsVaultSearching,
    vaultSearchEpochRef,
    depthMode,
    setDepthMode,
    autoResolved,
    setAutoResolved,
    enoughForThisChat,
    setEnoughForThisChat,
    createConversation,
    thoughtFocusContext,
  } = useAiConversationSession();

  /** 与 stream 事件监听同步，避免闭包读到陈旧的「本会话够了」 */
  const enoughForThisChatRef = useRef(enoughForThisChat);
  enoughForThisChatRef.current = enoughForThisChat;

  /** stream-done 内读取最新深度，避免监听 effect 未订阅 depth 导致 Auto→浅 仍检索 */
  const depthModeForInviteRef = useRef(depthMode);
  const autoResolvedForInviteRef = useRef(autoResolved);
  depthModeForInviteRef.current = depthMode;
  autoResolvedForInviteRef.current = autoResolved;

  /** stream-done 时排除当前附件笔记，与 vault 检索一致 */
  const thoughtInviteExcludeRef = useRef<string[]>([]);

  /** 追踪本轮对话中已发送给 AI 的文档路径（noteContext + vault 检索） */
  const sharedDocPathsRef = useRef<Set<string>>(new Set());
  /** 隐私变更警告：某文档曾在本对话中分享但现在被标为私密 */
  const [privacyChangeWarning, setPrivacyChangeWarning] = useState<string | null>(null);

  const dragExcludeProps = tauriRuntime
    ? ({ "data-tauri-drag-region-exclude": true } as const)
    : {};
  const dragProps = tauriRuntime ? ({ "data-tauri-drag-region": true } as const) : {};

  const { attachCurrentNote, setAttachCurrentNote, getCurrentNoteContext, activePath, openMarkdownTab } =
    useAiNoteContext();

  const attachCitationToLastAssistant = useCallback(
    (thought: ThoughtRetrievalResult | null) => {
      if (!thought || thought.privateOmitted || !thought.thoughtId || !thought.excerpt) {
        return;
      }
      setMessages((prev) => {
        const next = [...prev];
        const li = next.length - 1;
        const la = next[li];
        if (la?.role !== "assistant") {
          return prev;
        }
        next[li] = { ...la, meta: { ...la.meta, thoughtCitation: thought } };
        return next;
      });
    },
    [setMessages],
  );

  const [input, setInput] = useState("");
  /** 供事件监听读取最新输入，避免闭包陈旧（与中止时「是否有草稿」策略一致） */
  const composerInputRef = useRef(input);
  composerInputRef.current = input;

  const [errorBanner, setErrorBanner] = useState<string | null>(null);
  const [copyToast, setCopyToast] = useState<"copied" | "failed" | null>(null);
  const [privacyHint, setPrivacyHint] = useState<string | null>(null);
  const [vaultSearchSummary, setVaultSearchSummary] = useState<string | null>(null);
  const [isPlanning, setIsPlanning] = useState(false);

  const activeSessionRef = useRef<string | null>(null);
  const listEndRef = useRef<HTMLDivElement>(null);

  /** Iter 5 #4：工具调用总开关从 vault config 读取（默认 true,旧 vault 缺字段时取 true）。
   *  通过 VAULT_CONFIG_UPDATED_EVENT 在设置保存后实时同步,无需重开会话。 */
  const [toolsEnabled, setToolsEnabled] = useState(true);
  /** 发送时快照：agent 模式下 stream-done 仅是中间信号，不能最终化助手消息 */
  const isAgentModeRef = useRef(false);

  /** Iter 5 #4: while a `skill.<id>` auto-invocation is in flight, save the
   * parent session here so we can restore activeSessionRef once the skill
   * sub-turn fires `agent-done`. Null when no skill is currently in progress. */
  const parentSessionRef = useRef<string | null>(null);
  /** 映射：skillSessionId → { parentToolCallId } */
  const skillSessionMapRef = useRef<Map<string, { parentToolCallId: string }>>(new Map());

  /** Iter 5 #3：内置 skill manifest 缓存。skill 在后端 setup() 注册一次后不变，挂载时取一次即可。 */
  const [skillsCache, setSkillsCache] = useState<SkillManifestJson[]>([]);
  useEffect(() => {
    if (!isTauri()) return;
    listSkills()
      .then(setSkillsCache)
      .catch(() => {
        /* 拉取失败不阻塞，handleSkillSend 会兜底报错 */
      });
  }, []);

  /** Iter 5 #4：从 vault config 拉取 toolsEnabled,并订阅设置保存广播以热更新。 */
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    const reload = async () => {
      try {
        const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
        if (disposed) return;
        setToolsEnabled(cfg.ai.toolsEnabled !== false);
      } catch {
        /* 加载失败保持默认 true,避免阻塞用户 */
      }
    };
    void reload();
    const onCfg = () => void reload();
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
    return () => {
      disposed = true;
      window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
    };
  }, []);

  /** Iter 5 #3 后续：斜杠命令自动补全。输入以 `/` 开头且仍在打 id 时弹下拉。 */
  const composerRef = useRef<HTMLTextAreaElement | null>(null);
  const [slashIndex, setSlashIndex] = useState(0);
  const [slashDismissed, setSlashDismissed] = useState(false);

  type SlashCandidate = { value: string; label: string; description: string };
  const slashCandidates = useMemo<SlashCandidate[]>(() => {
    // 仅当用户还在打 id 段（无空格 body）时给出建议；body 一旦开始就交给 parseSlashCommand。
    const m = /^\/([a-z0-9_]*)$/i.exec(input);
    if (!m) return [];
    const query = m[1].toLowerCase();
    const items: SlashCandidate[] = [];
    for (const s of skillsCache) {
      const idLc = s.id.toLowerCase();
      const nameLc = s.name.toLowerCase();
      if (!query || idLc.startsWith(query) || nameLc.includes(query)) {
        items.push({
          value: `/${s.id} `,
          label: `/${s.id}`,
          description: `${s.name} — ${s.description}`,
        });
      }
    }
    if (!query || "skills".startsWith(query)) {
      items.push({
        value: "/skills",
        label: "/skills",
        description: t("aiPanel.skillsListTitle"),
      });
    }
    return items;
  }, [input, skillsCache, t]);

  const slashOpen = !slashDismissed && slashCandidates.length > 0;
  const safeSlashIndex = Math.min(slashIndex, Math.max(0, slashCandidates.length - 1));

  const acceptSlashCandidate = useCallback((c: SlashCandidate) => {
    setInput(c.value);
    setSlashDismissed(true);
    setSlashIndex(0);
    composerRef.current?.focus();
  }, []);

  /** P3 审批：按到达顺序串行展示弹窗，避免并行 tool_calls 时 UI 叠加 */
  const approvalQueueRef = useRef<ApprovalRequest[]>([]);
  const [activeApproval, setActiveApproval] = useState<ApprovalRequest | null>(null);
  const dequeueNextApproval = useCallback(() => {
    setActiveApproval(() => {
      const q = approvalQueueRef.current;
      return q.length > 0 ? q.shift()! : null;
    });
  }, []);
  const handleApprovalResolve = useCallback(
    (approvalId: string, decision: boolean) => {
      void respondToolApproval(approvalId, decision).catch(() => {});
      dequeueNextApproval();
    },
    [dequeueNextApproval],
  );
  // 切换会话时清掉 UI 状态；后端 ConfirmOncePerSession 缓存按 conv_id 维度保留，
  // 用户切回原会话时仍生效。
  useEffect(() => {
    approvalQueueRef.current = [];
    setActiveApproval(null);
  }, [conversationId]);

  /** invite-after-answer 状态 */
  const [inviteData, setInviteData] = useState<InviteData | null>(null);
  const [challengeInlineData, setChallengeInlineData] = useState<ChallengeInlineData | null>(null);
  const inviteSearchEpochRef = useRef(0);
  /** 缓存最近一次发送的用户查询，供 stream-done 回调读取 */
  const lastSentQueryRef = useRef("");

  /** 频控逻辑 */
  const freqCtrl = useCognitiveFrequencyControl();
  /** 用户回合计数（用于频控 shouldShowInvite 的 turnIndex） */
  const turnIndex = messages.filter((m) => m.role === "user").length;

  /** Phase 4E: 保存为想法 popover 状态 */
  const [savePopoverMsgId, setSavePopoverMsgId] = useState<string | null>(null);
  const [savePopoverContent, setSavePopoverContent] = useState<string>("");
  const [savePopoverVariant, setSavePopoverVariant] = useState<"default" | "passive">("default");
  const [savePopoverUserMsgId, setSavePopoverUserMsgId] = useState<string | null>(null);
  const [thoughtSaveToast, setThoughtSaveToast] = useState<"saved" | "failed" | null>(null);
  /** 被动高亮门控横幅（非错误） */
  const [passiveHighlightBanner, setPassiveHighlightBanner] = useState<null | "noOllama" | "short" | "cap">(null);

  const passiveHighlightTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** 会话切换 / 卸载时递增，使已排队的 setTimeout 与 await 后续逻辑与当前生命周期对齐 */
  const passiveHighlightEpochRef = useRef(0);
  const passiveHighlightMarkedCountRef = useRef(0);
  const conversationIdForPassiveRef = useRef<string | null>(null);
  conversationIdForPassiveRef.current = conversationId;

  /** 选区浮动工具栏状态 */
  const [selToolbar, setSelToolbar] = useState<{
    msgId: string;
    text: string;
    top: number;
    left: number;
  } | null>(null);
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  /** selectionchange 回调读取，避免 messages 高频变更时反复 add/remove document 监听 */
  const messagesRef = useRef(messages);
  messagesRef.current = messages;

  /** 面板卸载后置 true，避免 await 后继续 setState（与 AiLlmSettingsModal disposedRef 一致） */
  const disposedRef = useRef(false);
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
      passiveHighlightEpochRef.current += 1;
      if (passiveHighlightTimerRef.current) {
        clearTimeout(passiveHighlightTimerRef.current);
        passiveHighlightTimerRef.current = null;
      }
    };
  }, []);

  /** 组件挂载/workspace 就绪时从磁盘加载频控状态 */
  useEffect(() => {
    void freqCtrl.reload();
  }, [workspaceReady]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    passiveHighlightEpochRef.current += 1;
    if (passiveHighlightTimerRef.current) {
      clearTimeout(passiveHighlightTimerRef.current);
      passiveHighlightTimerRef.current = null;
    }
    passiveHighlightMarkedCountRef.current = 0;
    setPassiveHighlightBanner(null);
    setSavePopoverVariant("default");
    setSavePopoverUserMsgId(null);
    setVaultSearchSummary(null);
    setInviteData(null);
    setChallengeInlineData(null);
    setEnoughForThisChat(false);
    setAutoResolved(null);
    setSavePopoverMsgId(null);
    setSelToolbar(null);
    setPrivacyChangeWarning(null);
    sharedDocPathsRef.current = new Set();
  }, [conversationId, setAutoResolved, setEnoughForThisChat]);

  /** 被动高亮门控横幅：展示 5 秒后自动收起（手动关闭见横幅按钮） */
  useEffect(() => {
    if (!passiveHighlightBanner) return;
    const id = window.setTimeout(() => {
      setPassiveHighlightBanner(null);
    }, 5000);
    return () => window.clearTimeout(id);
  }, [passiveHighlightBanner]);

  // ---- 监听 kf-private 变更事件：检查是否涉及已分享文档 ----
  useEffect(() => {
    const onPrivacyChange = (e: Event) => {
      const detail = (e as CustomEvent<{ relPath: string }>).detail;
      if (detail?.relPath && sharedDocPathsRef.current.has(detail.relPath)) {
        setPrivacyChangeWarning(detail.relPath);
      }
    };
    window.addEventListener("kf-private-changed", onPrivacyChange);
    return () => window.removeEventListener("kf-private-changed", onPrivacyChange);
  }, []);

  // ---- 选区浮动工具栏：监听 selectionchange ----
  useEffect(() => {
    const onSelChange = () => {
      const sel = window.getSelection();
      if (!sel || sel.isCollapsed || !sel.toString().trim()) {
        setSelToolbar(null);
        return;
      }
      const container = messagesContainerRef.current;
      if (!container) { setSelToolbar(null); return; }
      // 选区必须在 assistant bubble 内
      const anchor = sel.anchorNode;
      const focus = sel.focusNode;
      if (!anchor || !focus) { setSelToolbar(null); return; }
      const bubble = anchor.parentElement?.closest?.(".ai-chat__bubble--assistant")
        ?? (anchor as HTMLElement).closest?.(".ai-chat__bubble--assistant");
      if (!bubble || !bubble.contains(focus)) { setSelToolbar(null); return; }
      // 查找对应的消息 row 获取 msgId
      const row = bubble.closest(".ai-chat__row") as HTMLElement | null;
      if (!row) { setSelToolbar(null); return; }
      // msgId 从 data attribute 或 key 获取不了，改为从 messages 数组按 DOM 索引查找
      const allRows = container.querySelectorAll(".ai-chat__row");
      let rowIndex = -1;
      allRows.forEach((r, i) => { if (r === row) rowIndex = i; });
      const latestMessages = messagesRef.current;
      if (rowIndex < 0 || rowIndex >= latestMessages.length) { setSelToolbar(null); return; }
      const msg = latestMessages[rowIndex];
      if (!msg || msg.role !== "assistant") { setSelToolbar(null); return; }
      // 计算浮动位置（相对于 messages container）
      const range = sel.getRangeAt(0);
      const rect = range.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      setSelToolbar({
        msgId: msg.id,
        text: sel.toString().trim(),
        top: rect.top - containerRect.top - 36 + container.scrollTop,
        left: Math.max(0, rect.left - containerRect.left + rect.width / 2 - 60),
      });
    };
    document.addEventListener("selectionchange", onSelChange);
    return () => document.removeEventListener("selectionchange", onSelChange);
  }, []);

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
        const skillMapping = skillSessionMapRef.current.get(p.sessionId);
        if (skillMapping) {
          // Skill 会话的 chunk → 追加到父气泡的工具调用项的 skillContent
          setMessages((prev) => {
            const next = [...prev];
            for (let i = next.length - 1; i >= 0; i--) {
              const m = next[i];
              if (m.role !== "assistant") continue;
              const tcs = m.meta?.toolCalls;
              if (!tcs) continue;
              const tcIdx = tcs.findIndex((tc) => tc.toolCallId === skillMapping.parentToolCallId);
              if (tcIdx === -1) continue;
              const updatedCalls = [...tcs];
              updatedCalls[tcIdx] = {
                ...updatedCalls[tcIdx],
                skillContent: (updatedCalls[tcIdx].skillContent || "") + p.delta,
              };
              next[i] = {
                ...m,
                meta: { ...m.meta, toolCalls: updatedCalls },
              };
              break;
            }
            return next;
          });
          return;
        }
        // 主会话的 chunk → 追加到气泡 content（原有逻辑）
        setMessages((prev) => {
          let next = [...prev];
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
        // P2 Tool Calling Loop：agent 模式下 stream-done 只是轮次中的中间信号，
        // 后续还会有 tool-call-* 与后续文本输出，只能由 llm:agent-done 最终化。
        if (isAgentModeRef.current) {
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
          const strippedPack = stripMarkedPassiveHighlightWithCount(next);
          if (strippedPack.stripped > 0) {
            passiveHighlightMarkedCountRef.current = Math.max(
              0,
              passiveHighlightMarkedCountRef.current - strippedPack.stripped,
            );
          }
          return strippedPack.messages;
        });

        // 后台：先答后邀优先；否则在满足频控时尝试挑战式回顾（通道二）
        const epoch = ++inviteSearchEpochRef.current;
        setInviteData(null);
        setChallengeInlineData(null);
        const query = lastSentQueryRef.current.trim();
        if (!query || enoughForThisChatRef.current) return;
        const dm = depthModeForInviteRef.current;
        const ar = autoResolvedForInviteRef.current;
        if (dm === "shallow" || (dm === "auto" && ar === "shallow")) {
          return;
        }
        void (async () => {
          try {
            type SearchResp = {
              thought: ThoughtRetrievalResult | null;
              thoughts?: ThoughtRetrievalResult[];
              meta: { scannedFiles: number; stoppedEarly: boolean; elapsedMs: number };
            };

            const turnIdx = messagesRef.current.filter((m) => m.role === "user").length;
            const inviteEligible = freqCtrl.shouldShowInvite(
              dm,
              enoughForThisChatRef.current,
              turnIdx,
              ar,
            );

            if (inviteEligible) {
              const resp = await invoke<SearchResp>("search_thought_for_invite", {
                args: {
                  query,
                  excludeRelPaths: thoughtInviteExcludeRef.current,
                  maxResults: 1,
                },
              });
              if (inviteSearchEpochRef.current !== epoch || disposed) return;
              const excerptRaw = resp.thought?.excerpt;
              const question =
                excerptRaw && excerptRaw.length > 0
                  ? t("invite.thoughtQuestion", {
                      excerpt:
                        excerptRaw.length > 60 ? `${excerptRaw.slice(0, 60)}...` : excerptRaw,
                    })
                  : t("invite.defaultQuestion");
              setInviteData({ thought: resp.thought, question });
              attachCitationToLastAssistant(resp.thought ?? null);
              const th = resp.thought;
              if (
                th &&
                !th.privateOmitted &&
                th.thoughtId &&
                isTauri() &&
                th.relPath
              ) {
                void invoke("append_ai_thought_reference", {
                  args: {
                    relPath: th.relPath,
                    thoughtId: th.thoughtId,
                    context: query.slice(0, 2000),
                    relevance: "ai-conversation-invite",
                  },
                }).catch(() => {});
              }
              return;
            }

            let cfg: VaultCfgForSend;
            try {
              cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
            } catch {
              return;
            }
            if (inviteSearchEpochRef.current !== epoch || disposed) return;
            const prov = cfg.ai?.activeProvider ?? "ollama";
            if (prov !== "ollama") return;
            if (!isChallengeInlineLlmReady(cfg as VaultConfigForUi)) return;

            const countResp = await invoke<CountVaultThoughtsForReviewResponse>(
              "count_vault_thoughts_for_review",
            );
            if (inviteSearchEpochRef.current !== epoch || disposed) return;
            const total = countResp.totalThoughts;
            if (
              !freqCtrl.shouldShowChallengeInline(dm, ar, {
                inviteWillShow: false,
                thoughtId: null,
                vaultThoughtTotal: total,
              })
            ) {
              return;
            }

            const respMany = await invoke<SearchResp>("search_thought_for_invite", {
              args: {
                query,
                excludeRelPaths: thoughtInviteExcludeRef.current,
                maxResults: 3,
              },
            });
            if (inviteSearchEpochRef.current !== epoch || disposed) return;
            const pick =
              respMany.thoughts?.find((x) => x.excerpt && !x.privateOmitted) ?? respMany.thought;
            if (!pick?.excerpt || pick.privateOmitted) return;

            const gen = await invoke<GenerateChallengeQuestionResponse>("generate_challenge_question", {
              args: {
                thoughtExcerpt: pick.excerpt,
                relPath: pick.relPath,
                conversationQuery: query,
                depthMode: dm,
                uiLocale: getAppLocale(),
              },
            });
            if (inviteSearchEpochRef.current !== epoch || disposed) return;
            if (gen.shouldSkip || !gen.question.trim()) return;

            void freqCtrl.recordChallengeInlineShown(pick.thoughtId);
            void trackKnowforgeEvent("review.inline_shown", {
              thoughtId: pick.thoughtId,
              templateKind: gen.templateKind,
            });
            setChallengeInlineData({
              thought: pick,
              question: gen.question,
              templateKind: gen.templateKind,
            });
            attachCitationToLastAssistant(pick);
            if (!pick.privateOmitted && pick.thoughtId && isTauri()) {
              void invoke("append_ai_thought_reference", {
                args: {
                  relPath: pick.relPath,
                  thoughtId: pick.thoughtId,
                  context: query.slice(0, 2000),
                  relevance: "ai-challenge-inline",
                },
              }).catch(() => {});
            }
          } catch {
            // 超时或其他失败：静默跳过
          }
        })();
      }),
      listen<{ sessionId: string; code?: string; message: string }>("llm:stream-error", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
        // Iter 5 #4: error during a skill sub-turn — restore parent session.
        // The parent agent_loop will receive the SkillAsTool tool error in its
        // tool_result and decide whether to recover. Don't tear down top-level streaming UI here.
        if (parentSessionRef.current) {
          const parent = parentSessionRef.current;
          parentSessionRef.current = null;
          activeSessionRef.current = parent;
          setMessages((prev) => {
            const next = [...prev];
            const last = next[next.length - 1];
            if (last?.role === "assistant" && last.streaming) {
              next[next.length - 1] = { ...last, streaming: false };
            }
            return next;
          });
          return;
        }
        markNeedPersist();
        activeSessionRef.current = null;
        isAgentModeRef.current = false;
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
          const strippedPack = stripMarkedPassiveHighlightWithCount(next);
          if (strippedPack.stripped > 0) {
            passiveHighlightMarkedCountRef.current = Math.max(
              0,
              passiveHighlightMarkedCountRef.current - strippedPack.stripped,
            );
          }
          return strippedPack.messages;
        });
        setErrorBanner(p.message);
      }),
      // P2 Tool Calling Loop：工具调用开始 → 在末尾 assistant 消息中插入 running 状态项
      // Skill 内部的工具调用路由到对应 toolCall 的 skillToolCalls
      listen<{ sessionId: string; toolCallId: string; toolName: string; inputSummary?: string }>(
        "llm:tool-call-start",
        (e) => {
          const p = e.payload;
          if (p.sessionId !== activeSessionRef.current) {
            return;
          }
          const skillMapping = skillSessionMapRef.current.get(p.sessionId);
          const newCall: ToolCallDisplayInfo = {
            toolCallId: p.toolCallId,
            toolName: p.toolName,
            status: "running",
            inputSummary: p.inputSummary,
          };
          if (skillMapping) {
            // Skill 内部的工具调用 → 添加到对应的 skillToolCalls
            setMessages((prev) => {
              const next = [...prev];
              for (let i = next.length - 1; i >= 0; i--) {
                const m = next[i];
                if (m.role !== "assistant") continue;
                const tcs = m.meta?.toolCalls;
                if (!tcs) continue;
                const tcIdx = tcs.findIndex((tc) => tc.toolCallId === skillMapping.parentToolCallId);
                if (tcIdx === -1) continue;
                const updatedCalls = [...tcs];
                updatedCalls[tcIdx] = {
                  ...updatedCalls[tcIdx],
                  skillToolCalls: [...(updatedCalls[tcIdx].skillToolCalls || []), newCall],
                };
                next[i] = {
                  ...m,
                  meta: { ...m.meta, toolCalls: updatedCalls },
                };
                break;
              }
              return next;
            });
            return;
          }
          // 主会话的工具调用 → 添加到末尾 assistant 气泡的 toolCalls
          setMessages((prev) => {
            const next = [...prev];
            const last = next[next.length - 1];
            if (last?.role !== "assistant") return prev;
            const existing = last.meta?.toolCalls ?? [];
            return [
              ...next.slice(0, -1),
              {
                ...last,
                meta: { ...last.meta, toolCalls: [...existing, newCall] },
              },
            ];
          });
        },
      ),
      // P2 Tool Calling Loop：工具调用完成 → 更新对应 toolCallId 的状态为 done/error
      // 按新字段更新 resultSummary / durationMs / errorMessage，处理 Skill 内部工具调用
      listen<{ sessionId: string; toolCallId: string; success: boolean; resultSummary?: string; durationMs?: number; errorMessage?: string }>(
        "llm:tool-call-done",
        (e) => {
          const p = e.payload;
          if (p.sessionId !== activeSessionRef.current) {
            return;
          }
          const skillMapping = skillSessionMapRef.current.get(p.sessionId);
          if (skillMapping) {
            // Skill 内部的工具完成 → 更新对应的 skillToolCalls
            setMessages((prev) => {
              const next = [...prev];
              for (let i = next.length - 1; i >= 0; i--) {
                const m = next[i];
                if (m.role !== "assistant") continue;
                const tcs = m.meta?.toolCalls;
                if (!tcs) continue;
                const tcIdx = tcs.findIndex((tc) => tc.toolCallId === skillMapping.parentToolCallId);
                if (tcIdx === -1) continue;
                const updatedCalls = [...tcs];
                const skillTcs = (updatedCalls[tcIdx].skillToolCalls || []).map((stc) =>
                  stc.toolCallId === p.toolCallId
                    ? { ...stc, status: (p.success ? "done" : "error") as ToolCallDisplayInfo["status"], resultSummary: p.resultSummary, durationMs: p.durationMs, errorMessage: p.errorMessage }
                    : stc,
                );
                updatedCalls[tcIdx] = { ...updatedCalls[tcIdx], skillToolCalls: skillTcs };
                next[i] = { ...m, meta: { ...m.meta, toolCalls: updatedCalls } };
                break;
              }
              return next;
            });
            return;
          }
          // 主会话的工具完成 → 从后向前搜索 toolCallId
          setMessages((prev) => {
            let foundIndex = -1;
            for (let i = prev.length - 1; i >= 0; i -= 1) {
              const m = prev[i];
              if (m.role !== "assistant") continue;
              if (m.meta?.toolCalls?.some((tc) => tc.toolCallId === p.toolCallId)) {
                foundIndex = i;
                break;
              }
            }
            if (foundIndex === -1) {
              return prev;
            }
            const target = prev[foundIndex];
            const updatedCalls = (target.meta?.toolCalls ?? []).map((tc) =>
              tc.toolCallId === p.toolCallId
                ? { ...tc, status: (p.success ? "done" : "error") as ToolCallDisplayInfo["status"], resultSummary: p.resultSummary, durationMs: p.durationMs, errorMessage: p.errorMessage }
                : tc,
            );
            const next = [...prev];
            next[foundIndex] = {
              ...target,
              meta: { ...target.meta, toolCalls: updatedCalls },
            };
            return next;
          });
        },
      ),
      // P3 审批：后端在执行非 Auto 工具前请求用户决策
      listen<ApprovalRequest>("llm:tool-approval-request", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
        setActiveApproval((cur) => {
          if (cur === null) {
            return p;
          }
          approvalQueueRef.current.push(p);
          return cur;
        });
      }),
      // Iter 5 #4: 主对话自动调用 `skill.<id>` 时，后端发出 skill-spawn,
      // 携带新 sessionId — 这里保存父 session、切换 active、更新父气泡中对应工具调用项的 skill 字段。
      // 不再创建新气泡，Skill 内容内嵌到父气泡的工具调用项中。
      listen<{
        sessionId: string;
        conversationId: string;
        skillId: string;
        skillName: string;
        parentToolCallId: string;
      }>("llm:skill-spawn", (e) => {
        const p = e.payload;
        if (!activeSessionRef.current || activeSessionRef.current === p.sessionId) {
          return;
        }
        if (conversationId && p.conversationId !== conversationId) {
          return;
        }
        parentSessionRef.current = activeSessionRef.current;
        activeSessionRef.current = p.sessionId;
        // 记录 skill session → parent tool call 的映射
        skillSessionMapRef.current.set(p.sessionId, {
          parentToolCallId: p.parentToolCallId,
        });
        // 更新父气泡中对应工具调用项的 skill 字段（不创建新气泡）
        setMessages((prev) => {
          const next = [...prev];
          for (let i = next.length - 1; i >= 0; i--) {
            const m = next[i];
            if (m.role !== "assistant") continue;
            const tcs = m.meta?.toolCalls;
            if (!tcs) continue;
            const tcIdx = tcs.findIndex((tc) => tc.toolCallId === p.parentToolCallId);
            if (tcIdx === -1) continue;
            const updatedCalls = [...tcs];
            updatedCalls[tcIdx] = {
              ...updatedCalls[tcIdx],
              skillId: p.skillId,
              skillName: p.skillName,
              skillContent: "",
              skillToolCalls: [],
              skillStreaming: true,
            };
            next[i] = {
              ...m,
              meta: { ...m.meta, toolCalls: updatedCalls },
            };
            break;
          }
          return next;
        });
      }),
      listen<{ sessionId: string }>("llm:planning-start", (e) => {
        if (e.payload.sessionId !== activeSessionRef.current) return;
        setIsPlanning(true);
      }),
      listen<{ sessionId: string; planText: string }>("llm:planning-done", (e) => {
        if (e.payload.sessionId !== activeSessionRef.current) return;
        setIsPlanning(false);
      }),
      // Budget warning: tool call budget reaching 80%
      listen<{ sessionId: string; used: number; limit: number; type: string }>(
        "llm:budget-warning",
        (e) => {
          if (e.payload.sessionId !== activeSessionRef.current) return;
          setMessages((prev) => {
            const next = [...prev];
            const last = next[next.length - 1];
            if (last?.role === "assistant") {
              next[next.length - 1] = {
                ...last,
                meta: {
                  ...last.meta,
                  budgetWarning: { used: e.payload.used, limit: e.payload.limit },
                },
              };
            }
            return next;
          });
        },
      ),
      // P2 Tool Calling Loop：Agent 轮次结束 → 最终化助手消息、清理 streaming 状态
      listen<{ sessionId: string }>("llm:agent-done", (e) => {
        const sid = e.payload.sessionId;
        const skillMapping = skillSessionMapRef.current.get(sid);

        // Skill 子轮次完成：优先处理，不依赖 activeSessionRef
        if (skillMapping) {
          skillSessionMapRef.current.delete(sid);
          if (activeSessionRef.current === sid) {
            activeSessionRef.current = parentSessionRef.current;
          }
          parentSessionRef.current = null;

          setMessages((prev) => {
            const next = [...prev];
            for (let i = next.length - 1; i >= 0; i--) {
              const m = next[i];
              if (m.role !== "assistant") continue;
              const tcs = m.meta?.toolCalls;
              if (!tcs) continue;
              const tcIdx = tcs.findIndex((tc) => tc.toolCallId === skillMapping.parentToolCallId);
              if (tcIdx === -1) continue;

              const updatedCalls = [...tcs];
              updatedCalls[tcIdx] = { ...updatedCalls[tcIdx], skillStreaming: false };
              next[i] = { ...m, meta: { ...m.meta, toolCalls: updatedCalls } };
              break;
            }
            return next;
          });
          return;
        }

        // 其余情况：保持原有顶层 agent-done 逻辑
        if (sid !== activeSessionRef.current) {
          return;
        }

        // 顶层 agent 循环完成
        markNeedPersist();
        activeSessionRef.current = null;
        isAgentModeRef.current = false;
        setIsStreaming(false);
        setIsPlanning(false);
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
          const strippedPack = stripMarkedPassiveHighlightWithCount(next);
          if (strippedPack.stripped > 0) {
            passiveHighlightMarkedCountRef.current = Math.max(
              0,
              passiveHighlightMarkedCountRef.current - strippedPack.stripped,
            );
          }
          return strippedPack.messages;
        });
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
  }, [
    markNeedPersist,
    setMessages,
    setIsStreaming,
    freqCtrl,
    t,
    attachCitationToLastAssistant,
    conversationId,
  ]);

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

    // Iter 5 #4: while a skill sub-turn is in flight, abort BOTH the skill
    // and its parent — cancelling only the skill leaves the parent agent loop
    // blocked awaiting a tool result that will never arrive.
    const parentSid = parentSessionRef.current;

    const hasComposerDraft = input.trim().length > 0;

    markNeedPersist();
    activeSessionRef.current = null;
    parentSessionRef.current = null;
    skillSessionMapRef.current.clear();
    isAgentModeRef.current = false;
    setIsStreaming(false);
    setErrorBanner(null);
    if (hasComposerDraft) {
      setMessages((prev) => {
        const finalized = finalizeStreamingAssistant(prev);
        const strippedPack = stripMarkedPassiveHighlightWithCount(finalized);
        if (strippedPack.stripped > 0) {
          passiveHighlightMarkedCountRef.current = Math.max(
            0,
            passiveHighlightMarkedCountRef.current - strippedPack.stripped,
          );
        }
        return strippedPack.messages;
      });
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
      if (parentSid && parentSid !== sid) {
        await invoke("abort_llm_stream", { sessionId: parentSid });
      }
    } catch {
      /* 中止失败不阻塞 UI */
    }
  }, [input, messages, markNeedPersist, setMessages, setIsStreaming, isVaultSearching]);

  /** 流式建立成功后 2s 防抖：仅对最后一条用户消息做被动检测 */
  const schedulePassiveHighlightDetection = useCallback((userMsgId: string, userText: string) => {
    const epoch = passiveHighlightEpochRef.current;
    if (passiveHighlightTimerRef.current) {
      clearTimeout(passiveHighlightTimerRef.current);
      passiveHighlightTimerRef.current = null;
    }
    if (!isTauri()) return;
    passiveHighlightTimerRef.current = window.setTimeout(() => {
      passiveHighlightTimerRef.current = null;
      if (epoch !== passiveHighlightEpochRef.current) {
        return;
      }
      void (async () => {
        const convId = conversationIdForPassiveRef.current;
        if (!convId || disposedRef.current) return;

        setPassiveHighlightBanner(null);

        let cfg: VaultCfgForSend;
        try {
          cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
        } catch {
          return;
        }
        if (
          disposedRef.current ||
          conversationIdForPassiveRef.current !== convId ||
          epoch !== passiveHighlightEpochRef.current
        ) {
          return;
        }

        const enabled = cfg.cognitive?.passiveHighlightEnabled !== false;
        if (!enabled) return;

        const prov = cfg.ai?.activeProvider ?? "ollama";
        if (prov !== "ollama") {
          setPassiveHighlightBanner("noOllama");
          return;
        }

        const dm = depthModeForInviteRef.current;
        const ar = autoResolvedForInviteRef.current;
        if (dm === "shallow" || (dm === "auto" && ar === "shallow")) {
          return;
        }

        const trimmed = userText.trim();
        if ([...trimmed].length < 20) {
          return;
        }

        if (passiveHighlightMarkedCountRef.current >= 3) {
          setPassiveHighlightBanner("cap");
          return;
        }

        let resp: DetectPassiveHighlightResponse;
        try {
          resp = await invoke<DetectPassiveHighlightResponse>("detect_passive_highlight", {
            args: { text: trimmed },
          });
        } catch {
          return;
        }
        if (
          disposedRef.current ||
          conversationIdForPassiveRef.current !== convId ||
          epoch !== passiveHighlightEpochRef.current
        ) {
          return;
        }
        if (!resp.detected || !resp.kind) return;

        const { kind } = resp;

        passiveHighlightMarkedCountRef.current += 1;
        void trackKnowforgeEvent("passive_highlight.detected", {
          kind,
          confidence: resp.confidence ?? null,
        });

        setMessages((prev) =>
          prev.map((m) => {
            if (m.id !== userMsgId) return m;
            const marked: PassiveHighlightMarked = {
              phase: "marked",
              kind,
              confidence: resp.confidence ?? 0,
              summary: resp.summary ?? "",
              useRawFallback: resp.useRawFallback === true,
            };
            return {
              ...m,
              meta: { ...m.meta, passiveHighlight: marked },
            };
          }),
        );
      })();
    }, 2000);
  }, [setMessages]);

  /** Iter 5 #3：渲染一条本地 assistant 气泡列出可用 skill。不发后端、不进 active session。 */
  const handleSkillsList = useCallback(() => {
    setInput("");
    const lines = skillsCache.length === 0
      ? `_${t("aiPanel.skillsListEmpty")}_`
      : skillsCache
          .map((s) => `- \`${s.id}\` (${s.name}) — ${s.description}`)
          .join("\n");
    const body = `**${t("aiPanel.skillsListTitle")}**\n\n${lines}`;
    setMessages((prev) => [
      ...prev,
      {
        id: crypto.randomUUID(),
        role: "assistant",
        content: body,
      },
    ]);
  }, [skillsCache, setInput, setMessages, t]);

  /** Iter 5 #3：触发后端 invoke_skill 子轮次。复用 activeSessionRef + isAgentModeRef，
   *  现有 llm:* listeners 会自动按 sessionId 接管 skill 子流，UI 渲染按 meta.skillId 加 badge。 */
  const handleSkillSend = useCallback(
    async (skillId: string, body: string) => {
      setCopyToast(null);
      if (isStreaming || isVaultSearching) {
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

      const manifest = skillsCache.find((s) => s.id === skillId);
      if (!manifest) {
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: t("aiPanel.skillUnknown", { id: skillId }),
          },
        ]);
        setInput("");
        return;
      }

      setErrorBanner(null);
      setInput("");
      const userMsg: ChatMessage = {
        id: crypto.randomUUID(),
        role: "user",
        content: body,
        meta: { skillId, skillName: manifest.name },
      };
      setMessages((prev) => [...prev, userMsg]);

      let modelName: string | undefined;
      try {
        const cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
        const o = cfg.ai?.ollama;
        modelName = (o?.lastUsedModel?.trim() || o?.defaultModel?.trim()) || undefined;
      } catch {
        /* 拉取失败时让后端用默认模型 */
      }

      try {
        const res = await invokeSkill(skillId, body, conversationId, modelName);
        activeSessionRef.current = res.sessionId;
        // skill 一定走 agent loop（有自己的工具白名单），与 toolsEnabled 无关
        isAgentModeRef.current = true;
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
              skillId,
              skillName: manifest.name,
            },
          },
        ]);
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        setMessages((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: t("aiPanel.skillInvokeFailed", { message }),
          },
        ]);
      }
    },
    [
      conversationId,
      isStreaming,
      isVaultSearching,
      sessionReady,
      setIsStreaming,
      setMessages,
      skillsCache,
      t,
      workspaceReady,
    ],
  );

  const handleSend = useCallback(async () => {
    setCopyToast(null);
    const trimmed = input.trim();
    if (!trimmed || isStreaming || isVaultSearching) {
      return;
    }

    // Iter 5 #3：斜杠命令分叉。优先于普通聊天路径处理。
    // 仅当 id 命中已注册 skill 时才走 skill 流；其余 `/...` 输入按普通消息发送（兜底，不报错）。
    const slash = parseSlashCommand(
      trimmed,
      skillsCache.map((s) => s.id),
    );
    if (slash?.kind === "skills-list") {
      handleSkillsList();
      return;
    }
    if (slash?.kind === "skill") {
      void handleSkillSend(slash.skillId, slash.body);
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

    const ctx = getCurrentNoteContext();
    if (attachCurrentNote && ctx.kind === "unavailable" && ctx.reason === "no_workspace") {
      setErrorBanner(t("aiPanel.openFolder"));
      return;
    }

    lastSentQueryRef.current = trimmed;
    setInviteData(null);
    setChallengeInlineData(null);

    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: trimmed,
    };
    const nextChat = [...messages, userMsg];
    // Iter 5 #3：兑现"独立子轮次,不污染主对话历史"——skill 子轮次的 user/assistant 消息
    // 在 UI 中可见但不进 LLM context。运行时存活，刷新后退化为普通历史（详见 ChatMessage.meta 注释）。
    const chatTurns = nextChat
      .filter((m) => !m.meta?.skillId)
      .map((m) => ({ role: m.role, content: m.content }));
    const noteContext =
      attachCurrentNote && ctx.kind === "attached"
        ? { relPath: ctx.relPath, markdownForGate: ctx.markdown }
        : null;

    let modelName = "";
    let allowPrivateLocal = false;
    try {
      const cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
      const o = cfg.ai?.ollama;
      modelName = (o?.lastUsedModel?.trim() || o?.defaultModel?.trim()) ?? "";
      allowPrivateLocal = cfg.ai?.privacy?.allowPrivateContentInLocalLlm === true;
      if (!modelName) {
        setErrorBanner(t("aiPanel.noModel"));
        return;
      }
    } catch {
      /* 配置拉取失败时仍尝试发流，由后端报错 */
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
    if (includeVaultContext && workspaceReady) {
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

    thoughtInviteExcludeRef.current =
      noteContext != null ? [noteContext.relPath.replace(/\\/g, "/")] : [];

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

    // 记录本轮发送涉及的文档路径，用于隐私变更检测
    if (noteContext && !markdownTreatAsKfPrivateForUi(noteContext.markdownForGate)) {
      sharedDocPathsRef.current.add(noteContext.relPath);
    }
    if (vaultSearchResult) {
      for (const s of vaultSearchResult.snippets) {
        if (s.kind !== "privateOmitted") {
          sharedDocPathsRef.current.add(s.relPath);
        }
      }
    }

    setErrorBanner(null);
    setInput("");
    setMessages(nextChat);

    try {
      const streamArgs: {
        messages: { role: string; content: string }[];
        noteContext?: { relPath: string; markdownForGate: string };
        vaultContext?: { snippets: SearchWorkspaceContextResponse["snippets"] };
        depthMode: typeof depthMode;
        thoughtFocusContext?: ThoughtFocusContext;
        toolsEnabled?: boolean;
        conversationId?: string;
      } = {
        messages: chatTurns,
        depthMode,
        toolsEnabled,
        conversationId: conversationId ?? undefined,
      };
      if (noteContext) {
        streamArgs.noteContext = noteContext;
      }
      if (
        includeVaultContext &&
        vaultSearchResult != null &&
        vaultSearchResult.snippets.length > 0
      ) {
        streamArgs.vaultContext = { snippets: vaultSearchResult.snippets };
      }
      if (
        thoughtFocusContext != null &&
        thoughtFocusContext.thoughtId.trim() !== "" &&
        thoughtFocusContext.thoughtBody.trim() !== ""
      ) {
        streamArgs.thoughtFocusContext = thoughtFocusContext;
      }
      const res = await invoke<StartStreamResponse>("start_ollama_chat_stream", {
        args: streamArgs,
      });
      if (vaultSearchEpochRef.current !== searchEpoch) {
        void invoke("abort_llm_stream", { sessionId: res.sessionId }).catch(() => {});
        return;
      }
      if (depthMode === "auto" && res.resolvedDepth) {
        setAutoResolved(res.resolvedDepth);
      } else if (depthMode !== "auto") {
        setAutoResolved(null);
      }
      activeSessionRef.current = res.sessionId;
      // P2 Tool Calling Loop：仅在 invoke 成功后设置 agent 模式快照，
      // 避免失败下 stream-done 处理被错误提前拦截。
      isAgentModeRef.current = toolsEnabled;
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
          },
        },
      ]);
      schedulePassiveHighlightDetection(userMsg.id, trimmed);
    } catch (e) {
      setPrivacyHint(null);
      setVaultSearchSummary(null);
      setErrorBanner(e instanceof Error ? e.message : String(e));
    }
  }, [
    attachCurrentNote,
    conversationId,
    getCurrentNoteContext,
    includeVaultContext,
    input,
    isStreaming,
    isVaultSearching,
    messages,
    sessionReady,
    workspaceReady,
    setMessages,
    setIsStreaming,
    setAutoResolved,
    depthMode,
    thoughtFocusContext,
    toolsEnabled,
    t,
    schedulePassiveHighlightDetection,
    handleSkillSend,
    handleSkillsList,
    skillsCache,
  ]);

  const onComposerKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (slashOpen) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSlashIndex((i) => (i + 1) % slashCandidates.length);
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSlashIndex((i) => (i - 1 + slashCandidates.length) % slashCandidates.length);
          return;
        }
        if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
          e.preventDefault();
          const c = slashCandidates[safeSlashIndex];
          if (c) acceptSlashCandidate(c);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setSlashDismissed(true);
          return;
        }
      }
      if (e.key !== "Enter" || e.shiftKey) {
        return;
      }
      if (isStreaming || isVaultSearching) {
        return;
      }
      e.preventDefault();
      void handleSend();
    },
    [
      handleSend,
      isStreaming,
      isVaultSearching,
      slashOpen,
      slashCandidates,
      safeSlashIndex,
      acceptSlashCandidate,
    ],
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

  /** 用户接受邀请 -> 追加一条 deepening 用户消息并触发 LLM */
  const handleInviteAccept = useCallback(
    async (question: string, thought: ThoughtRetrievalResult | null) => {
      if (!isTauri() || isStreaming || !workspaceReady || !sessionReady || !conversationId) return;
      setInviteData(null);
      void freqCtrl.recordAccepted();

      // 默认邀请文案是面向用户的问句，写入聊天记录会显得像系统自言自语；改为第一人称自然表述。
      const defaultInvite = t("invite.defaultQuestion").trim();
      const userLine =
        question.trim() === defaultInvite ? t("invite.acceptedUserMessageDefault") : question;

      const userMsg: ChatMessage = {
        id: crypto.randomUUID(),
        role: "user",
        content: userLine,
      };
      const nextChat = [...messages, userMsg];
      const chatTurns = nextChat.map((m) => ({ role: m.role, content: m.content }));

      let modelName = "";
      try {
        const cfg = await invoke<VaultCfgForSend>("get_vault_config_for_ui");
        if (disposedRef.current) return;
        const o = cfg.ai?.ollama;
        modelName = (o?.lastUsedModel?.trim() || o?.defaultModel?.trim()) ?? "";
        if (!modelName) return;
      } catch {
        if (disposedRef.current) return;
        /* ignore */
      }

      if (disposedRef.current) return;

      setErrorBanner(null);
      setMessages(nextChat);
      lastSentQueryRef.current = userLine;

      try {
        const inviteContext: { question: string; thoughtExcerpt?: string } = { question };
        if (thought?.excerpt) {
          inviteContext.thoughtExcerpt = thought.excerpt;
        }
        const streamArgs: {
          messages: { role: string; content: string }[];
          depthMode: typeof depthMode;
          inviteContext?: { question: string; thoughtExcerpt?: string };
          thoughtFocusContext?: ThoughtFocusContext;
          conversationId?: string;
        } = {
          messages: chatTurns,
          depthMode,
          inviteContext,
          conversationId: conversationId ?? undefined,
        };
        if (
          thoughtFocusContext != null &&
          thoughtFocusContext.thoughtId.trim() !== "" &&
          thoughtFocusContext.thoughtBody.trim() !== ""
        ) {
          streamArgs.thoughtFocusContext = thoughtFocusContext;
        }

        const res = await invoke<StartStreamResponse>("start_ollama_chat_stream", {
          args: streamArgs,
        });
        if (disposedRef.current) return;
        if (depthMode === "auto" && res.resolvedDepth) {
          setAutoResolved(res.resolvedDepth);
        } else if (depthMode !== "auto") {
          setAutoResolved(null);
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
              deepening: true,
              timing: { startMs: Date.now() },
              replyContextSources: res.replyContextSources,
            },
          },
        ]);
        schedulePassiveHighlightDetection(userMsg.id, userLine);
      } catch (e) {
        if (disposedRef.current) return;
        setErrorBanner(e instanceof Error ? e.message : String(e));
      }
    },
    [
      conversationId,
      depthMode,
      isStreaming,
      messages,
      sessionReady,
      workspaceReady,
      setMessages,
      setIsStreaming,
      setAutoResolved,
      freqCtrl,
      thoughtFocusContext,
      schedulePassiveHighlightDetection,
      t,
    ],
  );

  /** 用户拒绝邀请 -> 本轮对话不再弹出 + 持久化频控计数 */
  const handleInviteDismiss = useCallback(() => {
    setInviteData(null);
    setEnoughForThisChat(true);
    void freqCtrl.recordEnough();
  }, [setEnoughForThisChat, freqCtrl]);

  /** 用户选择 snooze 邀请 N 天 */
  const handleInviteSnooze = useCallback(
    (days: number) => {
      setInviteData(null);
      setEnoughForThisChat(true);
      void freqCtrl.snoozeInvites(days);
    },
    [setEnoughForThisChat, freqCtrl],
  );

  const openPassiveHighlightSave = useCallback((userMsgId: string, prefill: string) => {
    setSavePopoverVariant("passive");
    setSavePopoverUserMsgId(userMsgId);
    setSavePopoverContent(prefill);
    setSavePopoverMsgId(userMsgId);
    setMessages((prev) =>
      prev.map((m) => {
        if (m.id !== userMsgId || m.meta?.passiveHighlight?.phase !== "marked") return m;
        const ph = m.meta.passiveHighlight as PassiveHighlightMarked;
        return {
          ...m,
          meta: {
            ...m.meta,
            passiveHighlight: { ...ph, overlayOpen: true },
          },
        };
      }),
    );
  }, [setMessages]);

  const handlePassiveMarkInaccurate = useCallback(async () => {
    const uid = savePopoverUserMsgId;
    if (!uid) return;
    let kind: string | undefined;
    for (const m of messagesRef.current) {
      if (m.id === uid && m.meta?.passiveHighlight?.phase === "marked") {
        kind = (m.meta.passiveHighlight as PassiveHighlightMarked).kind;
        break;
      }
    }
    setSavePopoverMsgId(null);
    setSavePopoverVariant("default");
    setSavePopoverUserMsgId(null);
    setMessages((prev) =>
      prev.map((m) => {
        if (m.id !== uid) return m;
        const rest = { ...m.meta };
        delete rest.passiveHighlight;
        return { ...m, meta: Object.keys(rest).length ? rest : undefined };
      }),
    );
    passiveHighlightMarkedCountRef.current = Math.max(0, passiveHighlightMarkedCountRef.current - 1);
    if (kind) {
      try {
        await invoke("increment_passive_highlight_inaccuracy", { args: { kind } });
      } catch {
        /* 忽略 */
      }
      void trackKnowforgeEvent("passive_highlight.inaccurate", { kind });
    }
  }, [savePopoverUserMsgId, setMessages]);

  const handleThoughtPopoverCancel = useCallback(() => {
    if (savePopoverVariant === "passive" && savePopoverUserMsgId) {
      const uid = savePopoverUserMsgId;
      setSavePopoverMsgId(null);
      setSavePopoverVariant("default");
      setSavePopoverUserMsgId(null);
      setMessages((prev) =>
        prev.map((m) => {
          if (m.id !== uid || m.meta?.passiveHighlight?.phase !== "marked") return m;
          const ph = m.meta.passiveHighlight as PassiveHighlightMarked;
          return {
            ...m,
            meta: {
              ...m.meta,
              passiveHighlight: { ...ph, overlayOpen: false },
            },
          };
        }),
      );
      return;
    }
    setSavePopoverMsgId(null);
    setSavePopoverVariant("default");
    setSavePopoverUserMsgId(null);
  }, [savePopoverVariant, savePopoverUserMsgId, setMessages]);

  /** Phase 4E / 被动高亮：保存为想法成功回调 */
  const handleThoughtSaved = useCallback(() => {
    const wasPassive = savePopoverVariant === "passive";
    const uid = savePopoverUserMsgId;
    let kind: string | undefined;
    let conf: number | undefined;
    if (wasPassive && uid) {
      for (const m of messagesRef.current) {
        if (m.id === uid && m.meta?.passiveHighlight?.phase === "marked") {
          const mk = m.meta.passiveHighlight as PassiveHighlightMarked;
          kind = mk.kind;
          conf = mk.confidence;
          break;
        }
      }
      setMessages((prev) =>
        prev.map((m) => {
          if (m.id !== uid || m.meta?.passiveHighlight?.phase !== "marked") return m;
          const ph = m.meta.passiveHighlight as PassiveHighlightMarked;
          return {
            ...m,
            meta: {
              ...m.meta,
              passiveHighlight: { ...ph, saved: true, overlayOpen: false },
            },
          };
        }),
      );
      void trackKnowforgeEvent("passive_highlight.saved", { kind, confidence: conf });
    }
    setSavePopoverMsgId(null);
    setSavePopoverVariant("default");
    setSavePopoverUserMsgId(null);
    setThoughtSaveToast("saved");
    window.setTimeout(() => setThoughtSaveToast(null), 1600);
  }, [savePopoverVariant, savePopoverUserMsgId, setMessages]);

  const canSend =
    isTauri() &&
    workspaceReady &&
    sessionReady &&
    !!conversationId &&
    input.trim().length > 0 &&
    !isStreaming &&
    !isVaultSearching;

  const handleChallengeInlineDismiss = useCallback(() => {
    setChallengeInlineData(null);
  }, []);

  /** 获取当前笔记路径（用于 ThoughtSavePopover 默认值） */
  const defaultNoteRelPath = (() => {
    const ctx = getCurrentNoteContext();
    return ctx.kind === "attached" ? ctx.relPath : null;
  })();

  return (
    <section
      className="ai-chat"
      aria-label={t("aiPanel.section")}
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
        <div className="ai-chat__banner ai-chat__banner--hint ai-chat__vault-refs" aria-live="polite" {...dragExcludeProps}>
          {vaultSearchSummary}
        </div>
      ) : null}

      {isVaultSearching ? (
        <div className="ai-chat__banner ai-chat__banner--hint" aria-live="polite" {...dragExcludeProps}>
          {t("aiPanel.searchingVault")}
        </div>
      ) : null}

      {isPlanning ? (
        <div className="ai-chat__banner ai-chat__banner--hint" aria-live="polite" {...dragExcludeProps}>
          {t("aiPanel.planning")}
        </div>
      ) : null}

      {passiveHighlightBanner ? (
        <div
          className="ai-chat__banner ai-chat__banner--hint ai-chat__banner--dismissible"
          role="status"
          aria-live="polite"
          {...dragExcludeProps}
        >
          <span className="ai-chat__banner__text">
            {passiveHighlightBanner === "noOllama"
              ? t("aiPanel.passiveHighlightHintNoOllama")
              : passiveHighlightBanner === "short"
                ? t("aiPanel.passiveHighlightHintShort")
                : t("aiPanel.passiveHighlightHintCap")}
          </span>
          <button
            type="button"
            className="ai-chat__banner__dismiss"
            aria-label={t("aiPanel.passiveHighlightBannerClose")}
            title={t("aiPanel.passiveHighlightBannerClose")}
            onClick={() => setPassiveHighlightBanner(null)}
            {...dragExcludeProps}
          >
            <IconDismissBanner />
          </button>
        </div>
      ) : null}

      <div
        ref={messagesContainerRef}
        className="ai-chat__messages"
        role="log"
        aria-label={t("aiPanel.messages")}
        aria-live="polite"
        {...dragProps}
      >
        {messages.length === 0 ? (
          <p className="ai-chat__empty" {...dragProps}>
            {t("aiPanel.empty")}
          </p>
        ) : (
          messages.map((m) => (
            <div
              key={m.id}
              className={`ai-chat__row ai-chat__row--${m.role}`}
              {...dragExcludeProps}
            >
              <div className={`ai-chat__bubble ai-chat__bubble--${m.role}`}>
                {m.role === "assistant" ? (
                  <>
                    {m.meta?.toolCalls && m.meta.toolCalls.length > 0 ? (
                      <div className="ai-chat__tool-calls">
                        {m.meta.toolCalls.map((tc) => (
                          <details key={tc.toolCallId} className="ai-chat__tool-call-details">
                            <summary className={`ai-chat__tool-call ai-chat__tool-call--${tc.status}`}>
                              <span className="ai-chat__tool-call__icon" aria-hidden={true}>
                                {tc.status === "running" ? "⋯" : tc.status === "done" ? "✓" : "✗"}
                              </span>
                              <span className="ai-chat__tool-call__name">{tc.toolName}</span>
                              {tc.durationMs != null && (
                                <span className="ai-chat__tool-call__duration">
                                  {(tc.durationMs / 1000).toFixed(1)}s
                                </span>
                              )}
                            </summary>
                            <div className="ai-chat__tool-call__detail">
                              {tc.inputSummary && (
                                <div className="ai-chat__tool-call__input">
                                  <span className="ai-chat__tool-call__label">输入</span>
                                  <code>{tc.inputSummary}</code>
                                </div>
                              )}
                              {tc.skillId && (
                                <div className="ai-chat__skill-inline">
                                  <span className="ai-chat__skill-badge">🧠 {tc.skillName}</span>
                                  <AiAssistantMarkdown content={tc.skillContent || ""} />
                                  {tc.skillStreaming && <span className="ai-chat__typing">▌</span>}
                                  {tc.skillToolCalls && tc.skillToolCalls.length > 0 && (
                                    <div className="ai-chat__tool-calls">
                                      {tc.skillToolCalls.map((stc) => (
                                        <details key={stc.toolCallId} className="ai-chat__tool-call-details">
                                          <summary className={`ai-chat__tool-call ai-chat__tool-call--${stc.status}`}>
                                            <span className="ai-chat__tool-call__icon" aria-hidden={true}>
                                              {stc.status === "running" ? "⋯" : stc.status === "done" ? "✓" : "✗"}
                                            </span>
                                            <span className="ai-chat__tool-call__name">{stc.toolName}</span>
                                            {stc.durationMs != null && (
                                              <span className="ai-chat__tool-call__duration">
                                                {(stc.durationMs / 1000).toFixed(1)}s
                                              </span>
                                            )}
                                          </summary>
                                          <div className="ai-chat__tool-call__detail">
                                            {stc.inputSummary && (
                                              <div className="ai-chat__tool-call__input">
                                                <span className="ai-chat__tool-call__label">输入</span>
                                                <code>{stc.inputSummary}</code>
                                              </div>
                                            )}
                                            {stc.resultSummary && (
                                              <div className="ai-chat__tool-call__result">
                                                <span className="ai-chat__tool-call__label">结果</span>
                                                <code>{stc.resultSummary}</code>
                                              </div>
                                            )}
                                            {stc.errorMessage && (
                                              <div className="ai-chat__tool-call__error">
                                                <span className="ai-chat__tool-call__label">错误</span>
                                                <code>{stc.errorMessage}</code>
                                              </div>
                                            )}
                                          </div>
                                        </details>
                                      ))}
                                    </div>
                                  )}
                                </div>
                              )}
                              {tc.resultSummary && (
                                <div className="ai-chat__tool-call__result">
                                  <span className="ai-chat__tool-call__label">结果</span>
                                  <code>{tc.resultSummary}</code>
                                </div>
                              )}
                              {tc.errorMessage && (
                                <div className="ai-chat__tool-call__error">
                                  <span className="ai-chat__tool-call__label">错误</span>
                                  <code>{tc.errorMessage}</code>
                                </div>
                              )}
                            </div>
                          </details>
                        ))}
                      </div>
                    ) : null}
                    {m.meta?.budgetWarning && (
                      <div className="ai-chat__budget-warning">
                        Agent {m.meta.budgetWarning.used}/{m.meta.budgetWarning.limit} tool calls used
                      </div>
                    )}
                    <AiAssistantMarkdown content={m.content} />
                    {!m.streaming && m.meta?.thoughtCitation && !m.meta.thoughtCitation.privateOmitted ? (
                      <button
                        type="button"
                        className="ai-chat__thought-cite"
                        onClick={() => openMarkdownTab?.(m.meta!.thoughtCitation!.relPath)}
                        {...dragExcludeProps}
                      >
                        {t("aiPanel.thoughtCited", {
                          note:
                            m.meta.thoughtCitation.relPath.split("/").pop() ??
                            m.meta.thoughtCitation.relPath,
                        })}
                      </button>
                    ) : null}
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
                            setSavePopoverVariant("default");
                            setSavePopoverUserMsgId(null);
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
                      <StreamingTimer timing={m.meta.timing} streaming={!!m.streaming} />
                    ) : null}
                  </>
                ) : (
                  <div className="ai-chat__user-stack">
                    <p className="ai-chat__user-text">{m.content}</p>
                    {m.meta?.passiveHighlight?.phase === "marked" ? (
                      <PassiveHighlightSaveCue
                        t={t}
                        state={m.meta.passiveHighlight as PassiveHighlightMarked}
                        disabled={!!savePopoverMsgId}
                        onSaveClick={() => {
                          // 落库用用户原文：detect 的 summary 易为英文，与中文输入下「保存想法」预期不符
                          openPassiveHighlightSave(m.id, m.content);
                        }}
                      />
                    ) : null}
                  </div>
                )}
              </div>
            </div>
          ))
        )}

        {/* 选区浮动工具栏 */}
        {selToolbar && !savePopoverMsgId && (
          <div
            className="ai-chat__sel-toolbar"
            style={{ top: selToolbar.top, left: selToolbar.left }}
            onMouseDown={(e) => e.preventDefault()}
          >
            <button
              type="button"
              className="ai-chat__sel-toolbar-btn"
              onClick={() => {
                setSavePopoverVariant("default");
                setSavePopoverUserMsgId(null);
                setSavePopoverContent(selToolbar.text);
                setSavePopoverMsgId(selToolbar.msgId);
                setSelToolbar(null);
              }}
            >
              <IconSaveThought />
              <span>{t("thoughtSave.buttonTitle")}</span>
            </button>
          </div>
        )}

        {/* ThoughtSavePopover（从 portal 渲染，不在 bubble 内） */}
        {savePopoverMsgId ? (
          <ThoughtSavePopover
            content={savePopoverContent}
            defaultRelPath={defaultNoteRelPath}
            isSelection={true}
            variant={savePopoverVariant}
            onMarkInaccurate={
              savePopoverVariant === "passive" ? handlePassiveMarkInaccurate : undefined
            }
            onSaved={handleThoughtSaved}
            onCancel={handleThoughtPopoverCancel}
            depthSlot={
              <DepthSlider
                compact
                value={depthMode}
                onChange={setDepthMode}
                autoResolved={autoResolved}
                disabled={isStreaming}
              />
            }
          />
        ) : null}

        {!isStreaming &&
          !challengeInlineData &&
          inviteData &&
          freqCtrl.shouldShowInvite(depthMode, enoughForThisChat, turnIndex, autoResolved) && (
          <InviteAfterAnswer
            depthMode={depthMode}
            thought={inviteData.thought}
            question={inviteData.question}
            onAccept={handleInviteAccept}
            onDismiss={handleInviteDismiss}
            onSnooze={handleInviteSnooze}
            disabled={isStreaming}
          />
        )}

        {!isStreaming && challengeInlineData && !inviteData ? (
          <ChallengeReviewInline
            depthMode={depthMode}
            thought={challengeInlineData.thought}
            question={challengeInlineData.question}
            templateKind={challengeInlineData.templateKind}
            onDismiss={handleChallengeInlineDismiss}
          />
        ) : null}

        <div ref={listEndRef} />
      </div>

      <div className="ai-chat__composer" {...dragExcludeProps}>
        {activePath ? (
          <label className="ai-chat__attach ai-chat__attach--above-input">
            <input
              type="checkbox"
              checked={attachCurrentNote}
              onChange={(e) => setAttachCurrentNote(e.target.checked)}
              disabled={isStreaming || isVaultSearching}
              aria-describedby="ai-chat-attach-hint"
              {...dragExcludeProps}
            />
            <span id="ai-chat-attach-hint">
              {t("aiPanel.attachNoteWith", { name: activePath.split("/").pop() ?? activePath })}
            </span>
          </label>
        ) : null}
        <label className="ai-chat__attach ai-chat__attach--above-input">
          <input
            type="checkbox"
            checked={includeVaultContext}
            onChange={(e) => setIncludeVaultContext(e.target.checked)}
            disabled={isStreaming || isVaultSearching}
            aria-describedby="ai-chat-vault-hint"
            {...dragExcludeProps}
          />
          <span id="ai-chat-vault-hint">{t("aiPanel.vaultSearch")}</span>
        </label>
        <div className="ai-chat__composer-field">
          <div className="ai-chat__composer-stack">
            {slashOpen ? (
              <ul
                className="ai-chat__slash-popup"
                role="listbox"
                aria-label={t("aiPanel.slashPopupLabel")}
              >
                {slashCandidates.map((c, i) => {
                  const active = i === safeSlashIndex;
                  return (
                    <li
                      key={c.value}
                      role="option"
                      aria-selected={active}
                      className={
                        active
                          ? "ai-chat__slash-option ai-chat__slash-option--active"
                          : "ai-chat__slash-option"
                      }
                      onMouseDown={(e) => {
                        e.preventDefault();
                        acceptSlashCandidate(c);
                      }}
                      onMouseEnter={() => setSlashIndex(i)}
                    >
                      <code className="ai-chat__slash-option__name">{c.label}</code>
                      <span className="ai-chat__slash-option__desc">{c.description}</span>
                    </li>
                  );
                })}
              </ul>
            ) : null}
            <textarea
              ref={composerRef}
              className="ai-chat__input"
              value={input}
              onChange={(e) => {
                setInput(e.target.value);
                setSlashDismissed(false);
                setSlashIndex(0);
              }}
              onKeyDown={onComposerKeyDown}
              placeholder={
                !isTauri()
                  ? t("aiPanel.placeholderDesktop")
                  : !workspaceReady
                    ? t("aiPanel.placeholderOpenFolder")
                    : isStreaming || isVaultSearching
                      ? t("aiPanel.placeholderStreaming")
                      : t("aiPanel.placeholderCompose")
              }
              disabled={!isTauri() || !workspaceReady}
              rows={3}
              aria-label={t("aiPanel.messageLabel")}
              aria-autocomplete="list"
              aria-expanded={slashOpen}
            />
            <div className="ai-chat__composer-toolbar" {...dragExcludeProps}>
              {!savePopoverMsgId ? (
                <div className="ai-chat__composer-depth">
                  <DepthSlider
                    compact
                    value={depthMode}
                    onChange={setDepthMode}
                    autoResolved={autoResolved}
                    disabled={isStreaming}
                  />
                </div>
              ) : null}
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

      {privacyChangeWarning ? (
        <PrivacyChangeOverlay
          onNewChat={() => { setPrivacyChangeWarning(null); void createConversation(); }}
          onContinue={() => setPrivacyChangeWarning(null)}
        />
      ) : null}

      <AiToolApprovalDialog request={activeApproval} onResolve={handleApprovalResolve} />
    </section>
  );
}
