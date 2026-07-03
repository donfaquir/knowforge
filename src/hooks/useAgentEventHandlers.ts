import { type MutableRefObject, useRef, useEffect } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ToolCallDisplayInfo } from "./useWorkspaceAiConversations";
import type { ReplyContextSources } from "../types/replyContextSources";
import type { ApprovalRequest } from "../types/toolTypes";
import type { AutoResolvedDepth, DepthMode, ThoughtRetrievalResult, CountVaultThoughtsForReviewResponse, GenerateChallengeQuestionResponse } from "../types/cognitiveTypes";
import type { VaultConfigForUi } from "../types/vaultAiConfig";
import { isChallengeInlineLlmReady } from "../utils/isChallengeReviewLlmReady";
import { stripMarkedPassiveHighlightWithCount } from "../utils/passiveHighlightLifecycle";
import { trackKnowforgeEvent } from "../utils/knowforgeAnalytics";
import { getAppLocale } from "../i18n";
import type { TFunction } from "i18next";
import type { CognitiveFrequencyControl } from "./useCognitiveFrequencyControl";

// ---------------------------------------------------------------------------
// Helper functions (exported for use by handleStop in AiConversationPanel)
// ---------------------------------------------------------------------------

export function retractInterruptedTurn(prev: ChatMessage[]): ChatMessage[] {
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

export function finalizeStreamingAssistant(prev: ChatMessage[]): ChatMessage[] {
  const next = [...prev];
  const last = next[next.length - 1];
  if (last?.role === "assistant" && last.streaming) {
    next[next.length - 1] = { ...last, streaming: false };
  }
  return next;
}

export function findAndPatchToolCall(
  prev: ChatMessage[],
  targetToolCallId: string,
  patcher: (tc: ToolCallDisplayInfo) => ToolCallDisplayInfo,
): ChatMessage[] | null {
  for (let i = prev.length - 1; i >= 0; i--) {
    const m = prev[i];
    if (m.role !== "assistant") continue;

    const tcs = m.meta?.toolCalls;
    if (tcs) {
      const tcIdx = tcs.findIndex((tc) => tc.toolCallId === targetToolCallId);
      if (tcIdx !== -1) {
        const next = [...prev];
        const updated = [...tcs];
        updated[tcIdx] = patcher(updated[tcIdx]);
        next[i] = { ...m, meta: { ...m.meta, toolCalls: updated } };
        return next;
      }
    }
  }
  return null;
}

export function mergeStreamDelta(current: string, delta: string): string {
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

// ---------------------------------------------------------------------------
// Types used inside event listeners but not exported by other files
// ---------------------------------------------------------------------------

type MemoryProposalBatch = {
  session_id: string;
  proposals: Array<{
    id: string;
    action: string;
    category: string;
    target: string | null;
    content: unknown;
    reason: string;
  }>;
  created_at: string;
};

type InviteData = { thought: ThoughtRetrievalResult | null; question: string };

type ChallengeInlineData = {
  thought: ThoughtRetrievalResult;
  question: string;
  templateKind: string;
};

type VaultCfgForSend = VaultConfigForUi;

// ---------------------------------------------------------------------------
// Hook deps & return types
// ---------------------------------------------------------------------------

export type AgentEventDeps = {
  setMessages: React.Dispatch<React.SetStateAction<ChatMessage[]>>;
  setIsStreaming: (v: boolean) => void;
  setIsVaultSearching: (v: boolean) => void;
  setErrorBanner: (v: string | null) => void;
  setInput: (v: string) => void;
  setVaultSearchSummary: (v: string | null) => void;

  setInviteData: (v: InviteData | null) => void;
  setChallengeInlineData: (v: ChallengeInlineData | null) => void;
  setMemoryProposals: (v: MemoryProposalBatch | null) => void;
  setProposalDecisions: (v: Record<string, boolean>) => void;
  setActiveApproval: React.Dispatch<React.SetStateAction<ApprovalRequest | null>>;

  composerInputRef: MutableRefObject<string>;
  messagesRef: MutableRefObject<ChatMessage[]>;
  sharedDocPathsRef: MutableRefObject<Set<string>>;
  approvalQueueRef: MutableRefObject<ApprovalRequest[]>;
  inviteSearchEpochRef: MutableRefObject<number>;
  lastSentQueryRef: MutableRefObject<string>;
  enoughForThisChatRef: MutableRefObject<boolean>;
  depthModeForInviteRef: MutableRefObject<DepthMode>;
  autoResolvedForInviteRef: MutableRefObject<AutoResolvedDepth | null>;
  thoughtInviteExcludeRef: MutableRefObject<string[]>;
  passiveHighlightMarkedCountRef: MutableRefObject<number>;

  markNeedPersist: () => void;
  attachCitationToLastAssistant: (citation: ThoughtRetrievalResult | null) => void;
  freqCtrl: CognitiveFrequencyControl;
  conversationId: string | null;
  t: TFunction;
};

export type AgentSessionState = {
  activeSessionRef: MutableRefObject<string | null>;
  isAgentModeRef: MutableRefObject<boolean>;
  parentSessionRef: MutableRefObject<string | null>;
  skillSessionMapRef: MutableRefObject<Map<string, { parentToolCallId: string }>>;
};

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useAgentEventHandlers(deps: AgentEventDeps): AgentSessionState {
  const activeSessionRef = useRef<string | null>(null);
  const isAgentModeRef = useRef(false);
  const parentSessionRef = useRef<string | null>(null);
  const skillSessionMapRef = useRef<Map<string, { parentToolCallId: string }>>(new Map());

  const {
    setMessages,
    setIsStreaming,
    setIsVaultSearching,
    setErrorBanner,
    setInput,
    setVaultSearchSummary,
    setInviteData,
    setChallengeInlineData,
    setMemoryProposals,
    setProposalDecisions,
    setActiveApproval,
    composerInputRef,
    messagesRef,
    sharedDocPathsRef,
    approvalQueueRef,
    inviteSearchEpochRef,
    lastSentQueryRef,
    enoughForThisChatRef,
    depthModeForInviteRef,
    autoResolvedForInviteRef,
    thoughtInviteExcludeRef,
    passiveHighlightMarkedCountRef,
    markNeedPersist,
    attachCitationToLastAssistant,
    freqCtrl,
    conversationId,
    t,
  } = deps;

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
          setMessages((prev) =>
            findAndPatchToolCall(prev, skillMapping.parentToolCallId, (tc) => ({
              ...tc,
              skillContent: (tc.skillContent || "") + p.delta,
            })) ?? prev,
          );
          return;
        }
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
        setIsVaultSearching(false);
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
            // timeout or other failure: silently skip
          }
        })();
      }),
      listen<{ sessionId: string; code?: string; message: string }>("llm:stream-error", (e) => {
        const p = e.payload;
        if (p.sessionId !== activeSessionRef.current) {
          return;
        }
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
        setIsVaultSearching(false);
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
      listen<{ sessionId: string; toolCallId: string; toolName: string; inputSummary?: string; displaySummary?: string }>(
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
            displaySummary: p.displaySummary || undefined,
            status: "running",
            inputSummary: p.inputSummary,
          };
          if (skillMapping) {
            setMessages((prev) =>
              findAndPatchToolCall(prev, skillMapping.parentToolCallId, (tc) => ({
                ...tc,
                skillToolCalls: [...(tc.skillToolCalls || []), newCall],
              })) ?? prev,
            );
            return;
          }
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
      listen<{ sessionId: string; toolCallId: string; success: boolean; resultSummary?: string; durationMs?: number; errorMessage?: string }>(
        "llm:tool-call-done",
        (e) => {
          const p = e.payload;
          if (p.sessionId !== activeSessionRef.current) {
            return;
          }
          const skillMapping = skillSessionMapRef.current.get(p.sessionId);
          if (skillMapping) {
            setMessages((prev) =>
              findAndPatchToolCall(prev, skillMapping.parentToolCallId, (tc) => ({
                ...tc,
                skillToolCalls: (tc.skillToolCalls || []).map((stc) =>
                  stc.toolCallId === p.toolCallId
                    ? { ...stc, status: (p.success ? "done" : "error") as ToolCallDisplayInfo["status"], resultSummary: p.resultSummary, durationMs: p.durationMs, errorMessage: p.errorMessage }
                    : stc,
                ),
              })) ?? prev,
            );
            return;
          }
          setMessages((prev) => {
            const patchTc = (tc: ToolCallDisplayInfo): ToolCallDisplayInfo =>
              tc.toolCallId === p.toolCallId
                ? { ...tc, status: (p.success ? "done" : "error") as ToolCallDisplayInfo["status"], resultSummary: p.resultSummary, durationMs: p.durationMs, errorMessage: p.errorMessage }
                : tc;

            for (let i = prev.length - 1; i >= 0; i -= 1) {
              const m = prev[i];
              if (m.role !== "assistant") continue;

              if (m.meta?.toolCalls?.some((tc) => tc.toolCallId === p.toolCallId)) {
                const next = [...prev];
                next[i] = { ...m, meta: { ...m.meta, toolCalls: m.meta.toolCalls!.map(patchTc) } };
                return next;
              }
            }
            return prev;
          });
        },
      ),
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
      listen<MemoryProposalBatch>("llm:memory-proposals", (e) => {
        if (e.payload.session_id === activeSessionRef.current) {
          setMemoryProposals(e.payload);
          setProposalDecisions({});
        }
      }),
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
        skillSessionMapRef.current.set(p.sessionId, {
          parentToolCallId: p.parentToolCallId,
        });
        setMessages((prev) =>
          findAndPatchToolCall(prev, p.parentToolCallId, (tc) => ({
            ...tc,
            skillId: p.skillId,
            skillName: p.skillName,
            skillContent: "",
            skillToolCalls: [],
            skillStreaming: true,
          })) ?? prev,
        );
      }),
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
      listen<{
        sessionId: string;
        snippets: import("../types/vaultContextSearch").VaultSnippetRecord[];
        meta: import("../types/vaultContextSearch").SearchWorkspaceContextMeta | null;
        replyContextSources: ReplyContextSources;
      }>("llm:context-ready", (e) => {
        const { sessionId, snippets, meta, replyContextSources } = e.payload;
        if (sessionId !== activeSessionRef.current) return;

        if (snippets.length > 0 && meta) {
          const paths = snippets.map((s) => s.relPath).join(", ");
          const priv = snippets.filter((s) => s.kind === "privateOmitted").length;
          let line = t("aiPanel.vaultLine", {
            paths,
            scannedFiles: meta.scannedFiles,
            elapsedMs: meta.elapsedMs,
          });
          if (priv > 0) {
            line += ` ${t("aiPanel.vaultPrivateOmitted", { count: priv })}`;
          }
          if (meta.stoppedEarly) {
            line += ` ${t("aiPanel.vaultStoppedEarly")}`;
          }
          setVaultSearchSummary(line);
        } else {
          setVaultSearchSummary(null);
        }

        for (const s of snippets) {
          if (s.kind !== "privateOmitted") {
            sharedDocPathsRef.current.add(s.relPath);
          }
        }

        setMessages((prev) => {
          const idx = prev.findIndex((m) => m.role === "assistant" && m.streaming);
          if (idx < 0) return prev;
          const next = [...prev];
          next[idx] = { ...next[idx], meta: { ...next[idx].meta, replyContextSources } };
          return next;
        });

        setIsVaultSearching(false);
      }),
      listen<{ sessionId: string }>("llm:agent-done", (e) => {
        const sid = e.payload.sessionId;
        const skillMapping = skillSessionMapRef.current.get(sid);

        if (skillMapping) {
          skillSessionMapRef.current.delete(sid);
          if (activeSessionRef.current === sid) {
            activeSessionRef.current = parentSessionRef.current;
          }
          parentSessionRef.current = null;

          setMessages((prev) =>
            findAndPatchToolCall(prev, skillMapping.parentToolCallId, (tc) => ({
              ...tc,
              skillStreaming: false,
            })) ?? prev,
          );
          return;
        }

        if (sid !== activeSessionRef.current) {
          return;
        }

        markNeedPersist();
        activeSessionRef.current = null;
        isAgentModeRef.current = false;
        setIsStreaming(false);
        setIsVaultSearching(false);
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

  return { activeSessionRef, isAgentModeRef, parentSessionRef, skillSessionMapRef };
}
