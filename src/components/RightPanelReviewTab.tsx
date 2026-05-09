/**
 * 右栏「回顾」标签页：独立挑战回顾（须包在 AiConversationSessionProvider 内）。
 */
import { useAiConversationSession } from "../contexts/AiConversationSessionContext";
import { ChallengeReviewPanel } from "./ChallengeReviewPanel";
import "./RightPanelReviewTab.css";

type Props = {
  /** 「结束回顾」时回到 AI 标签，避免落在大纲（可能不可用） */
  onClose: () => void;
};

export function RightPanelReviewTab({ onClose }: Props) {
  const { depthMode } = useAiConversationSession();
  return (
    <div className="right-panel-review-tab">
      <ChallengeReviewPanel onClose={onClose} depthMode={depthMode} />
    </div>
  );
}
