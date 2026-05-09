import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

type Props = {
  content: string;
  className?: string;
};

/**
 * 助手气泡内 Markdown；与编辑器栈共用 remark-gfm / remark-breaks。
 */
export function AiAssistantMarkdown({ content, className }: Props) {
  if (!content.trim()) {
    return <span className="ai-chat__md-placeholder">…</span>;
  }
  return (
    <div className={className ?? "ai-chat__md"}>
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]}>{content}</ReactMarkdown>
    </div>
  );
}
