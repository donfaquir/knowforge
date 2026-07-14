import { useEffect, useRef } from "react";

export interface GlobalShortcutActions {
  openAiPanel: () => void;
  toggleCommandPalette: () => void;
  triggerWritingCoach: () => void;
  openEditorFind: () => void;
}

export interface GlobalShortcutConditions {
  workspaceReady: boolean;
  editorUsable: boolean;
}

/**
 * Single window-level keydown listener for global shortcuts.
 * Uses refs to always read the latest condition values without re-registering the listener.
 */
export function useGlobalShortcuts(
  actions: GlobalShortcutActions,
  conditions: GlobalShortcutConditions,
): void {
  const workspaceReadyRef = useRef(conditions.workspaceReady);
  const editorUsableRef = useRef(conditions.editorUsable);
  workspaceReadyRef.current = conditions.workspaceReady;
  editorUsableRef.current = conditions.editorUsable;

  const actionsRef = useRef(actions);
  actionsRef.current = actions;

  useEffect(() => {
    const inEditableField = (t: EventTarget | null) =>
      t instanceof HTMLElement && t.closest("input, textarea, select, [contenteditable='true']");

    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) {
        return;
      }

      // Cmd+L / Ctrl+L: open AI panel (not in editable fields)
      if (!e.shiftKey && (e.key === "l" || e.key === "L")) {
        if (inEditableField(e.target)) {
          return;
        }
        e.preventDefault();
        actionsRef.current.openAiPanel();
        return;
      }

      // Cmd+Shift+P / Ctrl+Shift+P: toggle command palette (not in editable fields)
      if (e.shiftKey && (e.key === "p" || e.key === "P")) {
        if (inEditableField(e.target)) {
          return;
        }
        e.preventDefault();
        actionsRef.current.toggleCommandPalette();
        return;
      }

      // Cmd+Shift+W / Ctrl+Shift+W: trigger writing coach (needs editor usable)
      if (e.shiftKey && (e.key === "w" || e.key === "W")) {
        if (!editorUsableRef.current) {
          return;
        }
        e.preventDefault();
        actionsRef.current.triggerWritingCoach();
        return;
      }

      // Cmd+F / Ctrl+F: in-editor find (only when focus is in doc area)
      if (!e.shiftKey && (e.key === "f" || e.key === "F")) {
        const el = e.target;
        if (!(el instanceof HTMLElement)) {
          return;
        }
        if (el.closest("[data-editor-find-input]")) {
          return;
        }
        if (!workspaceReadyRef.current || !editorUsableRef.current) {
          return;
        }
        const inDoc = el.closest("[data-milkdown-root], .main__raw-doc-source, .editor-scroll__body");
        if (!inDoc) {
          return;
        }
        e.preventDefault();
        actionsRef.current.openEditorFind();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
