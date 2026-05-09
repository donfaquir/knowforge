import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { DocState } from "../hooks/useOpenDocs";
import type { AiCurrentNoteContext } from "../types/aiNoteContext";

type Snapshot = {
  workspaceReady: boolean;
  activePath: string | null;
  docByPath: Record<string, DocState>;
};

export type AiNoteBridgeValue = {
  attachCurrentNote: boolean;
  setAttachCurrentNote: (value: boolean) => void;
  getCurrentNoteContext: () => AiCurrentNoteContext;
  /** 当前活动文档的相对路径（仅用于 UI 展示，不触发按键级重渲染） */
  activePath: string | null;
  /** 从 AI 侧打开笔记标签（例如成熟度 Toast、引用标签跳转） */
  openMarkdownTab?: (relPath: string) => void;
};

const AiNoteBridgeContext = createContext<AiNoteBridgeValue | null>(null);

type ProviderProps = {
  children: ReactNode;
  workspaceReady: boolean;
  activePath: string | null;
  docByPath: Record<string, DocState>;
  openMarkdownTab?: (relPath: string) => void;
};

/**
 * 将活动文档快照写入 ref（每帧同步），避免把 markdown 放进 Context 导致 AI 栏随按键重渲染。
 */
export function AiNoteContextProvider({
  children,
  workspaceReady,
  activePath,
  docByPath,
  openMarkdownTab,
}: ProviderProps) {
  const [attachCurrentNote, setAttachCurrentNote] = useState(true);
  const snapshotRef = useRef<Snapshot>({
    workspaceReady: false,
    activePath: null,
    docByPath: {},
  });

  snapshotRef.current = {
    workspaceReady,
    activePath,
    docByPath,
  };

  const getCurrentNoteContext = useCallback((): AiCurrentNoteContext => {
    if (!attachCurrentNote) {
      return { kind: "detached" };
    }
    const { workspaceReady: wr, activePath: ap, docByPath: db } = snapshotRef.current;
    if (!wr) {
      return { kind: "unavailable", reason: "no_workspace" };
    }
    if (!ap) {
      return { kind: "none" };
    }
    const doc = db[ap];
    if (!doc) {
      return { kind: "none" };
    }
    if (doc.loading) {
      return { kind: "unavailable", reason: "loading" };
    }
    if (doc.loadError) {
      return { kind: "unavailable", reason: "load_error" };
    }
    return {
      kind: "attached",
      relPath: ap,
      markdown: doc.content,
      anchor: null,
    };
  }, [attachCurrentNote]);

  const value = useMemo(
    () => ({
      attachCurrentNote,
      setAttachCurrentNote,
      getCurrentNoteContext,
      activePath,
      openMarkdownTab,
    }),
    [attachCurrentNote, getCurrentNoteContext, activePath, openMarkdownTab],
  );

  return <AiNoteBridgeContext.Provider value={value}>{children}</AiNoteBridgeContext.Provider>;
}

export function useAiNoteContext(): AiNoteBridgeValue {
  const v = useContext(AiNoteBridgeContext);
  if (v == null) {
    throw new Error("useAiNoteContext must be used within AiNoteContextProvider");
  }
  return v;
}
