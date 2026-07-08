/** AI 多会话状态：供顶栏工具条与对话面板共享（须包在 AiNoteContextProvider 内）。 */
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import type { DepthMode, AutoResolvedDepth } from "../types/cognitiveTypes";
import { useWorkspaceAiConversations } from "../hooks/useWorkspaceAiConversations";

type SessionApi = ReturnType<typeof useWorkspaceAiConversations>;

export type AiConversationSessionValue = SessionApi & {
  isStreaming: boolean;
  setIsStreaming: React.Dispatch<React.SetStateAction<boolean>>;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  depthMode: DepthMode;
  setDepthMode: React.Dispatch<React.SetStateAction<DepthMode>>;
  autoResolved: AutoResolvedDepth | null;
  setAutoResolved: React.Dispatch<React.SetStateAction<AutoResolvedDepth | null>>;
  enoughForThisChat: boolean;
  setEnoughForThisChat: React.Dispatch<React.SetStateAction<boolean>>;
};

const AiConversationSessionContext = createContext<AiConversationSessionValue | null>(null);

export function AiConversationSessionProvider({
  children,
  workspaceReady,
  workspaceRoot,
  tauriRuntime,
  initialDepthMode,
}: {
  children: ReactNode;
  workspaceReady: boolean;
  /** 当前工作区根路径；切换文件夹时须变以触发 AI 会话重新加载（避免 workspaceReady 被批处理吞掉） */
  workspaceRoot: string | null;
  tauriRuntime: boolean;
  initialDepthMode?: DepthMode;
}) {
  const [isStreaming, setIsStreaming] = useState(false);
  const [depthMode, setDepthMode] = useState<DepthMode>(initialDepthMode ?? "deep");
  const [autoResolved, setAutoResolved] = useState<AutoResolvedDepth | null>(null);
  const [enoughForThisChat, setEnoughForThisChat] = useState(false);
  const session = useWorkspaceAiConversations({
    workspaceReady,
    workspaceRoot,
    tauriRuntime,
    isStreaming,
  });

  // config 异步加载完成后同步 depthMode（initialDepthMode 从 undefined → 实际值）
  useEffect(() => {
    if (initialDepthMode) setDepthMode(initialDepthMode);
  }, [initialDepthMode]);

  const value: AiConversationSessionValue = {
    ...session,
    isStreaming,
    setIsStreaming,
    workspaceReady,
    tauriRuntime,
    depthMode,
    setDepthMode,
    autoResolved,
    setAutoResolved,
    enoughForThisChat,
    setEnoughForThisChat,
  };

  return (
    <AiConversationSessionContext.Provider value={value}>{children}</AiConversationSessionContext.Provider>
  );
}

export function useAiConversationSession(): AiConversationSessionValue {
  const v = useContext(AiConversationSessionContext);
  if (!v) {
    throw new Error("useAiConversationSession must be used within AiConversationSessionProvider");
  }
  return v;
}
