import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CrepeMarkdownEditorApi } from "./CrepeMarkdownEditor";
import "./EditorFindBar.css";

export type EditorFindWorkspaceJumpSeed = {
  query: string;
  caseSensitive: boolean;
  nonce: number;
};

type Props = {
  open: boolean;
  onClose: () => void;
  /** 与 App 中 showMarkdownSource 相反：false = 预览 / Crepe */
  previewMode: boolean;
  rawFullMarkdown: string;
  rawTextareaRef: React.RefObject<HTMLTextAreaElement | null>;
  crepeApiRef: React.RefObject<CrepeMarkdownEditorApi | null>;
  /** 换文时重置索引 */
  docKey: string | null;
  /** 全文搜索跳转：预填关键词并同步大小写；应用后父级应清空 */
  workspaceSearchJumpSeed?: EditorFindWorkspaceJumpSeed | null;
  onWorkspaceSearchJumpSeedConsumed?: () => void;
};

/** 非重叠 UTF-16 区间（与 textarea selection 对齐） */
function collectRawRanges(content: string, needle: string, caseSensitive: boolean): { start: number; end: number }[] {
  const q = needle.trim();
  if (!q) {
    return [];
  }
  const hay = caseSensitive ? content : content.toLowerCase();
  const ne = caseSensitive ? q : q.toLowerCase();
  const out: { start: number; end: number }[] = [];
  const step = Math.max(ne.length, 1);
  let pos = 0;
  while (pos < hay.length) {
    const idx = hay.indexOf(ne, pos);
    if (idx < 0) {
      break;
    }
    out.push({ start: idx, end: idx + q.length });
    pos = idx + step;
  }
  return out;
}

export function EditorFindBar({
  open,
  onClose,
  previewMode,
  rawFullMarkdown,
  rawTextareaRef,
  crepeApiRef,
  docKey,
  workspaceSearchJumpSeed = null,
  onWorkspaceSearchJumpSeedConsumed,
}: Props) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [rawIndex, setRawIndex] = useState(0);
  /** 预览模式：从 Crepe 插件读取的命中总数与当前序号（1-based） */
  const [previewMatchStats, setPreviewMatchStats] = useState<{
    total: number;
    current: number;
    truncated?: boolean;
  } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const rawRanges = useMemo(
    () => collectRawRanges(rawFullMarkdown, query, caseSensitive),
    [rawFullMarkdown, query, caseSensitive],
  );

  useEffect(() => {
    if (open) {
      queueMicrotask(() => inputRef.current?.focus());
    } else {
      setQuery("");
      setRawIndex(0);
      setPreviewMatchStats(null);
      crepeApiRef.current?.findClear();
    }
  }, [open, crepeApiRef]);

  /** 全文搜索命中打开文档后：写入查找框并通知 App 丢弃 seed */
  useEffect(() => {
    if (!open || !workspaceSearchJumpSeed) {
      return;
    }
    setQuery(workspaceSearchJumpSeed.query);
    setCaseSensitive(workspaceSearchJumpSeed.caseSensitive);
    queueMicrotask(() => {
      onWorkspaceSearchJumpSeedConsumed?.();
    });
  }, [open, workspaceSearchJumpSeed, onWorkspaceSearchJumpSeedConsumed]);

  useEffect(() => {
    setRawIndex(0);
  }, [docKey, query, caseSensitive, previewMode]);

  const applyPreviewFind = useCallback(() => {
    const api = crepeApiRef.current;
    if (!api || !previewMode) {
      return;
    }
    api.findSetQuery(query, caseSensitive);
  }, [crepeApiRef, previewMode, query, caseSensitive, docKey]);

  const syncPreviewMatchStats = useCallback(() => {
    if (!previewMode) {
      return;
    }
    if (!query.trim()) {
      setPreviewMatchStats(null);
      return;
    }
    const s = crepeApiRef.current?.getFindMatchSummary() ?? null;
    setPreviewMatchStats(s ?? { total: 0, current: 0 });
  }, [previewMode, query, crepeApiRef]);

  useEffect(() => {
    if (!previewMode) {
      setPreviewMatchStats(null);
    }
  }, [previewMode]);

  useEffect(() => {
    if (!open || !previewMode) {
      return;
    }
    const id = window.setTimeout(() => {
      applyPreviewFind();
      queueMicrotask(() => {
        syncPreviewMatchStats();
        inputRef.current?.focus({ preventScroll: true });
      });
    }, 120);
    return () => window.clearTimeout(id);
  }, [open, previewMode, applyPreviewFind, syncPreviewMatchStats]);

  useEffect(() => {
    if (!open || !previewMode) {
      return;
    }
    if (!query.trim()) {
      setPreviewMatchStats(null);
      return;
    }
    setPreviewMatchStats(null);
  }, [open, previewMode, query, caseSensitive, docKey]);

  const applyRawSelection = useCallback(
    (index: number) => {
      const ta = rawTextareaRef.current;
      if (!ta || rawRanges.length === 0) {
        return;
      }
      const i = ((index % rawRanges.length) + rawRanges.length) % rawRanges.length;
      const { start, end } = rawRanges[i];
      requestAnimationFrame(() => {
        ta.setSelectionRange(start, end);
        const lh = 17;
        const line = rawFullMarkdown.slice(0, start).split("\n").length;
        ta.scrollTop = Math.max(0, (line - 3) * lh);
        inputRef.current?.focus({ preventScroll: true });
      });
    },
    [rawFullMarkdown, rawRanges, rawTextareaRef],
  );

  useEffect(() => {
    if (!open || previewMode || rawRanges.length === 0) {
      return;
    }
    applyRawSelection(rawIndex);
  }, [open, previewMode, rawRanges, rawIndex, applyRawSelection]);

  const onNext = useCallback(() => {
    if (previewMode) {
      crepeApiRef.current?.findNext();
      queueMicrotask(() => {
        syncPreviewMatchStats();
        inputRef.current?.focus({ preventScroll: true });
      });
      return;
    }
    if (rawRanges.length === 0) {
      return;
    }
    setRawIndex((i) => (i + 1) % rawRanges.length);
  }, [previewMode, crepeApiRef, rawRanges.length, syncPreviewMatchStats]);

  const onPrev = useCallback(() => {
    if (previewMode) {
      crepeApiRef.current?.findPrev();
      queueMicrotask(() => {
        syncPreviewMatchStats();
        inputRef.current?.focus({ preventScroll: true });
      });
      return;
    }
    if (rawRanges.length === 0) {
      return;
    }
    setRawIndex((i) => (i - 1 + rawRanges.length) % rawRanges.length);
  }, [previewMode, crepeApiRef, rawRanges.length, syncPreviewMatchStats]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key === "Enter" && e.shiftKey) {
        e.preventDefault();
        onPrev();
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        onNext();
      }
      if (e.key === "F3") {
        e.preventDefault();
        if (e.shiftKey) {
          onPrev();
        } else {
          onNext();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose, onNext, onPrev]);

  const meta = useMemo(() => {
    if (!query.trim()) {
      return t("editorFind.keyHints");
    }
    if (previewMode) {
      if (previewMatchStats === null) {
        return t("editorFind.finding");
      }
      if (previewMatchStats.total === 0) {
        return t("editorFind.noMatch");
      }
      return t("editorFind.matchProgress", {
        total: previewMatchStats.truncated ? `${previewMatchStats.total}+` : previewMatchStats.total,
        current: previewMatchStats.current,
      });
    }
    if (rawRanges.length === 0) {
      return t("editorFind.noMatch");
    }
    return t("editorFind.matchProgress", { total: rawRanges.length, current: rawIndex + 1 });
  }, [query, previewMode, previewMatchStats, rawRanges.length, rawIndex, t]);

  if (!open) {
    return null;
  }

  return (
    <div className="editor-find-bar" role="search" aria-label={t("editorFind.title")}>
      <input
        ref={inputRef}
        data-editor-find-input
        className="editor-find-bar__input"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("editorFind.placeholder")}
        aria-label={t("editorFind.placeholder")}
      />
      <label className="editor-find-bar__case">
        <input type="checkbox" checked={caseSensitive} onChange={(e) => setCaseSensitive(e.target.checked)} />
        {t("editorFind.caseSensitive")}
      </label>
      <button
        type="button"
        className="editor-find-bar__btn editor-find-bar__btn--icon"
        onClick={onPrev}
        disabled={!query.trim()}
        title={t("editorFind.prev")}
        aria-label={t("editorFind.prev")}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden={true}>
          <path d="M15 18l-6-6 6-6" />
        </svg>
      </button>
      <button
        type="button"
        className="editor-find-bar__btn editor-find-bar__btn--icon"
        onClick={onNext}
        disabled={!query.trim()}
        title={t("editorFind.next")}
        aria-label={t("editorFind.next")}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden={true}>
          <path d="M9 18l6-6-6-6" />
        </svg>
      </button>
      <button
        type="button"
        className="editor-find-bar__btn editor-find-bar__btn--icon"
        onClick={onClose}
        title={t("editorFind.close")}
        aria-label={t("editorFind.close")}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden={true}>
          <path d="M18 6L6 18" />
          <path d="M6 6l12 12" />
        </svg>
      </button>
      <span className="editor-find-bar__meta">{meta}</span>
    </div>
  );
}
