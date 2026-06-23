import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  ConversationBodyOut,
  ConversationMeta,
  ListAiConversationsResponse,
  PersistedChatMessage,
  ThoughtFocusContext,
} from "../types/aiConversation";
import type { ReplyContextSources } from "../types/replyContextSources";

export type ThoughtMgmtChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  streaming?: boolean;
  meta?: {
    deepening?: boolean;
    timing?: { startMs: number; firstTokenMs?: number; endMs?: number };
    replyContextSources?: ReplyContextSources;
    providerLabel?: string;
    modelName?: string;
  };
};

function bodyToMessages(body: ConversationBodyOut): ThoughtMgmtChatMessage[] {
  return body.messages.map((m) => ({
    id: m.id,
    role: m.role === "assistant" ? ("assistant" as const) : ("user" as const),
    content: m.content,
    meta: m.replyContextSources ? { replyContextSources: m.replyContextSources } : undefined,
  }));
}

function toPersistPayload(messages: ThoughtMgmtChatMessage[]): PersistedChatMessage[] {
  return messages
    .filter((m) => !m.streaming)
    .map((m) => ({
      id: m.id,
      role: m.role,
      content: m.content,
      replyContextSources: m.meta?.replyContextSources,
    }));
}

type CreateResponse = { id: string };

/** 想法管理页专用：持久化目录为 `.knowforge/conversations_thought_mgmt/`，与文档 AI 会话隔离 */
export function useThoughtMgmtAiConversations(opts: {
  workspaceReady: boolean;
  workspaceRoot: string | null;
  tauriRuntime: boolean;
  isStreaming: boolean;
}) {
  const { workspaceReady, workspaceRoot, tauriRuntime, isStreaming } = opts;

  const [conversationId, setConversationId] = useState<string | null>(null);
  const [conversations, setConversations] = useState<ConversationMeta[]>([]);
  const [messages, setMessages] = useState<ThoughtMgmtChatMessage[]>([]);
  const [sessionReady, setSessionReady] = useState(false);
  const [thoughtFocusContext, setThoughtFocusContext] = useState<ThoughtFocusContext | null>(null);
  const [isVaultSearching, setIsVaultSearching] = useState(false);
  const vaultSearchEpochRef = useRef(0);

  const saveChainRef = useRef(Promise.resolve());
  const pendingPersistRef = useRef(false);
  const messagesRef = useRef(messages);
  const thoughtFocusRef = useRef<ThoughtFocusContext | null>(null);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    thoughtFocusRef.current = thoughtFocusContext;
  }, [thoughtFocusContext]);

  useEffect(() => {
    vaultSearchEpochRef.current += 1;
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
    const list = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
    setConversations(list.conversations);
  }, [workspaceReady, workspaceRoot]);

  const persistConversation = useCallback(
    async (msgs: ThoughtMgmtChatMessage[], setAsActive = true) => {
      const cid = conversationId;
      if (!cid || !isTauri() || !workspaceReady) {
        return;
      }
      const cleaned = toPersistPayload(msgs);
      await runSerializedSave(async () => {
        await invoke("save_thought_mgmt_ai_conversation", {
          args: {
            conversationId: cid,
            attachCurrentNote: false,
            includeVaultContext: true,
            thoughtFocusContext: thoughtFocusRef.current,
            messages: cleaned,
            setAsActive,
          },
        });
        const list = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
        setConversations(list.conversations);
      });
    },
    [conversationId, workspaceReady, runSerializedSave],
  );

  const markNeedPersist = useCallback(() => {
    pendingPersistRef.current = true;
  }, []);

  useEffect(() => {
    if (!workspaceReady || !tauriRuntime || !isTauri()) {
      setConversationId(null);
      setMessages([]);
      setConversations([]);
      setThoughtFocusContext(null);
      setSessionReady(true);
      return;
    }

    let cancelled = false;
    setSessionReady(false);
    setConversationId(null);
    setMessages([]);
    setConversations([]);

    void (async () => {
      try {
        let list = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
        if (cancelled) {
          return;
        }

        if (list.conversations.length === 0) {
          const created = await invoke<CreateResponse>("create_thought_mgmt_ai_conversation", { args: {} });
          if (cancelled) {
            return;
          }
          setConversationId(created.id);
          setMessages([]);
          setThoughtFocusContext(null);
          list = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
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

        const body = await invoke<ConversationBodyOut>("load_thought_mgmt_ai_conversation", {
          args: { conversationId: pick },
        });
        if (cancelled) {
          return;
        }
        setConversationId(body.id);
        setMessages(bodyToMessages(body));
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
  }, [workspaceReady, workspaceRoot, tauriRuntime]);

  useEffect(() => {
    if (!sessionReady || !conversationId || !workspaceReady || !tauriRuntime) {
      return;
    }
    if (isStreaming || !pendingPersistRef.current) {
      return;
    }
    pendingPersistRef.current = false;
    void persistConversation(messagesRef.current, true);
  }, [isStreaming, sessionReady, conversationId, workspaceReady, tauriRuntime, persistConversation]);

  useEffect(() => {
    if (!sessionReady || !conversationId || !workspaceReady || !tauriRuntime) {
      return;
    }
    if (isStreaming) {
      return;
    }
    const t = window.setTimeout(() => {
      void persistConversation(messagesRef.current, true);
    }, 550);
    return () => window.clearTimeout(t);
  }, [
    messages,
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
      await invoke("set_active_thought_mgmt_ai_conversation", { args: { conversationId: id } });
      const body = await invoke<ConversationBodyOut>("load_thought_mgmt_ai_conversation", {
        args: { conversationId: id },
      });
      setConversationId(body.id);
      setMessages(bodyToMessages(body));
      setThoughtFocusContext(body.thoughtFocusContext ?? null);
      await refreshList();
    },
    [isStreaming, isVaultSearching, workspaceReady, refreshList],
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
      const created = await invoke<CreateResponse>("create_thought_mgmt_ai_conversation", { args });
      setConversationId(created.id);
      setMessages([]);
      setThoughtFocusContext(focus ?? null);
      await refreshList();
    },
    [isStreaming, isVaultSearching, workspaceReady, refreshList],
  );

  const deleteConversation = useCallback(
    async (id: string) => {
      if (isStreaming || isVaultSearching || !isTauri() || !workspaceReady) {
        return;
      }
      await invoke("delete_thought_mgmt_ai_conversation", { args: { conversationId: id } });
      const list = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
      setConversations(list.conversations);

      if (conversationId !== id) {
        return;
      }

      if (list.conversations.length === 0) {
        const created = await invoke<CreateResponse>("create_thought_mgmt_ai_conversation", { args: {} });
        setConversationId(created.id);
        setMessages([]);
        setThoughtFocusContext(null);
        const again = await invoke<ListAiConversationsResponse>("list_thought_mgmt_ai_conversations");
        setConversations(again.conversations);
        return;
      }

      let pick =
        list.activeConversationId &&
        list.conversations.some((c) => c.id === list.activeConversationId)
          ? list.activeConversationId
          : list.conversations.reduce((a, b) => (b.updatedAt > a.updatedAt ? b : a)).id;

      const body = await invoke<ConversationBodyOut>("load_thought_mgmt_ai_conversation", {
        args: { conversationId: pick },
      });
      setConversationId(body.id);
      setMessages(bodyToMessages(body));
      setThoughtFocusContext(body.thoughtFocusContext ?? null);
    },
    [isStreaming, isVaultSearching, workspaceReady, conversationId],
  );

  return {
    conversationId,
    conversations,
    messages,
    setMessages,
    sessionReady,
    thoughtFocusContext,
    setThoughtFocusContext,
    isVaultSearching,
    setIsVaultSearching,
    vaultSearchEpochRef,
    switchConversation,
    createConversation,
    deleteConversation,
    refreshList,
    markNeedPersist,
    persistConversation,
    runSerializedSave,
  };
}
