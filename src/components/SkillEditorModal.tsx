import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  SkillManifestJson,
  SkillUiEntry,
  ToolSummary,
} from "../types/skillTypes";
import "./SkillEditorModal.css";

export type SkillEditorModalProps = {
  open: boolean;
  mode: "edit" | "create";
  /** Snapshot used to seed the local draft when the modal opens. */
  initialManifest: SkillManifestJson;
  /** Available tool inventory for the allowed-tools picker. */
  tools: ToolSummary[];
  workspaceReady: boolean;
  /** Persists the manifest. May throw — caller surfaces backend errors via rejection. */
  onSave: (manifest: SkillManifestJson) => Promise<void>;
  onClose: () => void;
};

const UI_ENTRY_OPTIONS: SkillUiEntry[] = [
  "conversation_mode",
  "editor_panel",
  "standalone",
];

/** valid id: lowercase letters, digits, underscore; 2–64 chars, must start with letter. */
function isValidSkillId(id: string): boolean {
  return /^[a-z][a-z0-9_]{1,63}$/.test(id);
}

function parseTagsInput(raw: string): string[] {
  return raw
    .split(",")
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}

function IconClose() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

export function SkillEditorModal(props: SkillEditorModalProps) {
  const {
    open,
    mode,
    initialManifest,
    tools,
    workspaceReady,
    onSave,
    onClose,
  } = props;
  const { t } = useTranslation();

  const [draft, setDraft] = useState<SkillManifestJson>(initialManifest);
  const [tagsRaw, setTagsRaw] = useState<string>(
    (initialManifest.tags ?? []).join(", "),
  );
  const [allowedToolsFilter, setAllowedToolsFilter] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-seed local state every time the modal opens with a new initialManifest.
  // Track the previous open state so we don't clobber edits on every render.
  const prevOpenRef = useRef(false);
  useEffect(() => {
    if (open && !prevOpenRef.current) {
      setDraft(initialManifest);
      setTagsRaw((initialManifest.tags ?? []).join(", "));
      setAllowedToolsFilter("");
      setError(null);
      setBusy(false);
    }
    prevOpenRef.current = open;
  }, [open, initialManifest]);

  // ESC closes (when not busy)
  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  const filteredTools = useMemo(() => {
    const q = allowedToolsFilter.trim().toLowerCase();
    if (q.length === 0) {
      return tools;
    }
    return tools.filter(
      (tool) =>
        tool.name.toLowerCase().includes(q) ||
        tool.description.toLowerCase().includes(q),
    );
  }, [tools, allowedToolsFilter]);

  const allowedSet = useMemo(
    () => new Set(draft.allowedTools),
    [draft.allowedTools],
  );
  const orphanAllowed = useMemo(
    () => draft.allowedTools.filter((n) => !tools.some((tl) => tl.name === n)),
    [draft.allowedTools, tools],
  );

  const toggleTool = useCallback((name: string) => {
    setDraft((d) => {
      const has = d.allowedTools.includes(name);
      return {
        ...d,
        allowedTools: has
          ? d.allowedTools.filter((n) => n !== name)
          : [...d.allowedTools, name],
      };
    });
  }, []);

  const validateDraft = useCallback((): string | null => {
    if (!isValidSkillId(draft.id)) {
      return t("skillMgmt.errInvalidId");
    }
    if (draft.name.trim().length === 0) {
      return t("skillMgmt.errNameRequired");
    }
    if (draft.systemPromptTemplate.trim().length === 0) {
      return t("skillMgmt.errPromptRequired");
    }
    if (
      !Number.isFinite(draft.maxToolCalls) ||
      draft.maxToolCalls < 0 ||
      draft.maxToolCalls > 64
    ) {
      return t("skillMgmt.errMaxToolCalls");
    }
    if (
      !Number.isFinite(draft.timeoutSecs) ||
      draft.timeoutSecs < 5 ||
      draft.timeoutSecs > 1800
    ) {
      return t("skillMgmt.errTimeout");
    }
    return null;
  }, [draft, t]);

  const submit = useCallback(async () => {
    const err = validateDraft();
    if (err) {
      setError(err);
      return;
    }
    const payload: SkillManifestJson = {
      ...draft,
      tags: parseTagsInput(tagsRaw),
      whenToUse:
        draft.whenToUse && draft.whenToUse.trim().length > 0
          ? draft.whenToUse.trim()
          : null,
    };
    setBusy(true);
    setError(null);
    try {
      await onSave(payload);
      // parent is expected to close the modal on success
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [draft, tagsRaw, validateDraft, onSave]);

  if (!open) {
    return null;
  }

  const idLocked = mode === "edit";
  const cantWrite = !workspaceReady;
  const titleText =
    mode === "create"
      ? t("skillMgmt.editorTitleCreate")
      : t("skillMgmt.editorTitleEdit", { name: draft.name || draft.id });

  return (
    <div
      className="app-modal-scrim skill-editor__scrim"
      role="presentation"
      onClick={() => {
        if (!busy) {
          onClose();
        }
      }}
    >
      <div
        className="app-modal app-modal--editor-lg skill-editor"
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-editor-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="skill-editor__header">
          <h2 id="skill-editor-title" className="skill-editor__title">
            {titleText}
          </h2>
          <button
            type="button"
            className="skill-editor__close"
            aria-label={t("skillMgmt.close")}
            title={t("skillMgmt.close")}
            disabled={busy}
            onClick={onClose}
          >
            <IconClose />
          </button>
        </header>

        <div className="skill-editor__body">
          {/* ── Left column: META ──────────────────────────────────────── */}
          <section
            className="skill-editor__col skill-editor__col--meta"
            aria-label={t("skillMgmt.editorMetaCol")}
          >
            <h3 className="skill-editor__col-label">
              {t("skillMgmt.editorMetaCol")}
            </h3>

            <div className="skill-editor__field">
              <label className="skill-editor__label" htmlFor="skill-editor-id">
                {t("skillMgmt.fieldId")}
              </label>
              <input
                id="skill-editor-id"
                className="skill-editor__input skill-editor__input--mono"
                value={draft.id}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, id: e.target.value.trim() }))
                }
                placeholder="my_skill"
                autoComplete="off"
                disabled={busy || idLocked}
              />
              <p className="skill-editor__hint">
                {t("skillMgmt.fieldIdHint")}
              </p>
            </div>

            <div className="skill-editor__field">
              <label
                className="skill-editor__label"
                htmlFor="skill-editor-name"
              >
                {t("skillMgmt.fieldName")}
              </label>
              <input
                id="skill-editor-name"
                className="skill-editor__input"
                value={draft.name}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, name: e.target.value }))
                }
                autoComplete="off"
                disabled={busy}
              />
            </div>

            <div className="skill-editor__row-2">
              <div className="skill-editor__field">
                <label
                  className="skill-editor__label"
                  htmlFor="skill-editor-version"
                >
                  {t("skillMgmt.fieldVersion")}
                </label>
                <input
                  id="skill-editor-version"
                  className="skill-editor__input skill-editor__input--mono"
                  value={draft.version}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, version: e.target.value }))
                  }
                  autoComplete="off"
                  disabled={busy}
                />
              </div>
              <div className="skill-editor__field">
                <label
                  className="skill-editor__label"
                  htmlFor="skill-editor-uientry"
                >
                  {t("skillMgmt.fieldUiEntry")}
                </label>
                <select
                  id="skill-editor-uientry"
                  className="skill-editor__input"
                  value={draft.uiEntry}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      uiEntry: e.target.value as SkillUiEntry,
                    }))
                  }
                  disabled={busy}
                >
                  {UI_ENTRY_OPTIONS.map((opt) => (
                    <option key={opt} value={opt}>
                      {t(`skillMgmt.uiEntry.${opt}`)}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            <div className="skill-editor__field">
              <label
                className="skill-editor__label"
                htmlFor="skill-editor-desc"
              >
                {t("skillMgmt.fieldDescription")}
              </label>
              <textarea
                id="skill-editor-desc"
                className="skill-editor__input skill-editor__textarea"
                rows={2}
                value={draft.description}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, description: e.target.value }))
                }
                disabled={busy}
              />
            </div>

            <div className="skill-editor__field">
              <label
                className="skill-editor__label"
                htmlFor="skill-editor-when"
              >
                {t("skillMgmt.fieldWhenToUse")}
              </label>
              <textarea
                id="skill-editor-when"
                className="skill-editor__input skill-editor__textarea"
                rows={2}
                value={draft.whenToUse ?? ""}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, whenToUse: e.target.value }))
                }
                disabled={busy}
              />
            </div>

            <div className="skill-editor__row-2">
              <div className="skill-editor__field">
                <label
                  className="skill-editor__label"
                  htmlFor="skill-editor-max"
                >
                  {t("skillMgmt.fieldMaxToolCalls")}
                </label>
                <input
                  id="skill-editor-max"
                  className="skill-editor__input skill-editor__input--mono"
                  type="number"
                  min={0}
                  max={64}
                  value={draft.maxToolCalls}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      maxToolCalls: Number.parseInt(e.target.value, 10) || 0,
                    }))
                  }
                  disabled={busy}
                />
              </div>
              <div className="skill-editor__field">
                <label
                  className="skill-editor__label"
                  htmlFor="skill-editor-timeout"
                >
                  {t("skillMgmt.fieldTimeoutSecs")}
                </label>
                <input
                  id="skill-editor-timeout"
                  className="skill-editor__input skill-editor__input--mono"
                  type="number"
                  min={5}
                  max={1800}
                  value={draft.timeoutSecs}
                  onChange={(e) =>
                    setDraft((d) => ({
                      ...d,
                      timeoutSecs: Number.parseInt(e.target.value, 10) || 0,
                    }))
                  }
                  disabled={busy}
                />
              </div>
            </div>

            <div className="skill-editor__field">
              <label
                className="skill-editor__label"
                htmlFor="skill-editor-tags"
              >
                {t("skillMgmt.fieldTags")}
              </label>
              <input
                id="skill-editor-tags"
                className="skill-editor__input"
                value={tagsRaw}
                onChange={(e) => setTagsRaw(e.target.value)}
                placeholder={t("skillMgmt.fieldTagsPlaceholder")}
                disabled={busy}
              />
            </div>

            <label className="skill-editor__check">
              <input
                type="checkbox"
                checked={draft.autoInvocable ?? true}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, autoInvocable: e.target.checked }))
                }
                disabled={busy}
              />
              <span className="skill-editor__check-label">
                {t("skillMgmt.fieldAutoInvocable")}
              </span>
            </label>
            <p className="skill-editor__hint skill-editor__hint--tight">
              {t("skillMgmt.fieldAutoInvocableHint")}
            </p>
          </section>

          {/* ── Right column: PROMPT + TOOLS ──────────────────────────── */}
          <section
            className="skill-editor__col skill-editor__col--prompt"
            aria-label={t("skillMgmt.editorPromptCol")}
          >
            <h3 className="skill-editor__col-label">
              {t("skillMgmt.editorPromptCol")}
            </h3>

            <div className="skill-editor__field skill-editor__field--prompt">
              <label
                className="skill-editor__label"
                htmlFor="skill-editor-prompt"
              >
                {t("skillMgmt.fieldPrompt")}
              </label>
              <textarea
                id="skill-editor-prompt"
                className="skill-editor__input skill-editor__prompt-area"
                value={draft.systemPromptTemplate}
                onChange={(e) =>
                  setDraft((d) => ({
                    ...d,
                    systemPromptTemplate: e.target.value,
                  }))
                }
                disabled={busy}
                placeholder="You are working in {{workspace_name}} at {{workspace_root}} …"
                spellCheck={false}
              />
              <p className="skill-editor__hint">
                {t("skillMgmt.fieldPromptHint")}
              </p>
            </div>

            <div className="skill-editor__field skill-editor__field--tools">
              <label className="skill-editor__label">
                {t("skillMgmt.fieldAllowedTools", {
                  n: draft.allowedTools.length,
                })}
              </label>
              <input
                className="skill-editor__input skill-editor__tool-filter"
                value={allowedToolsFilter}
                onChange={(e) => setAllowedToolsFilter(e.target.value)}
                placeholder={t("skillMgmt.toolsFilterPlaceholder")}
                disabled={busy}
              />
              <div className="skill-editor__tool-list" role="group">
                {filteredTools.length === 0 ? (
                  <p className="skill-editor__tool-empty">
                    {t("skillMgmt.toolsEmpty")}
                  </p>
                ) : (
                  filteredTools.map((tool) => {
                    const checked = allowedSet.has(tool.name);
                    return (
                      <label
                        key={tool.name}
                        className={`skill-editor__tool-item${
                          checked ? " skill-editor__tool-item--on" : ""
                        }`}
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() => toggleTool(tool.name)}
                          disabled={busy}
                        />
                        <span className="skill-editor__tool-name">
                          {tool.name}
                        </span>
                        {tool.description ? (
                          <span className="skill-editor__tool-desc">
                            {tool.description}
                          </span>
                        ) : null}
                      </label>
                    );
                  })
                )}
              </div>
              {orphanAllowed.length > 0 ? (
                <p className="skill-editor__hint skill-editor__tool-orphan">
                  {t("skillMgmt.toolsOrphan", {
                    tools: orphanAllowed.join(", "),
                  })}
                </p>
              ) : null}
            </div>
          </section>
        </div>

        <footer className="skill-editor__footer">
          <div className="skill-editor__footer-msg" aria-live="polite">
            {error ? (
              <p className="skill-editor__banner skill-editor__banner--error">
                {error}
              </p>
            ) : cantWrite ? (
              <p className="skill-editor__banner skill-editor__banner--warn">
                {t("skillMgmt.workspaceWarning")}
              </p>
            ) : null}
          </div>
          <div className="skill-editor__footer-actions">
            <button
              type="button"
              className="skill-editor__btn"
              disabled={busy}
              onClick={onClose}
            >
              {t("skillMgmt.cancel")}
            </button>
            <button
              type="button"
              className="skill-editor__btn skill-editor__btn--primary"
              disabled={busy || cantWrite}
              onClick={() => void submit()}
            >
              {busy
                ? t("skillMgmt.saving")
                : mode === "create"
                  ? t("skillMgmt.create")
                  : t("skillMgmt.save")}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

export default SkillEditorModal;
