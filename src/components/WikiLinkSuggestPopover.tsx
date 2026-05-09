import type { EditorView } from "@milkdown/prose/view";
import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import type { WikiSuggestFileRow } from "../utils/flattenMarkdownTreeForWikiSuggest";
import {
  clearWikiSuggestDismiss,
  dismissWikiSuggestAtAnchor,
  getWikiSuggestSnapshot,
  subscribeWikiSuggest,
} from "./wikiLinkSuggestStore";
import "./WikiLinkSuggestPopover.css";

type Props = {
  getEditorView: () => EditorView | null;
  wikiSuggestFiles: WikiSuggestFileRow[];
};

export function WikiLinkSuggestPopover({ getEditorView, wikiSuggestFiles }: Props) {
  const { t } = useTranslation();
  const snap = useSyncExternalStore(subscribeWikiSuggest, getWikiSuggestSnapshot, getWikiSuggestSnapshot);
  const panelRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    if (!snap.open) {
      return [];
    }
    const q = snap.filter.trim().toLowerCase();
    if (!q) {
      return wikiSuggestFiles;
    }
    return wikiSuggestFiles.filter(
      (r) =>
        r.displayName.toLowerCase().includes(q) ||
        r.relPath.toLowerCase().includes(q) ||
        r.insertLabel.toLowerCase().includes(q),
    );
  }, [snap, wikiSuggestFiles]);

  const style = useMemo(() => {
    if (!snap.open) {
      return undefined;
    }
    const view = getEditorView();
    if (!view) {
      return { display: "none" as const };
    }
    try {
      const coords = view.coordsAtPos(snap.head);
      return {
        position: "fixed" as const,
        left: Math.min(coords.left, window.innerWidth - 280),
        top: coords.bottom + 6,
        zIndex: 80,
      };
    } catch {
      return { display: "none" as const };
    }
  }, [snap, getEditorView]);

  const applyPick = useCallback(
    (row: WikiSuggestFileRow) => {
      if (!snap.open) {
        return;
      }
      const view = getEditorView();
      if (!view) {
        return;
      }
      const { anchor, head } = snap;
      const text = `[[${row.insertLabel}]]`;
      const node = view.state.schema.text(text);
      const tr = view.state.tr.replaceWith(anchor, head, node);
      view.dispatch(tr);
      clearWikiSuggestDismiss();
      view.focus();
    },
    [snap, getEditorView],
  );

  useEffect(() => {
    if (!snap.open) {
      return;
    }
    const onDocMouseDown = (e: MouseEvent) => {
      const el = panelRef.current;
      if (el && e.target instanceof Node && el.contains(e.target)) {
        return;
      }
      const cur = getWikiSuggestSnapshot();
      if (cur.open) {
        dismissWikiSuggestAtAnchor(cur.anchor);
      }
    };
    document.addEventListener("mousedown", onDocMouseDown, true);
    return () => document.removeEventListener("mousedown", onDocMouseDown, true);
  }, [snap]);

  if (!snap.open || !style || style.display === "none") {
    return null;
  }

  return (
    <div
      ref={panelRef}
      className="kf-wiki-suggest"
      style={style}
      role="listbox"
      aria-label={t("editor.wikiSuggestListLabel")}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          dismissWikiSuggestAtAnchor(snap.anchor);
          e.preventDefault();
          e.stopPropagation();
        }
      }}
    >
      <div className="kf-wiki-suggest__scroll">
        {filtered.length === 0 ? (
          <p className="kf-wiki-suggest__empty">{t("editor.wikiSuggestEmpty")}</p>
        ) : (
          filtered.map((row) => (
            <button
              key={row.relPath}
              type="button"
              role="option"
              className="kf-wiki-suggest__row"
              onMouseDown={(e) => {
                e.preventDefault();
                applyPick(row);
              }}
            >
              <span className="kf-wiki-suggest__name">{row.displayName}</span>
              <span className="kf-wiki-suggest__path">{row.relPath}</span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
