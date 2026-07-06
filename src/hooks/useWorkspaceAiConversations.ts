import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAiNoteContext } from "../contexts/AiNoteContext";
import type {
  ConversationBodyOut,
  ConversationMeta,
  ListAiConversationsResponse,
  PersistedChatMessage,
  PersistedToolCall,
  ThoughtFocusContext,
} from "../types/aiConversation";
import type { ThoughtRetrievalResult } from "../types/cognitiveTypes";
import type { PassiveHighlightState } from "../types/passiveHighlight";
import type { ReplyContextSources } from "../types/replyContextSources";

export type ChatMessageTiming = {
  startMs: number;
  firstTokenMs?: number;
  endMs?: number;
};

/** P2 Tool Calling Loop：assistant 消息上展示的工具调用状态（运行时，不持久化） */
export type ToolCallDisplayInfo = {
  toolCallId: string;
  toolName: string;
  displaySummary?: string;
  status: "running" | "done" | "error";
  /** 可解释性字段 */
  inputSummary?: string;
  resultSummary?: string;
  durationMs?: number;
  errorMessage?: string;
  /** Skill 内嵌内容（仅 skill.* 工具有值） */
  skillId?: string;
  skillName?: string;
  skillContent?: string;
  skillToolCalls?: ToolCallDisplayInfo[];
  skillStreaming?: boolean;
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  streaming?: boolean;
  /** 运行时标记，不持久化到磁盘。 */
  meta?: {
    deepening?: boolean;
    timing?: ChatMessageTiming;
    passiveHighlight?: PassiveHighlightState;
    /** AI 轮次关联到的 vault 理解（用于引用标签，不持久化） */
    thoughtCitation?: ThoughtRetrievalResult;
    /** 本轮模型实际注入的上下文来源（持久化到历史会话） */
    replyContextSources?: ReplyContextSources;
    /** P2 Tool Calling Loop：本轮发生的工具调用（运行时） */
    toolCalls?: ToolCallDisplayInfo[];
    /** Budget warning when tool calls reach 80% of limit */
    budgetWarning?: { used: number; limit: number };
    providerLabel?: string;
    modelName?: string;
    /** Iter 5 #3：本轮属于 Skill 子轮次。已持久化到 PersistedChatMessage（followups #2）；
     *  作用：(a) UI 渲染 🧠 badge；(b) 下一次主对话发送前从 chatTurns 过滤掉，兑现"独立子轮次,不污染主对话历史"。
     *  即使关闭并重新打开 vault，徽章和过滤行为均保持一致。 */
    skillId?: string;
    skillName?: string;
  };
};

function normalizeToolCallForPersist(tc: ToolCallDisplayInfo): PersistedToolCall {
  const result: PersistedToolCall = {
    toolCallId: tc.toolCallId,
    toolName: tc.toolName,
    displaySummary: tc.displaySummary,
    status: tc.status === "running" ? "done" : (tc.status as "done" | "error"),
    inputSummary: tc.inputSummary,
    resultSummary: tc.resultSummary,
    durationMs: tc.durationMs,
    errorMessage: tc.errorMessage,
    skillId: tc.skillId,
    skillName: tc.skillName,
    skillContent: tc.skillContent,
  };
  if (tc.skillToolCalls && tc.skillToolCalls.length > 0) {
    result.skillToolCalls = tc.skillToolCalls.map(normalizeToolCallForPersist);
  }
  return result;
}

function bodyToMessages(body: ConversationBodyOut): ChatMessage[] {
  return body.messages.map((m) => {
    const meta: ChatMessage["meta"] = {};
    if (m.replyContextSources) meta.replyContextSources = m.replyContextSources;
    if (m.skillId) meta.skillId = m.skillId;
    if (m.skillName) meta.skillName = m.skillName;
    if (m.toolCalls) meta.toolCalls = m.toolCalls as unknown as ToolCallDisplayInfo[];
    return {
      id: m.id,
      role: m.role === "assistant" ? ("assistant" as const) : ("user" as const),
      content: m.content,
      meta: Object.keys(meta).length > 0 ? meta : undefined,
    };
  });
}

function toPersistPayload(messages: ChatMessage[]): PersistedChatMessage[] {
  return messages
    .filter((m) => !m.streaming)
    .map((m) => {
      const msg: PersistedChatMessage = {
        id: m.id,
        role: m.role,
        content: m.content,
        replyContextSources: m.meta?.replyContextSources,
        skillId: m.meta?.skillId,
        skillName: m.meta?.skillName,
      };
      if (m.meta?.toolCalls && m.meta.toolCalls.length > 0) {
        msg.toolCalls = m.meta.toolCalls.map(normalizeToolCallForPersist);
      }
      return msg;
    });
}

type CreateResponse = { id: string };

export function useWorkspaceAiConversations(opts: {
  workspaceReady: boolean;
  /** 与 vault 对齐的工作区根；变化时必须重载会话（切换文件夹时 workspaceReady 可能不经过 false） */
  workspaceRoot: string | null;
  tauriRuntime: boolean;
  isStreaming: boolean;
}) {
  const { workspaceReady, workspaceRoot, tauriRuntime, isStreaming } = opts;
  const { attachCurrentNote, setAttachCurrentNote } = useAiNoteContext();

  const [conversationId, setConversationId] = useState<string | null>(null);
  const [conversations, setConversations] = useState<ConversationMeta[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sessionReady, setSessionReady] = useState(false);
  const [includeVaultContext, setIncludeVaultContext] = useState(false);
  const [thoughtFocusContext, setThoughtFocusContext] = useState<ThoughtFocusContext | null>(null);
  const [isVaultSearching, setIsVaultSearching] = useState(false);

  const saveChainRef = useRef(Promise.resolve());
  const pendingPersistRef = useRef(false);
  const messagesRef = useRef(messages);
  const attachRef = useRef(false);
  const includeVaultRef = useRef(false);
  const thoughtFocusRef = useRef<ThoughtFocusContext | null>(null);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    attachRef.current = attachCurrentNote;
  }, [attachCurrentNote]);

  useEffect(() => {
    includeVaultRef.current = includeVaultContext;
  }, [includeVaultContext]);

  useEffect(() => {
    thoughtFocusRef.current = thoughtFocusContext;
  }, [thoughtFocusContext]);

  useEffect(() => {
    setIsVaultSearching(false);
  }, [conversationId]);

  const runSerializedSave = useCallback((fn: () => Promise<void>) => {
    saveChainRef.current = saveChainRef.current.then(fn).catch((e) => {
      console.error(e);
    });
    return saveChainRef.current;
  }, []);

  const refreshList = useCallback(async () => {
    if (!isTauri() || !workspaceReady) {
      return;
    }
    const list = await invoke<ListAiConversationsResponse>("list_ai_conversations");
    setConversations(list.conversations);
  }, [workspaceReady, workspaceRoot]);

  const persistConversation = useCallback(
    async (msgs: ChatMessage[], attach: boolean, includeVault: boolean, setAsActive = true) => {
      const cid = conversationId;
      if (!cid || !isTauri() || !workspaceReady) {
        return;
      }
      const cleaned = toPersistPayload(msgs);
      await runSerializedSave(async () => {
        await invoke("save_ai_conversation", {
          args: {
            conversationId: cid,
            attachCurrentNote: attach,
            includeVaultContext: includeVault,
            thoughtFocusContext: thoughtFocusRef.current,
            messages: cleaned,
            setAsActive,
          },
        });
        const list = await invoke<ListAiConversationsResponse>("list_ai_conversations");
        setConversations(list.conversations);
      });
    },
    [conversationId, workspaceReady, runSerializedSave],
  );

  /** 流式结束、中止等：由面板在更新 state 前调用 */
  const markNeedPersist = useCallback(() => {
    pendingPersistRef.current = true;
  }, []);

  /** 初始加载 / 切换工作区 */
  useEffect(() => {
    if (!workspaceReady || !tauriRuntime || !isTauri()) {
      setConversationId(null);
      setMessages([]);
      setConversations([]);
      setIncludeVaultContext(false);
      setThoughtFocusContext(null);
      setSessionReady(true);
      return;
    }

    let cancelled = false;
    setSessionReady(false);
    // 切换根路径时立即清空，避免批处理导致 workspaceReady 未变 false 时仍显示旧工作区消息
    setConversationId(null);
    setMessages([]);
    setConversations([]);

    void (async () => {
      try {
        let list = await invoke<ListAiConversationsResponse>("list_ai_conversations");
        if (cancelled) {
          return;
        }

        if (list.conversations.length === 0) {
          const created = await invoke<CreateResponse>("create_ai_conversation", { args: {} });
          if (cancelled) {
            return;
          }
          setConversationId(created.id);
          setMessages([]);
          setAttachCurrentNote(true);
          setIncludeVaultContext(false);
          setThoughtFocusContext(null);
          list = await invoke<ListAiConversationsResponse>("list_ai_conversations");
          if (cancelled) {
            return;
          }
          setConversations(list.conversations);
          setSessionReady(true);
          return;
        }

        let pick =
          list.activeConversationId &&
          list.conversations.some((c) => c.id === list.activeConversationId)
            ? list.activeConversationId
            : list.conversations.reduce((a, b) => (b.updatedAt > a.updatedAt ? b : a)).id;

        const body = await invoke<ConversationBodyOut>("load_ai_conversation", {
          args: { conversationId: pick },
        });
        if (cancelled) {
          return;
        }
        setConversationId(body.id);
        setMessages(bodyToMessages(body));
        setAttachCurrentNote(body.attachCurrentNote);
        setIncludeVaultContext(body.includeVaultContext);
        setThoughtFocusContext(body.thoughtFocusContext ?? null);
        setConversations(list.conversations);
        setSessionReady(true);
      } catch (e) {
        console.error(e);
        if (!cancelled) {
          setSessionReady(true);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [workspaceReady, workspaceRoot, tauriRuntime, setAttachCurrentNote]);

  /** 流式结束后立即落盘（§6） */
  useEffect(() => {
    if (!sessionReady || !conversationId || !workspaceReady || !tauriRuntime) {
      return;
    }
    if (isStreaming || !pendingPersistRef.current) {
      return;
    }
    pendingPersistRef.current = false;
    void persistConversation(messagesRef.current, attachRef.current, includeVaultRef.current, true);
  }, [isStreaming, sessionReady, conversationId, workspaceReady, tauriRuntime, persistConversation]);

  /** 非流式：debounce 保存（§6） */
  useEffect(() => {
    if (!sessionReady || !conversationId || !workspaceReady || !tauriRuntime) {
      return;
    }
    if (isStreaming) {
      return;
    }
    const t = window.setTimeout(() => {
      void persistConversation(
        messagesRef.current,
        attachRef.current,
        includeVaultRef.current,
        true,
      );
    }, 550);
    return () => window.clearTimeout(t);
  }, [
    messages,
    attachCurrentNote,
    includeVaultContext,
    thoughtFocusContext,
    sessionReady,
    conversationId,
    workspaceReady,
    tauriRuntime,
    isStreaming,
    persistConversation,
  ]);

  const switchConversation = useCallback(
    async (id: string) => {
      if (isStreaming || isVaultSearching || !isTauri() || !workspaceReady) {
        return;
      }
      await invoke("set_active_ai_conversation", { args: { conversationId: id } });
      const body = await invoke<ConversationBodyOut>("load_ai_conversation", {
        args: { conversationId: id },
      });
      setConversationId(body.id);
      setMessages(bodyToMessages(body));
      setAttachCurrentNote(body.attachCurrentNote);
      setIncludeVaultContext(body.includeVaultContext);
      setThoughtFocusContext(body.thoughtFocusContext ?? null);
      await refreshList();
    },
    [isStreaming, isVaultSearching, workspaceReady, setAttachCurrentNote, refreshList],
  );

  const createConversation = useCallback(
    async (focus?: ThoughtFocusContext | null) => {
      if (isStreaming || isVaultSearching || !isTauri() || !workspaceReady) {
        return;
      }
      const args =
        focus != null
          ? {
              thoughtFocusContext: {
                thoughtId: focus.thoughtId,
                thoughtBody: focus.thoughtBody,
                maturity: focus.maturity,
              },
            }
          : {};
      const created = await invoke<CreateResponse>("create_ai_conversation", { args });
      setConversationId(created.id);
      setMessages([]);
      setAttachCurrentNote(true);
      setIncludeVaultContext(false);
      setThoughtFocusContext(focus ?? null);
      await refreshList();
    },
    [isStreaming, isVaultSearching, workspaceReady, setAttachCurrentNote, refreshList],
  );

  const deleteConversation = useCallback(
    async (id: string) => {
      if (isStreaming || isVaultSearching || !isTauri() || !workspaceReady) {
        return;
      }
      await invoke("delete_ai_conversation", { args: { conversationId: id } });
      // 清理后端 ConfirmOncePerSession 缓存，避免 id 回收时残留批准。
      void invoke("clear_conversation_approvals", { args: { conversationId: id } }).catch(() => {});
      const list = await invoke<ListAiConversationsResponse>("list_ai_conversations");
      setConversations(list.conversations);

      if (conversationId !== id) {
        return;
      }

      if (list.conversations.length === 0) {
        const created = await invoke<CreateResponse>("create_ai_conversation", { args: {} });
        setConversationId(created.id);
        setMessages([]);
        setAttachCurrentNote(true);
        setIncludeVaultContext(false);
        setThoughtFocusContext(null);
        const again = await invoke<ListAiConversationsResponse>("list_ai_conversations");
        setConversations(again.conversations);
        return;
      }

      let pick =
        list.activeConversationId &&
        list.conversations.some((c) => c.id === list.activeConversationId)
          ? list.activeConversationId
          : list.conversations.reduce((a, b) => (b.updatedAt > a.updatedAt ? b : a)).id;

      const body = await invoke<ConversationBodyOut>("load_ai_conversation", {
        args: { conversationId: pick },
      });
      setConversationId(body.id);
      setMessages(bodyToMessages(body));
      setAttachCurrentNote(body.attachCurrentNote);
      setIncludeVaultContext(body.includeVaultContext);
      setThoughtFocusContext(body.thoughtFocusContext ?? null);
    },
    [isStreaming, isVaultSearching, workspaceReady, conversationId, setAttachCurrentNote],
  );

  return {
    conversationId,
    conversations,
    messages,
    setMessages,
    sessionReady,
    includeVaultContext,
    setIncludeVaultContext,
    thoughtFocusContext,
    setThoughtFocusContext,
    isVaultSearching,
    setIsVaultSearching,
    switchConversation,
    createConversation,
    deleteConversation,
    refreshList,
    markNeedPersist,
    persistConversation,
    runSerializedSave,
  };
}
