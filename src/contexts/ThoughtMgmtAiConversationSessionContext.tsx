import { createContext, useContext, useState, type ReactNode } from "react";
import { useThoughtMgmtAiConversations } from "../hooks/useThoughtMgmtAiConversations";

type SessionApi = ReturnType<typeof useThoughtMgmtAiConversations>;

export type ThoughtMgmtAiConversationSessionValue = SessionApi & {
  isStreaming: boolean;
  setIsStreaming: React.Dispatch<React.SetStateAction<boolean>>;
  workspaceReady: boolean;
  tauriRuntime: boolean;
};

const ThoughtMgmtAiConversationSessionContext = createContext<ThoughtMgmtAiConversationSessionValue | null>(null);

export function ThoughtMgmtAiConversationSessionProvider({
  children,
  workspaceReady,
  workspaceRoot,
  tauriRuntime,
}: {
  children: ReactNode;
  workspaceReady: boolean;
  workspaceRoot: string | null;
  tauriRuntime: boolean;
}) {
  const [isStreaming, setIsStreaming] = useState(false);
  const session = useThoughtMgmtAiConversations({
    workspaceReady,
    workspaceRoot,
    tauriRuntime,
    isStreaming,
  });

  const value: ThoughtMgmtAiConversationSessionValue = {
    ...session,
    isStreaming,
    setIsStreaming,
    workspaceReady,
    tauriRuntime,
  };

  return (
    <ThoughtMgmtAiConversationSessionContext.Provider value={value}>
      {children}
    </ThoughtMgmtAiConversationSessionContext.Provider>
  );
}

export function useThoughtMgmtAiConversationSession(): ThoughtMgmtAiConversationSessionValue {
  const v = useContext(ThoughtMgmtAiConversationSessionContext);
  if (!v) {
    throw new Error("useThoughtMgmtAiConversationSession must be used within ThoughtMgmtAiConversationSessionProvider");
  }
  return v;
}
