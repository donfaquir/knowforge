import type { ToolCallDisplayInfo } from "../hooks/useWorkspaceAiConversations";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";

export function ToolCallItem({ tc }: { tc: ToolCallDisplayInfo }) {
  return (
    <details className="ai-chat__tool-call-details">
      <summary className={`ai-chat__tool-call ai-chat__tool-call--${tc.status}`}>
        <span className="ai-chat__tool-call__icon" aria-hidden={true}>
          {tc.status === "running" ? "⋯" : tc.status === "done" ? "✓" : "✗"}
        </span>
        <span className="ai-chat__tool-call__name">{tc.displaySummary || tc.toolName}</span>
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
                  <ToolCallItem key={stc.toolCallId} tc={stc} />
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
  );
}
