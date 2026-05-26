import type { ReplyContextSources } from "./replyContextSources";

/** 与 `docs/ai_tasks/07-conversation-storage.md` 及 Rust IPC 对齐（camelCase） */

export type ConversationMeta = {
  id: string;
  title: string;
  createdAt: number;
  updatedAt: number;
};

export type PersistedChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  /** 本轮模型实际注入的上下文来源（用于历史会话回显引用来源） */
  replyContextSources?: ReplyContextSources;
  /** Skill 子轮次标识：标记该消息属于哪个 Skill 的子对话；
   *  持久化目的：(a) 重新打开 vault 后保留 🧠 徽章；
   *  (b) 主对话发送时按此过滤掉 skill 子轮次，避免污染 prompt context。 */
  skillId?: string;
  skillName?: string;
};

/** 与 Rust `ThoughtFocusContextDisk` / LLM `thought_focus_context` 对齐 */
export type ThoughtFocusContext = {
  thoughtId: string;
  thoughtBody: string;
  maturity: string;
};

export type ConversationBodyOut = {
  schemaVersion: number;
  id: string;
  updatedAt: number;
  attachCurrentNote: boolean;
  /** 任务 08：是否在发送前检索 Vault 关键词上下文 */
  includeVaultContext: boolean;
  /** 迭代 6.1：与某条想法深聊时持久化 */
  thoughtFocusContext?: ThoughtFocusContext | null;
  messages: PersistedChatMessage[];
};

export type ListAiConversationsResponse = {
  conversations: ConversationMeta[];
  activeConversationId: string | null;
};
