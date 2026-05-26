import { ask } from "@tauri-apps/plugin-dialog";
import { isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  SkillListItemJson,
  SkillManifestJson,
  SkillUiEntry,
  ToolSummary,
} from "../types/skillTypes";
import {
  createCustomSkill,
  deleteCustomSkill,
  listAvailableTools,
  listSkillItems,
  reloadCustomSkills,
  updateCustomSkill,
} from "../utils/skillInvoke";
import SkillEditorModal from "./SkillEditorModal";
import "./SkillManagementPanel.css";

export type TauriDragRegionExcludeProps =
  | { readonly "data-tauri-drag-region-exclude": true }
  | Record<string, never>;

export type SkillManagementPanelProps = {
  open: boolean;
  onClose: () => void;
  workspaceReady?: boolean;
  tauriRuntime?: boolean;
  dragExcludeProps?: TauriDragRegionExcludeProps;
  /** When true, renders only the inner content without the modal shell (scrim/dialog/close button). */
  embedded?: boolean;
};

type PanelMode = "idle" | "view" | "edit" | "create";

const UI_ENTRY_OPTIONS: SkillUiEntry[] = [
  "conversation_mode",
  "editor_panel",
  "standalone",
];

const DEFAULT_MAX_TOOL_CALLS = 8;
const DEFAULT_TIMEOUT_SECS = 60;

function emptyManifest(): SkillManifestJson {
  return {
    id: "",
    name: "",
    version: "1.0.0",
    description: "",
    systemPromptTemplate: "",
    allowedTools: [],
    maxToolCalls: DEFAULT_MAX_TOOL_CALLS,
    timeoutSecs: DEFAULT_TIMEOUT_SECS,
    uiEntry: "conversation_mode",
    tags: [],
    autoInvocable: true,
    whenToUse: null,
  };
}

function manifestFromListItem(item: SkillListItemJson): SkillManifestJson {
  return {
    id: item.id,
    name: item.name,
    version: item.version,
    description: item.description,
    systemPromptTemplate: item.systemPromptTemplate,
    allowedTools: [...item.allowedTools],
    maxToolCalls: item.maxToolCalls,
    timeoutSecs: item.timeoutSecs,
    uiEntry: item.uiEntry,
    tags: [...item.tags],
    autoInvocable: item.autoInvocable ?? true,
    whenToUse: item.whenToUse ?? null,
  };
}

/** valid id: lowercase letters, digits, underscore; 2-64 chars. */
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

function IconReload() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
      <path d="M16 16h5v5" />
    </svg>
  );
}

function IconPlus() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  );
}

export function SkillManagementPanel(props: SkillManagementPanelProps) {
  const {
    open,
    onClose,
    workspaceReady = true,
    tauriRuntime = isTauri(),
    dragExcludeProps = (tauriRuntime
      ? { "data-tauri-drag-region-exclude": true }
      : {}) as TauriDragRegionExcludeProps,
    embedded = false,
  } = props;
  const { t } = useTranslation();

  const disposedRef = useRef(false);
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
    };
  }, []);

  const [skills, setSkills] = useState<SkillListItemJson[]>([]);
  const [tools, setTools] = useState<ToolSummary[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<PanelMode>("idle");
  const [draft, setDraft] = useState<SkillManifestJson>(emptyManifest);
  /** mirror of draft.tags rendered as a comma-separated text input */
  const [tagsRaw, setTagsRaw] = useState<string>("");
  const [allowedToolsFilter, setAllowedToolsFilter] = useState<string>("");

  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  // Embedded mode: edit/create open a large dedicated editor modal instead of
  // switching the inline detail pane into form mode.
  const [editorOpen, setEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"edit" | "create">("create");
  const [editorInitial, setEditorInitial] = useState<SkillManifestJson>(
    emptyManifest,
  );

  const refreshSkills = useCallback(async () => {
    if (!tauriRuntime) {
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const list = await listSkillItems();
      if (!disposedRef.current) {
        setSkills(list);
      }
    } catch (e) {
      if (!disposedRef.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (!disposedRef.current) {
        setLoading(false);
      }
    }
  }, [tauriRuntime]);

  const refreshTools = useCallback(async () => {
    if (!tauriRuntime) {
      return;
    }
    try {
      const list = await listAvailableTools();
      if (!disposedRef.current) {
        setTools(list);
      }
    } catch {
      // tool list is auxiliary; surfacing here would muddle the primary error banner
    }
  }, [tauriRuntime]);

  useEffect(() => {
    if (!open) {
      return;
    }
    void refreshSkills();
    void refreshTools();
  }, [open, refreshSkills, refreshTools]);

  useEffect(() => {
    if (!open) {
      // reset transient state on close so reopening starts clean
      setMode("idle");
      setSelectedId(null);
      setDraft(emptyManifest());
      setTagsRaw("");
      setAllowedToolsFilter("");
      setError(null);
      setInfo(null);
    }
  }, [open]);

  const selectedItem = useMemo(
    () => skills.find((s) => s.id === selectedId) ?? null,
    [skills, selectedId],
  );

  const beginCreate = useCallback(() => {
    if (embedded) {
      // Embedded panel: pop out the large editor modal so the user has
      // adequate space (especially for the system prompt textarea).
      setEditorInitial(emptyManifest());
      setEditorMode("create");
      setEditorOpen(true);
      setError(null);
      setInfo(null);
      return;
    }
    const blank = emptyManifest();
    setDraft(blank);
    setTagsRaw("");
    setAllowedToolsFilter("");
    setMode("create");
    setSelectedId(null);
    setError(null);
    setInfo(null);
  }, [embedded]);

  const beginView = useCallback((item: SkillListItemJson) => {
    const m = manifestFromListItem(item);
    setDraft(m);
    setTagsRaw(m.tags.join(", "));
    setAllowedToolsFilter("");
    setSelectedId(item.id);
    setMode("view");
    setError(null);
    setInfo(null);
  }, []);

  const beginEdit = useCallback(
    (item: SkillListItemJson) => {
      if (item.isBuiltin) {
        return;
      }
      if (embedded) {
        setEditorInitial(manifestFromListItem(item));
        setEditorMode("edit");
        setEditorOpen(true);
        setSelectedId(item.id);
        setError(null);
        setInfo(null);
        return;
      }
      const m = manifestFromListItem(item);
      setDraft(m);
      setTagsRaw(m.tags.join(", "));
      setAllowedToolsFilter("");
      setSelectedId(item.id);
      setMode("edit");
      setError(null);
      setInfo(null);
    },
    [embedded],
  );

  const handleEditorSave = useCallback(
    async (manifest: SkillManifestJson) => {
      if (editorMode === "create") {
        await createCustomSkill(manifest);
        if (!disposedRef.current) {
          setInfo(t("skillMgmt.createdInfo", { id: manifest.id }));
        }
      } else {
        await updateCustomSkill(manifest);
        if (!disposedRef.current) {
          setInfo(t("skillMgmt.updatedInfo", { id: manifest.id }));
        }
      }
      await refreshSkills();
      if (!disposedRef.current) {
        setEditorOpen(false);
        setSelectedId(manifest.id);
        setMode("view");
        // Keep the inline draft in sync so the view pane reflects the saved data.
        setDraft(manifest);
        setTagsRaw(manifest.tags.join(", "));
      }
    },
    [editorMode, refreshSkills, t],
  );

  const cancelEdit = useCallback(() => {
    if (selectedItem) {
      beginView(selectedItem);
    } else {
      setMode("idle");
      setDraft(emptyManifest());
      setTagsRaw("");
    }
  }, [selectedItem, beginView]);

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
    if (!Number.isFinite(draft.maxToolCalls) || draft.maxToolCalls < 0 || draft.maxToolCalls > 64) {
      return t("skillMgmt.errMaxToolCalls");
    }
    if (!Number.isFinite(draft.timeoutSecs) || draft.timeoutSecs < 5 || draft.timeoutSecs > 1800) {
      return t("skillMgmt.errTimeout");
    }
    return null;
  }, [draft, t]);

  const submitDraft = useCallback(async () => {
    const err = validateDraft();
    if (err) {
      setError(err);
      return;
    }
    const payload: SkillManifestJson = {
      ...draft,
      tags: parseTagsInput(tagsRaw),
      whenToUse:
        draft.whenToUse && draft.whenToUse.trim().length > 0 ? draft.whenToUse.trim() : null,
    };
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      if (mode === "create") {
        await createCustomSkill(payload);
        setInfo(t("skillMgmt.createdInfo", { id: payload.id }));
      } else if (mode === "edit") {
        await updateCustomSkill(payload);
        setInfo(t("skillMgmt.updatedInfo", { id: payload.id }));
      }
      await refreshSkills();
      if (!disposedRef.current) {
        setSelectedId(payload.id);
        setMode("view");
      }
    } catch (e) {
      if (!disposedRef.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (!disposedRef.current) {
        setBusy(false);
      }
    }
  }, [draft, tagsRaw, mode, validateDraft, refreshSkills, t]);

  const requestDelete = useCallback(
    async (item: SkillListItemJson) => {
      if (item.isBuiltin) {
        return;
      }
      const message = t("skillMgmt.confirmDelete", { name: item.name });
      const ok = isTauri()
        ? await ask(message, { title: t("skillMgmt.confirmDeleteTitle"), kind: "warning" })
        : window.confirm(message);
      if (!ok) {
        return;
      }
      setBusy(true);
      setError(null);
      setInfo(null);
      try {
        await deleteCustomSkill(item.id);
        if (!disposedRef.current) {
          setInfo(t("skillMgmt.deletedInfo", { id: item.id }));
          if (selectedId === item.id) {
            setSelectedId(null);
            setMode("idle");
            setDraft(emptyManifest());
            setTagsRaw("");
          }
        }
        await refreshSkills();
      } catch (e) {
        if (!disposedRef.current) {
          setError(e instanceof Error ? e.message : String(e));
        }
      } finally {
        if (!disposedRef.current) {
          setBusy(false);
        }
      }
    },
    [refreshSkills, selectedId, t],
  );

  const handleReload = useCallback(async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const res = await reloadCustomSkills();
      if (!disposedRef.current) {
        if (res.failed.length > 0) {
          const failedSummary = res.failed.map((f) => `${f.file}: ${f.error}`).join("; ");
          setError(t("skillMgmt.reloadPartial", { failed: failedSummary }));
        }
        setInfo(t("skillMgmt.reloadInfo", { loaded: res.loaded.length, failed: res.failed.length }));
      }
      await refreshSkills();
    } catch (e) {
      if (!disposedRef.current) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (!disposedRef.current) {
        setBusy(false);
      }
    }
  }, [refreshSkills, t]);

  // ESC closes (only in standalone modal mode)
  useEffect(() => {
    if (!open || embedded) {
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
  }, [open, busy, onClose, embedded]);

  if (!open && !embedded) {
    return null;
  }

  const filteredTools = tools.filter((tool) => {
    const q = allowedToolsFilter.trim().toLowerCase();
    if (q.length === 0) {
      return true;
    }
    return (
      tool.name.toLowerCase().includes(q) || tool.description.toLowerCase().includes(q)
    );
  });

  const isFormMode = mode === "edit" || mode === "create";
  const formDisabled = busy || (mode !== "create" && mode !== "edit");
  const idFieldLocked = mode === "edit"; // id is the primary key — lock once persisted
  const cantWrite = !workspaceReady || !tauriRuntime;

  const innerContent = (
    <>
      <aside className="skill-mgmt__list" aria-label={t("skillMgmt.listAria")}>
            <div className="skill-mgmt__list-meta">
              <span className="skill-mgmt__count">
                {t("skillMgmt.totalCount", { n: skills.length })}
              </span>
            </div>
            {loading ? (
              <p className="skill-mgmt__list-empty">{t("skillMgmt.loading")}</p>
            ) : skills.length === 0 ? (
              <p className="skill-mgmt__list-empty">{t("skillMgmt.empty")}</p>
            ) : (
              <ul className="skill-mgmt__list-items" role="list">
                {skills.map((item) => {
                  const active = item.id === selectedId;
                  return (
                    <li
                      key={item.id}
                      className={`skill-mgmt__row${active ? " skill-mgmt__row--active" : ""}${
                        item.isBuiltin ? " skill-mgmt__row--builtin" : ""
                      }`}
                    >
                      <button
                        type="button"
                        className="skill-mgmt__row-main"
                        onClick={() => beginView(item)}
                        title={item.id}
                      >
                        <span className="skill-mgmt__row-headline">
                          <span className="skill-mgmt__row-name">{item.name}</span>
                          <span className="skill-mgmt__row-version">v{item.version}</span>
                        </span>
                        <span className="skill-mgmt__row-id">{item.id}</span>
                        <span className="skill-mgmt__row-desc">{item.description || "—"}</span>
                        <span className="skill-mgmt__row-meta">
                          <span
                            className={`skill-mgmt__badge${
                              item.isBuiltin
                                ? " skill-mgmt__badge--builtin"
                                : " skill-mgmt__badge--custom"
                            }`}
                          >
                            {item.isBuiltin ? t("skillMgmt.builtin") : t("skillMgmt.custom")}
                          </span>
                          <span className="skill-mgmt__entry">{item.uiEntry}</span>
                        </span>
                      </button>
                      <div className="skill-mgmt__row-actions">
                        <button
                          type="button"
                          className="skill-mgmt__row-action"
                          disabled={item.isBuiltin || busy || cantWrite}
                          onClick={() => beginEdit(item)}
                          title={
                            item.isBuiltin ? t("skillMgmt.builtinReadOnly") : t("skillMgmt.edit")
                          }
                        >
                          {t("skillMgmt.edit")}
                        </button>
                        <button
                          type="button"
                          className="skill-mgmt__row-action skill-mgmt__row-action--danger"
                          disabled={item.isBuiltin || busy || cantWrite}
                          onClick={() => void requestDelete(item)}
                          title={
                            item.isBuiltin ? t("skillMgmt.builtinReadOnly") : t("skillMgmt.delete")
                          }
                        >
                          {t("skillMgmt.delete")}
                        </button>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
      </aside>

      <section className="skill-mgmt__detail" aria-label={t("skillMgmt.detailAria")}>
            {error ? (
              <p className="ai-settings__banner ai-settings__banner--error" role="alert">
                {error}
              </p>
            ) : null}
            {info ? (
              <p className="skill-mgmt__banner skill-mgmt__banner--info">{info}</p>
            ) : null}
            {cantWrite ? (
              <p className="app-modal__hint skill-mgmt__hint-warning">
                {t("skillMgmt.workspaceWarning")}
              </p>
            ) : null}

            {mode === "idle" ? (
              <div className="skill-mgmt__placeholder">
                <h3 className="skill-mgmt__placeholder-title">
                  {t("skillMgmt.placeholderTitle")}
                </h3>
                <p className="app-modal__hint">{t("skillMgmt.placeholderHint")}</p>
              </div>
            ) : (
              <SkillForm
                draft={draft}
                setDraft={setDraft}
                tagsRaw={tagsRaw}
                setTagsRaw={setTagsRaw}
                tools={tools}
                filteredTools={filteredTools}
                allowedToolsFilter={allowedToolsFilter}
                setAllowedToolsFilter={setAllowedToolsFilter}
                disabled={formDisabled}
                idLocked={idFieldLocked || mode === "view"}
                viewOnly={mode === "view"}
                isBuiltin={!!selectedItem?.isBuiltin && mode === "view"}
                t={t}
              />
            )}

            {isFormMode ? (
              <div className="app-modal__actions skill-mgmt__actions">
                <button
                  type="button"
                  className="app-modal__btn"
                  disabled={busy}
                  onClick={cancelEdit}
                >
                  {t("skillMgmt.cancel")}
                </button>
                <button
                  type="button"
                  className="app-modal__btn app-modal__btn--primary"
                  disabled={busy || cantWrite}
                  onClick={() => void submitDraft()}
                >
                  {busy
                    ? t("skillMgmt.saving")
                    : mode === "create"
                      ? t("skillMgmt.create")
                      : t("skillMgmt.save")}
                </button>
              </div>
            ) : mode === "view" && selectedItem && !selectedItem.isBuiltin ? (
              <div className="app-modal__actions skill-mgmt__actions">
                <button
                  type="button"
                  className="app-modal__btn app-modal__btn--primary"
                  disabled={busy || cantWrite}
                  onClick={() => beginEdit(selectedItem)}
                >
                  {t("skillMgmt.edit")}
                </button>
              </div>
            ) : null}
      </section>
    </>
  );

  if (embedded) {
    return (
      <div className="skill-mgmt__embedded">
        <header className="skill-mgmt__header skill-mgmt__header--embedded">
          <div className="skill-mgmt__title-block">
            <span className="skill-mgmt__eyebrow">{t("skillMgmt.eyebrow")}</span>
            <h2 id="skill-mgmt-title" className="settings-modal__title skill-mgmt__title">
              {t("skillMgmt.title")}
            </h2>
          </div>
          <div className="skill-mgmt__header-actions">
            <button
              type="button"
              className="app-modal__btn skill-mgmt__btn-ghost"
              disabled={busy || cantWrite}
              onClick={() => void handleReload()}
              title={t("skillMgmt.reloadTitle")}
            >
              <IconReload />
              <span>{t("skillMgmt.reload")}</span>
            </button>
            <button
              type="button"
              className="app-modal__btn app-modal__btn--primary skill-mgmt__btn-primary"
              disabled={busy || cantWrite}
              onClick={beginCreate}
            >
              <IconPlus />
              <span>{t("skillMgmt.new")}</span>
            </button>
          </div>
        </header>
        <div className="skill-mgmt__body">
          {innerContent}
        </div>
        {editorOpen ? (
          <SkillEditorModal
            open={editorOpen}
            mode={editorMode}
            initialManifest={editorInitial}
            tools={tools}
            workspaceReady={workspaceReady && tauriRuntime}
            onSave={handleEditorSave}
            onClose={() => setEditorOpen(false)}
          />
        ) : null}
      </div>
    );
  }

  return (
    <div
      className="app-modal-scrim skill-mgmt__scrim"
      role="presentation"
      onClick={() => {
        if (!busy) {
          onClose();
        }
      }}
    >
      <div
        className="app-modal app-modal--settings skill-mgmt"
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-mgmt-title"
        {...dragExcludeProps}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings-modal__header skill-mgmt__header">
          <div className="skill-mgmt__title-block">
            <span className="skill-mgmt__eyebrow">{t("skillMgmt.eyebrow")}</span>
            <h2 id="skill-mgmt-title" className="settings-modal__title skill-mgmt__title">
              {t("skillMgmt.title")}
            </h2>
          </div>
          <div className="skill-mgmt__header-actions">
            <button
              type="button"
              className="app-modal__btn skill-mgmt__btn-ghost"
              disabled={busy || cantWrite}
              onClick={() => void handleReload()}
              title={t("skillMgmt.reloadTitle")}
            >
              <IconReload />
              <span>{t("skillMgmt.reload")}</span>
            </button>
            <button
              type="button"
              className="app-modal__btn app-modal__btn--primary skill-mgmt__btn-primary"
              disabled={busy || cantWrite}
              onClick={beginCreate}
            >
              <IconPlus />
              <span>{t("skillMgmt.new")}</span>
            </button>
            <button
              type="button"
              className="settings-modal__close"
              aria-label={t("skillMgmt.close")}
              title={t("skillMgmt.close")}
              disabled={busy}
              {...dragExcludeProps}
              onClick={() => onClose()}
            >
              <IconClose />
            </button>
          </div>
        </header>

        <div className="settings-modal__body skill-mgmt__body">
          {innerContent}
        </div>
      </div>
    </div>
  );
}

type SkillFormProps = {
  draft: SkillManifestJson;
  setDraft: React.Dispatch<React.SetStateAction<SkillManifestJson>>;
  tagsRaw: string;
  setTagsRaw: React.Dispatch<React.SetStateAction<string>>;
  tools: ToolSummary[];
  filteredTools: ToolSummary[];
  allowedToolsFilter: string;
  setAllowedToolsFilter: React.Dispatch<React.SetStateAction<string>>;
  disabled: boolean;
  idLocked: boolean;
  viewOnly: boolean;
  isBuiltin: boolean;
  t: ReturnType<typeof useTranslation>["t"];
};

function SkillForm(props: SkillFormProps) {
  const {
    draft,
    setDraft,
    tagsRaw,
    setTagsRaw,
    tools,
    filteredTools,
    allowedToolsFilter,
    setAllowedToolsFilter,
    disabled,
    idLocked,
    viewOnly,
    isBuiltin,
    t,
  } = props;

  const toggleTool = useCallback(
    (name: string) => {
      setDraft((d) => {
        const has = d.allowedTools.includes(name);
        return {
          ...d,
          allowedTools: has
            ? d.allowedTools.filter((n) => n !== name)
            : [...d.allowedTools, name],
        };
      });
    },
    [setDraft],
  );

  const allowedSet = useMemo(() => new Set(draft.allowedTools), [draft.allowedTools]);
  const orphanAllowed = draft.allowedTools.filter(
    (n) => !tools.some((tool) => tool.name === n),
  );

  return (
    <fieldset className="skill-mgmt__form" disabled={viewOnly}>
      {isBuiltin ? (
        <p className="app-modal__hint skill-mgmt__hint-builtin">
          {t("skillMgmt.builtinHint")}
        </p>
      ) : null}

      <div className="skill-mgmt__form-grid">
        <label className="ai-settings__label" htmlFor="skill-mgmt-id">
          {t("skillMgmt.fieldId")}
        </label>
        <input
          id="skill-mgmt-id"
          className="app-modal__field"
          value={draft.id}
          onChange={(e) => setDraft((d) => ({ ...d, id: e.target.value.trim() }))}
          placeholder="my_skill"
          autoComplete="off"
          disabled={disabled || idLocked}
        />
        <p className="ai-settings__hint">{t("skillMgmt.fieldIdHint")}</p>

        <label className="ai-settings__label" htmlFor="skill-mgmt-name">
          {t("skillMgmt.fieldName")}
        </label>
        <input
          id="skill-mgmt-name"
          className="app-modal__field"
          value={draft.name}
          onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          autoComplete="off"
          disabled={disabled}
        />

        <label className="ai-settings__label" htmlFor="skill-mgmt-version">
          {t("skillMgmt.fieldVersion")}
        </label>
        <input
          id="skill-mgmt-version"
          className="app-modal__field"
          value={draft.version}
          onChange={(e) => setDraft((d) => ({ ...d, version: e.target.value }))}
          autoComplete="off"
          disabled={disabled}
        />

        <label className="ai-settings__label" htmlFor="skill-mgmt-desc">
          {t("skillMgmt.fieldDescription")}
        </label>
        <textarea
          id="skill-mgmt-desc"
          className="app-modal__field skill-mgmt__textarea"
          rows={2}
          value={draft.description}
          onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
          disabled={disabled}
        />

        <label className="ai-settings__label" htmlFor="skill-mgmt-prompt">
          {t("skillMgmt.fieldPrompt")}
        </label>
        <textarea
          id="skill-mgmt-prompt"
          className="app-modal__field skill-mgmt__textarea skill-mgmt__textarea--prompt"
          rows={6}
          value={draft.systemPromptTemplate}
          onChange={(e) =>
            setDraft((d) => ({ ...d, systemPromptTemplate: e.target.value }))
          }
          disabled={disabled}
          placeholder="You are working in {{workspace_name}} at {{workspace_root}} …"
        />
        <p className="ai-settings__hint">{t("skillMgmt.fieldPromptHint")}</p>

        <label className="ai-settings__label" htmlFor="skill-mgmt-when">
          {t("skillMgmt.fieldWhenToUse")}
        </label>
        <textarea
          id="skill-mgmt-when"
          className="app-modal__field skill-mgmt__textarea"
          rows={2}
          value={draft.whenToUse ?? ""}
          onChange={(e) =>
            setDraft((d) => ({ ...d, whenToUse: e.target.value }))
          }
          disabled={disabled}
        />

        <div className="skill-mgmt__row-pair">
          <div>
            <label className="ai-settings__label" htmlFor="skill-mgmt-max">
              {t("skillMgmt.fieldMaxToolCalls")}
            </label>
            <input
              id="skill-mgmt-max"
              className="app-modal__field"
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
              disabled={disabled}
            />
          </div>
          <div>
            <label className="ai-settings__label" htmlFor="skill-mgmt-timeout">
              {t("skillMgmt.fieldTimeoutSecs")}
            </label>
            <input
              id="skill-mgmt-timeout"
              className="app-modal__field"
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
              disabled={disabled}
            />
          </div>
        </div>

        <div className="skill-mgmt__row-pair">
          <div>
            <label className="ai-settings__label" htmlFor="skill-mgmt-uientry">
              {t("skillMgmt.fieldUiEntry")}
            </label>
            <select
              id="skill-mgmt-uientry"
              className="app-modal__field"
              value={draft.uiEntry}
              onChange={(e) =>
                setDraft((d) => ({ ...d, uiEntry: e.target.value as SkillUiEntry }))
              }
              disabled={disabled}
            >
              {UI_ENTRY_OPTIONS.map((opt) => (
                <option key={opt} value={opt}>
                  {t(`skillMgmt.uiEntry.${opt}`)}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="ai-settings__label" htmlFor="skill-mgmt-tags">
              {t("skillMgmt.fieldTags")}
            </label>
            <input
              id="skill-mgmt-tags"
              className="app-modal__field"
              value={tagsRaw}
              onChange={(e) => setTagsRaw(e.target.value)}
              placeholder={t("skillMgmt.fieldTagsPlaceholder")}
              disabled={disabled}
            />
          </div>
        </div>

        <label className="ai-settings__check skill-mgmt__check">
          <input
            type="checkbox"
            checked={draft.autoInvocable ?? true}
            onChange={(e) =>
              setDraft((d) => ({ ...d, autoInvocable: e.target.checked }))
            }
            disabled={disabled}
          />
          <span>{t("skillMgmt.fieldAutoInvocable")}</span>
        </label>
        <p className="ai-settings__hint">{t("skillMgmt.fieldAutoInvocableHint")}</p>

        <label className="ai-settings__label">
          {t("skillMgmt.fieldAllowedTools", { n: draft.allowedTools.length })}
        </label>
        <input
          className="app-modal__field skill-mgmt__tool-filter"
          value={allowedToolsFilter}
          onChange={(e) => setAllowedToolsFilter(e.target.value)}
          placeholder={t("skillMgmt.toolsFilterPlaceholder")}
          disabled={disabled}
        />
        <div className="skill-mgmt__tool-list" role="group">
          {filteredTools.length === 0 ? (
            <p className="skill-mgmt__tool-empty">{t("skillMgmt.toolsEmpty")}</p>
          ) : (
            filteredTools.map((tool) => {
              const checked = allowedSet.has(tool.name);
              return (
                <label
                  key={tool.name}
                  className={`skill-mgmt__tool-item${checked ? " skill-mgmt__tool-item--on" : ""}`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => toggleTool(tool.name)}
                    disabled={disabled}
                  />
                  <span className="skill-mgmt__tool-name">{tool.name}</span>
                  {tool.description ? (
                    <span className="skill-mgmt__tool-desc">{tool.description}</span>
                  ) : null}
                </label>
              );
            })
          )}
        </div>
        {orphanAllowed.length > 0 ? (
          <p className="ai-settings__hint skill-mgmt__tool-orphan">
            {t("skillMgmt.toolsOrphan", { tools: orphanAllowed.join(", ") })}
          </p>
        ) : null}
      </div>
    </fieldset>
  );
}

export default SkillManagementPanel;
