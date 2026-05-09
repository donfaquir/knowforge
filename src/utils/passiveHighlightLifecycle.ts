import type { ChatMessage } from "../hooks/useWorkspaceAiConversations";
import type { PassiveHighlightMarked } from "../types/passiveHighlight";

function stripPassiveFromMeta(meta: ChatMessage["meta"]): ChatMessage["meta"] | undefined {
  if (!meta) return undefined;
  const { passiveHighlight: _ph, ...rest } = meta;
  return Object.keys(rest).length > 0 ? rest : undefined;
}

/** 本轮助手流结束：移除仍为 marked、未保存、浮层未开的被动提示 */
export function stripMarkedPassiveHighlightAfterTurn(messages: ChatMessage[]): ChatMessage[] {
  return stripMarkedPassiveHighlightWithCount(messages).messages;
}

export function stripMarkedPassiveHighlightWithCount(messages: ChatMessage[]): {
  messages: ChatMessage[];
  stripped: number;
} {
  let stripped = 0;
  const next = messages.map((m) => {
    if (m.role !== "user") return m;
    const ph = m.meta?.passiveHighlight;
    if (!ph || ph.phase !== "marked") return m;
    const mk = ph as PassiveHighlightMarked;
    if (mk.saved === true || mk.overlayOpen === true) return m;
    stripped += 1;
    const nextMeta = stripPassiveFromMeta(m.meta);
    return { ...m, meta: nextMeta };
  });
  return { messages: next, stripped };
}
