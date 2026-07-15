import type { TFunction } from "i18next";
import type { ChatMessage, ContentBlock } from "../hooks/useWorkspaceAiConversations";
import type { ReplyContextSources } from "../types/replyContextSources";
import type { PassiveHighlightMarked } from "../types/passiveHighlight";
import { ToolCallItem } from "./ToolCallItem";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import { StreamingTimer } from "./StreamingTimer";
import { PassiveHighlightSaveCue } from "./PassiveHighlightSaveCue";
import { AiReplyContextSources } from "./AiReplyContextSources";

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

export function IconSaveThought() {
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

type MessageBubbleProps = {
  message: ChatMessage;
  onCopy: (content: string) => void;
  onSaveAsThought: (msgId: string, content: string) => void;
  onOpenThoughtCite: ((relPath: string) => void) | undefined;
  savePopoverMsgId: string | null;
  onPassiveHighlightSave: (msgId: string, content: string) => void;
  t: TFunction;
  dragExcludeProps: Record<string, unknown>;
  hasReplyContextSourcesToShow: (sources: ReplyContextSources) => boolean;
};

export function MessageBubble({
  message: m,
  onCopy,
  onSaveAsThought,
  onOpenThoughtCite,
  savePopoverMsgId,
  onPassiveHighlightSave,
  t,
  dragExcludeProps,
  hasReplyContextSourcesToShow,
}: MessageBubbleProps) {
  return (
    <div
      className={`ai-chat__row ai-chat__row--${m.role}`}
      {...dragExcludeProps}
    >
      <div className={`ai-chat__bubble ai-chat__bubble--${m.role}`}>
        {m.role === "assistant" ? (
          <>
            {m.contentBlocks && m.contentBlocks.length > 0 ? (
              /* Chronological rendering: thinking -> tool call -> thinking -> tool call -> ... */
              <>
                {m.contentBlocks.map((block: ContentBlock, idx: number) => {
                  if (block.type === "text") {
                    return block.text.trim() ? (
                      <AiAssistantMarkdown key={`text-${idx}`} content={block.text} />
                    ) : null;
                  }
                  // block.type === "tool_call"
                  const tc = m.meta?.toolCalls?.find((t) => t.toolCallId === block.toolCallId);
                  return tc ? (
                    <div key={`tc-${block.toolCallId}`} className="ai-chat__tool-calls">
                      <ToolCallItem tc={tc} />
                    </div>
                  ) : null;
                })}
              </>
            ) : (
              /* Fallback for historical messages without contentBlocks */
              <>
                {m.meta?.toolCalls && m.meta.toolCalls.length > 0 ? (
                  <div className="ai-chat__tool-calls">
                    {m.meta.toolCalls.map((tc) => (
                      <ToolCallItem key={tc.toolCallId} tc={tc} />
                    ))}
                  </div>
                ) : null}
                <AiAssistantMarkdown content={m.content} />
              </>
            )}
            {m.meta?.budgetWarning && (
              <div className="ai-chat__budget-warning">
                Agent {m.meta.budgetWarning.used}/{m.meta.budgetWarning.limit} tool calls used
              </div>
            )}
            {!m.streaming && m.meta?.thoughtCitation && !m.meta.thoughtCitation.privateOmitted ? (
              <button
                type="button"
                className="ai-chat__thought-cite"
                onClick={() => onOpenThoughtCite?.(m.meta!.thoughtCitation!.relPath)}
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
              <AiReplyContextSources sources={m.meta.replyContextSources} onOpenMarkdown={onOpenThoughtCite} />
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
                  onClick={() => void onCopy(m.content)}
                  aria-label={t("aiPanel.copyAria")}
                  title={t("aiPanel.copyMd")}
                  {...dragExcludeProps}
                >
                  <IconCopyClipboard />
                </button>
                <button
                  type="button"
                  className="ai-chat__copy"
                  onClick={() => onSaveAsThought(m.id, m.content)}
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
            {m.meta?.passiveHighlight?.phase === "marked" ? (
              <PassiveHighlightSaveCue
                t={t}
                state={m.meta.passiveHighlight as PassiveHighlightMarked}
                disabled={!!savePopoverMsgId}
                onSaveClick={() => {
                  onPassiveHighlightSave(m.id, m.content);
                }}
              />
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
