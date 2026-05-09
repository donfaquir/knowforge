/** 想法管理页右侧栏：会话历史 / 新建 / 删除（与文档侧栏工具条视觉一致，数据源独立） */
import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState, type ButtonHTMLAttributes } from "react";
import { useTranslation } from "react-i18next";
import { useThoughtMgmtAiConversationSession } from "../contexts/ThoughtMgmtAiConversationSessionContext";
import type { ConversationMeta } from "../types/aiConversation";

const BULK_TOGGLE_STROKE = 1.65;

function IconHistory() {
  return (
    <svg
      className="file-tree__bulk-toggle-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={BULK_TOGGLE_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M3 3v5h5" />
      <path d="M3.05 13A9 9 0 1 0 6 5.3L3 8" />
      <path d="M12 7v5l4 2" />
    </svg>
  );
}

function IconPlusChat() {
  return (
    <svg
      className="file-tree__bulk-toggle-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={BULK_TOGGLE_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
      <path d="M12 7v6" />
      <path d="M9 10h6" />
    </svg>
  );
}

function IconTrash() {
  return (
    <svg
      className="file-tree__bulk-toggle-svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={BULK_TOGGLE_STROKE}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M3 6h18" />
      <path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
      <path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
      <line x1="10" x2="10" y1="11" y2="17" />
      <line x1="14" x2="14" y1="11" y2="17" />
    </svg>
  );
}

function formatConversationCreatedAt(createdAtMs: number, locale: string): string {
  const loc = locale.startsWith("zh") ? "zh-CN" : "en-US";
  try {
    return new Intl.DateTimeFormat(loc, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(createdAtMs));
  } catch {
    return new Date(createdAtMs).toLocaleString(loc);
  }
}

function shortConversationId(id: string): string {
  const compact = id.replace(/-/g, "");
  return compact.length > 8 ? `${compact.slice(0, 8)}…` : id;
}

function HistoryRow(props: {
  meta: ConversationMeta;
  active: boolean;
  onPick: () => void;
  dragExcludeProps: ButtonHTMLAttributes<HTMLButtonElement>;
  uiLocale: string;
}) {
  const { meta, active, onPick, dragExcludeProps, uiLocale } = props;
  return (
    <button
      type="button"
      role="listitem"
      className={`ai-chat__history-row${active ? " ai-chat__history-row--active" : ""}`}
      onClick={onPick}
      {...dragExcludeProps}
    >
      <span className="ai-chat__history-row-title">{meta.title}</span>
      <div className="ai-chat__history-row-meta">
        <span className="ai-chat__history-row-id" title={meta.id}>
          {shortConversationId(meta.id)}
        </span>
        <span className="ai-chat__history-row-time">
          {formatConversationCreatedAt(meta.createdAt, uiLocale)}
        </span>
      </div>
    </button>
  );
}

export function ThoughtMgmtAiConversationToolbar() {
  const { t, i18n } = useTranslation();
  const {
    conversationId,
    conversations,
    sessionReady,
    switchConversation,
    createConversation,
    deleteConversation,
    isStreaming,
    isVaultSearching,
    workspaceReady,
    tauriRuntime,
    thoughtFocusContext,
  } = useThoughtMgmtAiConversationSession();

  const dragExcludeProps = tauriRuntime
    ? ({ "data-tauri-drag-region-exclude": true } as const)
    : {};

  const [historyPanelOpen, setHistoryPanelOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement>(null);

  const sessionSwitchDisabled = isStreaming || isVaultSearching || !sessionReady || !workspaceReady;

  const handleDeleteChat = useCallback(async () => {
    if (!conversationId || isStreaming || isVaultSearching || !workspaceReady) {
      return;
    }
    const ok = await ask(t("dialogs.deleteConversation"), {
      title: t("dialogs.deleteChat"),
      kind: "warning",
    });
    if (!ok) {
      return;
    }
    await deleteConversation(conversationId);
  }, [conversationId, deleteConversation, isStreaming, isVaultSearching, workspaceReady, t]);

  const pickConversation = useCallback(
    (id: string) => {
      if (!id || id === conversationId) {
        setHistoryPanelOpen(false);
        return;
      }
      void switchConversation(id).then(() => setHistoryPanelOpen(false));
    },
    [conversationId, switchConversation],
  );

  useEffect(() => {
    if (!historyPanelOpen) {
      return;
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setHistoryPanelOpen(false);
      }
    };
    const onPointerDown = (e: MouseEvent | PointerEvent) => {
      const el = anchorRef.current;
      if (el && !el.contains(e.target as Node)) {
        setHistoryPanelOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown, true);
    };
  }, [historyPanelOpen]);

  return (
    <div className="thought-mgmt__ai-toolbar" ref={anchorRef} {...dragExcludeProps}>
      <button
        type="button"
        className="file-tree__bulk-toggle"
        aria-label={t("aiToolbar.history")}
        aria-expanded={historyPanelOpen}
        aria-haspopup="dialog"
        disabled={sessionSwitchDisabled}
        title={t("aiToolbar.history")}
        onClick={() => setHistoryPanelOpen((o) => !o)}
        {...dragExcludeProps}
      >
        <IconHistory />
      </button>
      {historyPanelOpen ? (
        <div
          className="ai-chat__history-panel ai-chat__history-panel--toolbar-end"
          role="dialog"
          aria-label={t("aiToolbar.history")}
          {...dragExcludeProps}
        >
          <div className="ai-chat__history-panel-header">{t("aiToolbar.conversations")}</div>
          <div className="ai-chat__history-panel-list" role="list">
            {!sessionReady ? (
              <div className="ai-chat__history-panel-empty">{t("aiToolbar.loading")}</div>
            ) : conversations.length === 0 ? (
              <div className="ai-chat__history-panel-empty">{t("aiToolbar.noConversations")}</div>
            ) : (
              [...conversations]
                .sort((a, b) => b.updatedAt - a.updatedAt)
                .map((c) => (
                  <HistoryRow
                    key={c.id}
                    meta={c}
                    active={c.id === conversationId}
                    onPick={() => pickConversation(c.id)}
                    dragExcludeProps={dragExcludeProps as ButtonHTMLAttributes<HTMLButtonElement>}
                    uiLocale={i18n.language}
                  />
                ))
            )}
          </div>
        </div>
      ) : null}
      <button
        type="button"
        className="file-tree__bulk-toggle"
        aria-label={t("aiToolbar.newChat")}
        disabled={sessionSwitchDisabled}
        title={t("aiToolbar.newChat")}
        onClick={() => {
          setHistoryPanelOpen(false);
          void createConversation(thoughtFocusContext);
        }}
        {...dragExcludeProps}
      >
        <IconPlusChat />
      </button>
      <button
        type="button"
        className="file-tree__bulk-toggle file-tree__bulk-toggle--danger"
        aria-label={t("aiToolbar.deleteCurrent")}
        disabled={sessionSwitchDisabled || !conversationId || conversations.length === 0}
        title={t("aiToolbar.deleteCurrent")}
        onClick={() => void handleDeleteChat()}
        {...dragExcludeProps}
      >
        <IconTrash />
      </button>
    </div>
  );
}
