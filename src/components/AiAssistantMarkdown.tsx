import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

type Props = {
  content: string;
  className?: string;
};

/**
 * Custom anchor component that opens external links in the system browser
 * instead of navigating the webview (which would crash the app).
 */
const markdownComponents: Components = {
  a({ href, children }) {
    const handleClick = (e: React.MouseEvent<HTMLAnchorElement>) => {
      e.preventDefault();
      if (href) {
        openUrl(href);
      }
    };
    return (
      <a href={href} onClick={handleClick} title={href}>
        {children}
      </a>
    );
  },
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
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks]}
        components={markdownComponents}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}
