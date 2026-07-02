import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import packageMeta from "../../package.json";
import i18n, { setAppLocale } from "../i18n";
import type { VaultConfigForUi, VaultConfigSavePatch } from "../types/vaultAiConfig";
import { dispatchVaultConfigUpdated } from "../utils/vaultConfigBroadcast";
import { SemanticIndexStatus } from "./SemanticIndexStatus";
import "./AiLlmSettingsModal.css";

const SkillManagementPanel = lazy(() => import("./SkillManagementPanel"));

/** 与 Tauri 可拖拽窗口配合：排除交互区（非桌面端传空对象） */
export type TauriDragRegionExcludeProps =
  | { readonly "data-tauri-drag-region-exclude": true }
  | Record<string, never>;

export type AiLlmSettingsModalProps = {
  open: boolean;
  onClose: () => void;
  workspaceReady: boolean;
  tauriRuntime: boolean;
  dragExcludeProps: TauriDragRegionExcludeProps;
};

type SettingsSection = "general" | "ai" | "skills";

/** 左侧「通用」分区：滑块调谐图标 */
function IconGeneralSettings() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <line x1="4" y1="21" x2="4" y2="14" />
      <line x1="4" y1="10" x2="4" y2="3" />
      <line x1="12" y1="21" x2="12" y2="12" />
      <line x1="12" y1="8" x2="12" y2="3" />
      <line x1="20" y1="21" x2="20" y2="16" />
      <line x1="20" y1="12" x2="20" y2="3" />
      <line x1="9" y1="14" x2="15" y2="14" />
      <line x1="8" y1="8" x2="16" y2="8" />
      <line x1="14" y1="16" x2="22" y2="16" />
    </svg>
  );
}

function IconRefreshModels() {
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
      <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
      <path d="M16 16h5v5" />
    </svg>
  );
}

/** 左侧「AI / LLM」专用分区：星芒图标（与侧栏齿轮入口区分） */
function IconAiLlmSection() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.582a.5.5 0 0 1 0 .962L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z" />
      <path d="M20 3v4" />
      <path d="M22 5h-4" />
      <path d="M4 17v2" />
      <path d="M5 18H3" />
    </svg>
  );
}

/** 左侧「技能」分区：扳手/工具图标 */
function IconSkillsSection() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={true}
    >
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
    </svg>
  );
}

// --- Provider form state ---

type ProviderFormState = {
  id: string;
  label: string;
  baseUrl: string;
  apiKey: string;
  apiKeyPresent: boolean;
  apiKeyChanged: boolean;
  defaultModel: string;
  organizationId: string;
  isRemote: boolean;
  models: string[];
  modelsBusy: boolean;
  modelsError: string | null;
};

type FormState = {
  providers: ProviderFormState[];
  activeProviderId: string;
  timeoutMs: string;
  maxContextTokens: string;
  temperature: string;
  topP: string;
  allowPrivateContentInLocalLlm: boolean;
  toolsEnabled: boolean;
  planningEnabled: boolean;
  planningApprovalEnabled: boolean;
  memoryEnabled: boolean;
  memoryReflectionMode: string;
  passiveHighlightEnabled: boolean;
  passiveHighlightConfidenceMin: string;
  writingCoachEnabled: boolean;
  writingCoachIdleSeconds: string;
  writingCoachDepthMinChars: string;
  writingCoachTermMinChars: string;
  writingCoachBubbleSeconds: string;
  writingCoachCooldownMinutes: string;
  independentReviewEnabled: boolean;
  challengeReviewDailyCapIndependent: string;
  challengeReviewDailyCapInline: string;
  semanticEnabled: boolean;
  semanticAutoIndex: boolean;
  semanticSearchWeight: string;
  searchProvider: string;
  searchSearxngBaseUrl: string;
  searchTavilyApiKey: string;
  searchAliyunEndpoint: string;
  searchAliyunApiKey: string;
};

const PROVIDER_TEMPLATES: ReadonlyArray<{ label: string; baseUrl: string; isRemote: boolean }> = [
  { label: "OpenAI", baseUrl: "https://api.openai.com/v1", isRemote: true },
  { label: "DeepSeek", baseUrl: "https://api.deepseek.com", isRemote: true },
  { label: "SiliconFlow", baseUrl: "https://api.siliconflow.cn/v1", isRemote: true },
  { label: "Moonshot", baseUrl: "https://api.moonshot.cn/v1", isRemote: true },
  { label: "Ollama", baseUrl: "http://127.0.0.1:11434/v1", isRemote: false },
];

/** 空串按缺省处理，避免 parseInt/parseFloat('') 为 NaN 导致保存校验误判 */
function parseIntWithEmptyDefault(raw: string, whenEmpty: number): number {
  const t = raw.trim();
  if (t.length === 0) {
    return whenEmpty;
  }
  return Number.parseInt(t, 10);
}

function parseFloatWithEmptyDefault(raw: string, whenEmpty: number): number {
  const t = raw.trim();
  if (t.length === 0) {
    return whenEmpty;
  }
  return Number.parseFloat(t);
}

/** 空串用缺省后校验 [min,max]；越界或 NaN 返回 null */
function parseIntInRange(opts: { raw: string; emptyDefault: number; min: number; max: number }): number | null {
  const n = parseIntWithEmptyDefault(opts.raw, opts.emptyDefault);
  if (!Number.isFinite(n) || n < opts.min || n > opts.max) {
    return null;
  }
  return n;
}

function parseFloatInRange(opts: { raw: string; emptyDefault: number; min: number; max: number }): number | null {
  const n = parseFloatWithEmptyDefault(opts.raw, opts.emptyDefault);
  if (!Number.isFinite(n) || n < opts.min || n > opts.max) {
    return null;
  }
  return n;
}

/** 写作教练五项数值；任一项非法则 ok: false */
function parseWritingCoachNumericFields(form: FormState):
  | {
      ok: true;
      wcIdle: number;
      wcDepth: number;
      wcTerm: number;
      wcBubble: number;
      wcCd: number;
    }
  | { ok: false } {
  const wcIdle = parseIntInRange({ raw: form.writingCoachIdleSeconds, emptyDefault: 15, min: 5, max: 600 });
  const wcDepth = parseIntInRange({ raw: form.writingCoachDepthMinChars, emptyDefault: 500, min: 10, max: 20000 });
  const wcTerm = parseIntInRange({ raw: form.writingCoachTermMinChars, emptyDefault: 36, min: 8, max: 2000 });
  const wcBubble = parseIntInRange({ raw: form.writingCoachBubbleSeconds, emptyDefault: 30, min: 5, max: 600 });
  const wcCd = parseIntInRange({ raw: form.writingCoachCooldownMinutes, emptyDefault: 15, min: 1, max: 1440 });
  if (wcIdle == null || wcDepth == null || wcTerm == null || wcBubble == null || wcCd == null) {
    return { ok: false };
  }
  return { ok: true, wcIdle, wcDepth, wcTerm, wcBubble, wcCd };
}

/** 回顾日 cap 两项 */
function parseChallengeReviewCaps(form: FormState): { ok: true; capInd: number; capInline: number } | { ok: false } {
  const capInd = parseIntInRange({ raw: form.challengeReviewDailyCapIndependent, emptyDefault: 3, min: 1, max: 20 });
  const capInline = parseIntInRange({ raw: form.challengeReviewDailyCapInline, emptyDefault: 2, min: 1, max: 20 });
  if (capInd == null || capInline == null) {
    return { ok: false };
  }
  return { ok: true, capInd, capInline };
}

function defaultForm(): FormState {
  return {
    providers: [],
    activeProviderId: "",
    timeoutMs: "120000",
    maxContextTokens: "",
    temperature: "0.7",
    topP: "",
    allowPrivateContentInLocalLlm: false,
    toolsEnabled: true,
    planningEnabled: false,
    planningApprovalEnabled: true,
    memoryEnabled: true,
    memoryReflectionMode: "confirm",
    passiveHighlightEnabled: true,
    passiveHighlightConfidenceMin: "0.55",
    writingCoachEnabled: true,
    writingCoachIdleSeconds: "15",
    writingCoachDepthMinChars: "500",
    writingCoachTermMinChars: "36",
    writingCoachBubbleSeconds: "30",
    writingCoachCooldownMinutes: "15",
    independentReviewEnabled: false,
    challengeReviewDailyCapIndependent: "3",
    challengeReviewDailyCapInline: "2",
    semanticEnabled: true,
    semanticAutoIndex: true,
    semanticSearchWeight: "0.6",
    searchProvider: "",
    searchSearxngBaseUrl: "",
    searchTavilyApiKey: "",
    searchAliyunEndpoint: "",
    searchAliyunApiKey: "",
  };
}

function providerFormEquals(a: ProviderFormState, b: ProviderFormState): boolean {
  return (
    a.id === b.id &&
    a.label === b.label &&
    a.baseUrl === b.baseUrl &&
    a.defaultModel === b.defaultModel &&
    a.organizationId === b.organizationId &&
    a.isRemote === b.isRemote &&
    !a.apiKeyChanged
  );
}

function aiFormEqualsPersisted(a: FormState, b: FormState): boolean {
  if (a.activeProviderId !== b.activeProviderId) return false;
  if (a.providers.length !== b.providers.length) return false;
  for (let i = 0; i < a.providers.length; i++) {
    if (!providerFormEquals(a.providers[i], b.providers[i])) return false;
  }
  const scalarKeys: ReadonlyArray<keyof Omit<FormState, "providers" | "activeProviderId">> = [
    "timeoutMs",
    "maxContextTokens",
    "temperature",
    "topP",
    "allowPrivateContentInLocalLlm",
    "toolsEnabled",
    "planningEnabled",
    "planningApprovalEnabled",
    "memoryEnabled",
    "memoryReflectionMode",
    "passiveHighlightEnabled",
    "passiveHighlightConfidenceMin",
    "writingCoachEnabled",
    "writingCoachIdleSeconds",
    "writingCoachDepthMinChars",
    "writingCoachTermMinChars",
    "writingCoachBubbleSeconds",
    "writingCoachCooldownMinutes",
    "independentReviewEnabled",
    "challengeReviewDailyCapIndependent",
    "challengeReviewDailyCapInline",
    "semanticEnabled",
    "semanticAutoIndex",
    "semanticSearchWeight",
    "searchProvider",
    "searchSearxngBaseUrl",
    "searchTavilyApiKey",
    "searchAliyunEndpoint",
    "searchAliyunApiKey",
  ];
  for (const k of scalarKeys) {
    if (a[k] !== b[k]) return false;
  }
  return true;
}

function vaultConfigToForm(cfg: VaultConfigForUi): FormState {
  const { ai, cognitive } = cfg;
  const semantic = cfg.semantic ?? {
    enabled: true,
    autoIndexOnSave: true,
    searchWeight: 0.6,
  };

  const providers: ProviderFormState[] = ai.providers.map((p) => ({
    id: p.id,
    label: p.label,
    baseUrl: p.baseUrl,
    apiKey: "",
    apiKeyPresent: p.apiKeyPresent,
    apiKeyChanged: false,
    defaultModel: p.defaultModel,
    organizationId: p.organizationId ?? "",
    isRemote: p.isRemote,
    models: [],
    modelsBusy: false,
    modelsError: null,
  }));

  return {
    providers,
    activeProviderId: ai.activeProviderId,
    timeoutMs: String(ai.request.timeoutMs),
    maxContextTokens:
      ai.request.maxContextTokens != null ? String(ai.request.maxContextTokens) : "",
    temperature: String(ai.parameters.temperature),
    topP: ai.parameters.topP != null ? String(ai.parameters.topP) : "",
    allowPrivateContentInLocalLlm: ai.privacy.allowPrivateContentInLocalLlm,
    toolsEnabled: ai.toolsEnabled !== false,
    planningEnabled: ai.planningEnabled === true,
    planningApprovalEnabled: ai.planningApprovalEnabled !== false,
    memoryEnabled: ai.memoryEnabled !== false,
    memoryReflectionMode: ai.memoryReflectionMode ?? "confirm",
    passiveHighlightEnabled: cognitive.passiveHighlightEnabled !== false,
    passiveHighlightConfidenceMin: String(cognitive.passiveHighlightConfidenceMin ?? 0.55),
    writingCoachEnabled: cognitive.writingCoachEnabled !== false,
    writingCoachIdleSeconds: String(cognitive.writingCoachIdleSeconds ?? 15),
    writingCoachDepthMinChars: String(cognitive.writingCoachDepthMinChars ?? 500),
    writingCoachTermMinChars: String(cognitive.writingCoachTermMinChars ?? 36),
    writingCoachBubbleSeconds: String(cognitive.writingCoachBubbleSeconds ?? 30),
    writingCoachCooldownMinutes: String(cognitive.writingCoachCooldownMinutes ?? 15),
    independentReviewEnabled: cognitive.independentReviewEnabled === true,
    challengeReviewDailyCapIndependent: String(cognitive.challengeReviewDailyCapIndependent ?? 3),
    challengeReviewDailyCapInline: String(cognitive.challengeReviewDailyCapInline ?? 2),
    semanticEnabled: semantic.enabled !== false,
    semanticAutoIndex: semantic.autoIndexOnSave !== false,
    semanticSearchWeight: String(semantic.searchWeight ?? 0.6),
    searchProvider: cfg.search?.provider ?? "",
    searchSearxngBaseUrl: cfg.search?.searxng?.baseUrl ?? "",
    searchTavilyApiKey: cfg.search?.tavily?.apiKey ?? "",
    searchAliyunEndpoint: cfg.search?.aliyunOpensearch?.endpoint ?? "",
    searchAliyunApiKey: cfg.search?.aliyunOpensearch?.apiKey ?? "",
  };
}

export function AiLlmSettingsModal({
  open,
  onClose,
  workspaceReady,
  tauriRuntime,
  dragExcludeProps,
}: AiLlmSettingsModalProps) {
  const { t } = useTranslation();
  const [form, setForm] = useState<FormState>(defaultForm);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [showTemplateMenu, setShowTemplateMenu] = useState(false);
  const [activeSection, setActiveSection] = useState<SettingsSection>("ai");
  const [appVersion, setAppVersion] = useState<string | null>(null);
  const [versionLoading, setVersionLoading] = useState(false);
  const [versionError, setVersionError] = useState<string | null>(null);

  const savedAiFormRef = useRef<FormState | null>(null);

  const disposedRef = useRef(false);
  useEffect(() => {
    disposedRef.current = false;
    return () => {
      disposedRef.current = true;
    };
  }, []);

  const reloadConfig = useCallback(async () => {
    if (!isTauri() || !workspaceReady) {
      return;
    }
    setLoading(true);
    savedAiFormRef.current = null;
    setLoadError(null);
    try {
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      if (disposedRef.current) {
        return;
      }
      const next = vaultConfigToForm(cfg);
      setForm(next);
      savedAiFormRef.current = next;
      setSelectedProviderId(cfg.ai.activeProviderId);
    } catch (e) {
      if (!disposedRef.current) {
        setLoadError(e instanceof Error ? e.message : String(e));
        const d = defaultForm();
        setForm(d);
        savedAiFormRef.current = d;
      }
    } finally {
      if (!disposedRef.current) {
        setLoading(false);
      }
    }
  }, [workspaceReady]);

  useEffect(() => {
    if (!open || !tauriRuntime || !workspaceReady) {
      return;
    }
    void reloadConfig();
  }, [open, tauriRuntime, workspaceReady, reloadConfig]);

  useEffect(() => {
    if (open) {
      setActiveSection("ai");
    }
  }, [open]);

  useEffect(() => {
    if (!open) {
      savedAiFormRef.current = null;
      return;
    }
    if (!tauriRuntime || !workspaceReady) {
      const d = defaultForm();
      setForm(d);
      savedAiFormRef.current = d;
    }
  }, [open, tauriRuntime, workspaceReady]);

  useEffect(() => {
    if (!open) {
      return;
    }
    let cancelled = false;
    if (isTauri() && tauriRuntime) {
      setVersionLoading(true);
      setVersionError(null);
      void (async () => {
        try {
          const v = await getVersion();
          if (!cancelled) {
            setAppVersion(v);
            setVersionError(null);
          }
        } catch (e) {
          if (!cancelled) {
            setAppVersion(packageMeta.version);
            setVersionError(
              e instanceof Error ? e.message : t("settings.versionFallback"),
            );
          }
        } finally {
          if (!cancelled) {
            setVersionLoading(false);
          }
        }
      })();
    } else {
      setVersionLoading(false);
      setVersionError(null);
      setAppVersion(packageMeta.version);
    }
    return () => {
      cancelled = true;
    };
  }, [open, tauriRuntime, t]);

  const isAiSettingsDirty = useCallback((): boolean => {
    if (loading) {
      return false;
    }
    const saved = savedAiFormRef.current;
    if (saved === null) {
      return !aiFormEqualsPersisted(form, defaultForm());
    }
    return !aiFormEqualsPersisted(form, saved);
  }, [form, loading]);

  const requestClose = useCallback(async () => {
    if (saving) {
      return;
    }
    if (!isAiSettingsDirty()) {
      onClose();
      return;
    }
    const message = t("dialogs.unsavedAiSettings");
    const discard = isTauri()
      ? await ask(message, { title: t("dialogs.unsavedChanges"), kind: "warning" })
      : window.confirm(message);
    if (discard) {
      onClose();
    }
  }, [saving, onClose, isAiSettingsDirty, t]);

  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        void requestClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, requestClose]);

  // --- Provider helpers ---

  const selectedProvider = useMemo(
    () => form.providers.find((p) => p.id === selectedProviderId),
    [form.providers, selectedProviderId],
  );

  const selectedModelOptions = useMemo(() => {
    if (!selectedProvider) return [];
    const cur = selectedProvider.defaultModel.trim();
    const seen = new Set<string>();
    const out: string[] = [];
    if (cur && !selectedProvider.models.includes(cur)) {
      out.push(cur);
      seen.add(cur);
    }
    for (const m of selectedProvider.models) {
      if (!seen.has(m)) {
        out.push(m);
        seen.add(m);
      }
    }
    return out;
  }, [selectedProvider]);

  const updateSelectedProvider = useCallback(
    (updates: Partial<ProviderFormState>) => {
      setForm((f) => ({
        ...f,
        providers: f.providers.map((p) =>
          p.id === selectedProviderId ? { ...p, ...updates } : p,
        ),
      }));
    },
    [selectedProviderId],
  );

  const addProviderFromTemplate = useCallback(
    (template: { label: string; baseUrl: string; isRemote: boolean }) => {
      const id = crypto.randomUUID();
      const newProvider: ProviderFormState = {
        id,
        label: template.label,
        baseUrl: template.baseUrl,
        apiKey: "",
        apiKeyPresent: false,
        apiKeyChanged: false,
        defaultModel: "",
        organizationId: "",
        isRemote: template.isRemote,
        models: [],
        modelsBusy: false,
        modelsError: null,
      };
      setForm((f) => ({
        ...f,
        providers: [...f.providers, newProvider],
        activeProviderId: f.providers.length === 0 ? id : f.activeProviderId,
      }));
      setSelectedProviderId(id);
      setShowTemplateMenu(false);
    },
    [],
  );

  const deleteProvider = useCallback((id: string) => {
    setForm((f) => {
      const filtered = f.providers.filter((p) => p.id !== id);
      if (filtered.length === 0) return f;
      const newActive = f.activeProviderId === id ? filtered[0].id : f.activeProviderId;
      return { ...f, providers: filtered, activeProviderId: newActive };
    });
  }, []);

  useEffect(() => {
    if (form.providers.length > 0 && !form.providers.some((p) => p.id === selectedProviderId)) {
      setSelectedProviderId(form.activeProviderId || form.providers[0].id);
    }
  }, [form.providers, form.activeProviderId, selectedProviderId]);

  const refreshModels = useCallback(
    async (providerId: string) => {
      if (!isTauri() || !workspaceReady) return;

      const provider = form.providers.find((p) => p.id === providerId);
      if (!provider) return;

      setForm((f) => ({
        ...f,
        providers: f.providers.map((p) =>
          p.id === providerId ? { ...p, modelsBusy: true, modelsError: null } : p,
        ),
      }));

      try {
        const base = provider.baseUrl.trim();
        const key = provider.apiKey.trim();
        const list = await invoke<string[]>("list_models", {
          args: {
            providerId,
            baseUrl: base.length > 0 ? base : undefined,
            apiKey: key.length > 0 ? key : undefined,
          },
        });
        if (!disposedRef.current) {
          setForm((f) => ({
            ...f,
            providers: f.providers.map((p) =>
              p.id === providerId ? { ...p, models: list, modelsBusy: false } : p,
            ),
          }));
        }
      } catch (e) {
        if (!disposedRef.current) {
          setForm((f) => ({
            ...f,
            providers: f.providers.map((p) =>
              p.id === providerId
                ? { ...p, models: [], modelsBusy: false, modelsError: e instanceof Error ? e.message : String(e) }
                : p,
            ),
          }));
        }
      }
    },
    [workspaceReady, form.providers],
  );

  const handleSave = useCallback(async () => {
    setSaveError(null);
    if (!isTauri() || !workspaceReady) {
      setSaveError(t("settings.saveNeedWorkspace"));
      return;
    }

    const timeoutMs = parseIntInRange({ raw: form.timeoutMs, emptyDefault: 120_000, min: 1_000, max: 600_000 });
    if (timeoutMs == null) {
      setSaveError(t("settings.errTimeout"));
      return;
    }

    const temperature = parseFloatInRange({ raw: form.temperature, emptyDefault: 0.7, min: 0, max: 2 });
    if (temperature == null) {
      setSaveError(t("settings.errTemperature"));
      return;
    }

    let topP: number | null = null;
    const topRaw = form.topP.trim();
    if (topRaw.length > 0) {
      const tp = Number.parseFloat(topRaw);
      if (!Number.isFinite(tp) || tp < 0 || tp > 1) {
        setSaveError(t("settings.errTopP"));
        return;
      }
      topP = tp;
    }

    let maxContextTokens: number | null = null;
    const maxRaw = form.maxContextTokens.trim();
    if (maxRaw.length > 0) {
      const m = Number.parseInt(maxRaw, 10);
      if (!Number.isFinite(m) || m < 1) {
        setSaveError(t("settings.errMaxCtx"));
        return;
      }
      maxContextTokens = m;
    }

    const phMin = parseFloatInRange({ raw: form.passiveHighlightConfidenceMin, emptyDefault: 0.55, min: 0, max: 1 });
    if (phMin == null) {
      setSaveError(t("settings.errPassiveConfidence"));
      return;
    }

    const wc = parseWritingCoachNumericFields(form);
    if (!wc.ok) {
      setSaveError(t("settings.errWritingCoachParams"));
      return;
    }
    const { wcIdle, wcDepth, wcTerm, wcBubble, wcCd } = wc;

    const caps = parseChallengeReviewCaps(form);
    if (!caps.ok) {
      setSaveError(t("settings.errChallengeReviewCaps"));
      return;
    }
    const { capInd, capInline } = caps;

    const semW = parseFloatInRange({
      raw: form.semanticSearchWeight,
      emptyDefault: 0.6,
      min: 0,
      max: 1,
    });
    if (semW == null) {
      setSaveError(t("settings.semanticErrWeight"));
      return;
    }

    const providerPatches = form.providers.map((p) => ({
      id: p.id,
      label: p.label,
      baseUrl: p.baseUrl.trim(),
      defaultModel: p.defaultModel.trim(),
      organizationId: p.organizationId.trim() || null,
      lastUsedModel: p.defaultModel.trim() || null,
      isRemote: p.isRemote,
      ...(p.apiKeyChanged && p.apiKey.trim() ? { apiKey: p.apiKey.trim() } : {}),
    }));

    const patch: VaultConfigSavePatch = {
      ai: {
        activeProviderId: form.activeProviderId,
        providers: providerPatches,
        request: {
          timeoutMs,
          maxContextTokens,
        },
        parameters: {
          temperature,
          topP,
        },
        privacy: {
          allowPrivateContentInLocalLlm: form.allowPrivateContentInLocalLlm,
        },
        toolsEnabled: form.toolsEnabled,
        planningEnabled: form.planningEnabled,
        planningApprovalEnabled: form.planningApprovalEnabled,
        memoryEnabled: form.memoryEnabled,
        memoryReflectionMode: form.memoryReflectionMode,
      },
      cognitive: {
        passiveHighlightEnabled: form.passiveHighlightEnabled,
        passiveHighlightConfidenceMin: phMin,
        writingCoachEnabled: form.writingCoachEnabled,
        writingCoachIdleSeconds: wcIdle,
        writingCoachDepthMinChars: wcDepth,
        writingCoachTermMinChars: wcTerm,
        writingCoachBubbleSeconds: wcBubble,
        writingCoachCooldownMinutes: wcCd,
        independentReviewEnabled: form.independentReviewEnabled,
        challengeReviewDailyCapIndependent: capInd,
        challengeReviewDailyCapInline: capInline,
      },
      semantic: {
        enabled: form.semanticEnabled,
        autoIndexOnSave: form.semanticAutoIndex,
        searchWeight: semW,
      },
      search: form.searchProvider
        ? {
            provider: form.searchProvider as "searxng" | "tavily" | "aliyun-opensearch",
            ...(form.searchProvider === "searxng" && form.searchSearxngBaseUrl.trim()
              ? { searxng: { baseUrl: form.searchSearxngBaseUrl.trim() } }
              : {}),
            ...(form.searchProvider === "tavily" && form.searchTavilyApiKey.trim()
              ? { tavily: { apiKey: form.searchTavilyApiKey.trim() } }
              : {}),
            ...(form.searchProvider === "aliyun-opensearch" &&
            form.searchAliyunEndpoint.trim() &&
            form.searchAliyunApiKey.trim()
              ? {
                  aliyunOpensearch: {
                    endpoint: form.searchAliyunEndpoint.trim(),
                    apiKey: form.searchAliyunApiKey.trim(),
                  },
                }
              : {}),
          }
        : { provider: null },
    };

    setSaving(true);
    try {
      await invoke("save_vault_config_patch", { patch });
      dispatchVaultConfigUpdated();
      if (!disposedRef.current) {
        onClose();
      }
    } catch (e) {
      if (!disposedRef.current) {
        setSaveError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (!disposedRef.current) {
        setSaving(false);
      }
    }
  }, [form, workspaceReady, onClose, t]);

  if (!open) {
    return null;
  }

  const sp = selectedProvider;

  const scrim = (
    <div
      className="app-modal-scrim"
      role="presentation"
      onClick={() => {
        if (!saving) {
          void requestClose();
        }
      }}
    >
      <div
        className="app-modal app-modal--settings settings-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
        {...dragExcludeProps}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings-modal__header">
          <h2 id="settings-modal-title" className="settings-modal__title">
            {t("settings.title")}
          </h2>
          <button
            type="button"
            className="settings-modal__close"
            aria-label={t("settings.closeSettings")}
            title={t("toolbar.close")}
            disabled={saving}
            {...dragExcludeProps}
            onClick={() => void requestClose()}
          >
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
          </button>
        </header>

        <div className="settings-modal__body">
          <nav className="settings-modal__tools" aria-label={t("settings.sectionsNav")}>
            <button
              type="button"
              className={`settings-modal__tool${activeSection === "general" ? " settings-modal__tool--active" : ""}`}
              aria-pressed={activeSection === "general"}
              onClick={() => setActiveSection("general")}
            >
              <span className="settings-modal__tool-icon" aria-hidden={true}>
                <IconGeneralSettings />
              </span>
              <span className="settings-modal__tool-label">{t("settings.general")}</span>
            </button>
            <button
              type="button"
              className={`settings-modal__tool${activeSection === "ai" ? " settings-modal__tool--active" : ""}`}
              aria-pressed={activeSection === "ai"}
              onClick={() => setActiveSection("ai")}
            >
              <span className="settings-modal__tool-icon" aria-hidden={true}>
                <IconAiLlmSection />
              </span>
              <span className="settings-modal__tool-label">{t("settings.aiLlm")}</span>
            </button>
            <button
              type="button"
              className={`settings-modal__tool${activeSection === "skills" ? " settings-modal__tool--active" : ""}`}
              aria-pressed={activeSection === "skills"}
              onClick={() => setActiveSection("skills")}
            >
              <span className="settings-modal__tool-icon" aria-hidden={true}>
                <IconSkillsSection />
              </span>
              <span className="settings-modal__tool-label">{t("settings.skills")}</span>
            </button>
          </nav>

          {/* 外层固定高度由 .app-modal--settings 控制；此处唯一滚动区适配 General/AI */}
          <div className="settings-modal__content">
            <div
              className="settings-modal__scroll"
              role="tabpanel"
              aria-labelledby={
                activeSection === "general"
                  ? "settings-general-heading"
                  : activeSection === "ai"
                    ? "ai-llm-settings-heading"
                    : "skill-mgmt-title"
              }
              id={`settings-tabpanel-${activeSection}`}
            >
              <div className="settings-modal__scroll-inner">
                {activeSection === "general" ? (
                  <div className="settings-modal__placeholder">
                    <h3 className="settings-modal__panel-title" id="settings-general-heading">
                      {t("settings.generalHeading")}
                    </h3>
                    <p className="app-modal__hint settings-general__intro">
                      {t("settings.versionIntro")}
                    </p>
                    <p className="app-modal__hint settings-general__intro">
                      {t("settings.backupKnowforgeHint")}
                    </p>
                    {versionLoading ? (
                      <p className="ai-settings__loading">{t("settings.loading")}</p>
                    ) : null}
                    {versionError ? (
                      <p className="ai-settings__banner ai-settings__banner--error" role="alert">
                        {versionError}
                      </p>
                    ) : null}
                    <dl className="settings-general__about">
                      {!versionLoading && appVersion != null ? (
                        <div className="settings-general__row">
                          <dt className="settings-general__label">{t("settings.versionLabel")}</dt>
                          <dd className="settings-general__value settings-general__value--mono">
                            {appVersion}
                          </dd>
                        </div>
                      ) : null}
                      <div className="settings-general__row">
                        <dt className="settings-general__label">{t("settings.language")}</dt>
                        <dd className="settings-general__value">
                          <div className="settings-lang-toggle">
                            <button
                              type="button"
                              className={`app-modal__btn settings-lang-toggle__btn${i18n.language === "en" ? " settings-lang-toggle__btn--active" : ""}`}
                              onClick={() => setAppLocale("en")}
                            >
                              {t("settings.languageEn")}
                            </button>
                            <button
                              type="button"
                              className={`app-modal__btn settings-lang-toggle__btn${i18n.language === "zh" ? " settings-lang-toggle__btn--active" : ""}`}
                              onClick={() => setAppLocale("zh")}
                            >
                              {t("settings.languageZh")}
                            </button>
                          </div>
                        </dd>
                      </div>
                    </dl>
                  </div>
                ) : activeSection === "ai" ? (
                  <div className="settings-modal__ai">
                    <h3 className="settings-modal__panel-title" id="ai-llm-settings-heading">
                      {t("settings.aiHeading")}
                    </h3>
                    <p className="app-modal__hint ai-settings__path-hint">
                      {t("settings.storedHintBefore")}{" "}
                      <code className="ai-settings__code">.knowforge/config.json</code>{" "}
                      {t("settings.storedHintAfter")}
                    </p>
                    <p className="app-modal__hint ai-settings__sections-intro">
                      {t("settings.aiSectionsIntro")}
                    </p>

                    {loadError ? (
                      <p className="ai-settings__banner ai-settings__banner--error">{loadError}</p>
                    ) : null}
                    {saveError ? (
                      <p className="ai-settings__banner ai-settings__banner--error">{saveError}</p>
                    ) : null}

                    {loading ? (
                      <p className="ai-settings__loading">{t("settings.loading")}</p>
                    ) : (
                      <>
            <div className="ai-settings__section-stack">

            {/* ── Group: Model Providers ── */}
            <details className="ai-settings__group" open>
              <summary className="ai-settings__group-summary">{t("settings.providersGroup")}</summary>
              <div className="ai-settings__group-body">

            {/* Provider tabs */}
            <div className="ai-settings__provider-tabs">
              {form.providers.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className={`app-modal__btn ai-settings__provider-tab${
                    p.id === selectedProviderId ? " ai-settings__provider-tab--selected" : ""
                  }`}
                  onClick={() => setSelectedProviderId(p.id)}
                  title={p.id === form.activeProviderId ? t("settings.activeProvider") : undefined}
                >
                  {p.label || p.id}
                  {p.id === form.activeProviderId && (
                    <span className="ai-settings__active-badge">{" ★"}</span>
                  )}
                </button>
              ))}
              <div className="ai-settings__provider-add-wrap">
                <button
                  type="button"
                  className="app-modal__btn ai-settings__provider-add"
                  onClick={() => setShowTemplateMenu((v) => !v)}
                  title={t("settings.addProvider")}
                  disabled={!tauriRuntime || !workspaceReady}
                >
                  +
                </button>
                {showTemplateMenu && (
                  <div className="ai-settings__template-menu">
                    {PROVIDER_TEMPLATES.map((tmpl) => (
                      <button
                        key={tmpl.label}
                        type="button"
                        className="app-modal__btn ai-settings__template-item"
                        onClick={() => addProviderFromTemplate(tmpl)}
                      >
                        {tmpl.label}
                      </button>
                    ))}
                    <button
                      type="button"
                      className="app-modal__btn ai-settings__template-item"
                      onClick={() =>
                        addProviderFromTemplate({
                          label: t("settings.customProvider"),
                          baseUrl: "",
                          isRemote: true,
                        })
                      }
                    >
                      {t("settings.customProvider")}
                    </button>
                  </div>
                )}
              </div>
            </div>

            {/* Edit panel for selected provider */}
            {sp ? (
            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{sp.label || sp.id}</legend>

              <label className="ai-settings__label" htmlFor="ai-provider-label">
                {t("settings.providerLabel")}
              </label>
              <input
                id="ai-provider-label"
                className="app-modal__field"
                value={sp.label}
                onChange={(e) => updateSelectedProvider({ label: e.target.value })}
                autoComplete="off"
              />

              <label className="ai-settings__label" htmlFor="ai-provider-base-url">
                {t("settings.providerBaseUrl")}
              </label>
              <input
                id="ai-provider-base-url"
                className="app-modal__field"
                value={sp.baseUrl}
                onChange={(e) => updateSelectedProvider({ baseUrl: e.target.value })}
                autoComplete="off"
                placeholder="https://api.openai.com/v1"
              />

              <label className="ai-settings__label" htmlFor="ai-provider-api-key">
                {t("settings.openaiApiKey")}
              </label>
              <input
                id="ai-provider-api-key"
                className="app-modal__field"
                type="password"
                value={sp.apiKey}
                onChange={(e) =>
                  updateSelectedProvider({ apiKey: e.target.value, apiKeyChanged: true })
                }
                autoComplete="off"
                placeholder={
                  sp.apiKeyPresent
                    ? t("settings.openaiApiKeyPresent")
                    : t("settings.openaiApiKeyEmpty")
                }
              />

              <label className="ai-settings__label" htmlFor="ai-provider-model">
                {t("settings.defaultModel")}
              </label>
              <div className="ai-settings__row">
                <select
                  id="ai-provider-model"
                  className="app-modal__field ai-settings__field-grow ai-settings__model-select"
                  value={sp.defaultModel}
                  onChange={(e) => updateSelectedProvider({ defaultModel: e.target.value })}
                  autoComplete="off"
                >
                  <option value="">{t("settings.modelSelectPlaceholder")}</option>
                  {selectedModelOptions.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
                <button
                  type="button"
                  className={`app-modal__btn ai-settings__refresh-models${sp.modelsBusy ? " ai-settings__refresh-models--busy" : ""}`}
                  disabled={sp.modelsBusy || !workspaceReady}
                  aria-label={t("settings.refreshModels")}
                  title={sp.modelsBusy ? t("settings.loading") : t("settings.refreshModels")}
                  aria-busy={sp.modelsBusy}
                  onClick={() => void refreshModels(sp.id)}
                >
                  <IconRefreshModels />
                </button>
              </div>
              {sp.modelsError ? (
                <p className="ai-settings__inline-error" role="alert">
                  {sp.modelsError}
                </p>
              ) : null}

              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={sp.isRemote}
                  onChange={(e) => updateSelectedProvider({ isRemote: e.target.checked })}
                />
                {t("settings.isRemote")}
              </label>
              <p className="ai-settings__hint">{t("settings.isRemoteHint")}</p>

              <div className="ai-settings__provider-actions">
                {sp.id !== form.activeProviderId && (
                  <button
                    type="button"
                    className="app-modal__btn app-modal__btn--primary"
                    onClick={() => setForm((f) => ({ ...f, activeProviderId: sp.id }))}
                  >
                    {t("settings.setActiveProvider")}
                  </button>
                )}
                {form.providers.length > 1 && (
                  <button
                    type="button"
                    className="app-modal__btn app-modal__btn--danger"
                    onClick={() => deleteProvider(sp.id)}
                  >
                    {t("settings.deleteProvider")}
                  </button>
                )}
              </div>
            </fieldset>
            ) : (
              <p className="ai-settings__hint">{t("settings.noProviderConfigured")}</p>
            )}

              </div>
            </details>

            {/* ── Group: Core ── */}
            <details className="ai-settings__group" open>
              <summary className="ai-settings__group-summary">{t("settings.coreGroup")}</summary>
              <div className="ai-settings__group-body">

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.requestSampling")}</legend>
              <label className="ai-settings__label" htmlFor="ai-timeout">
                {t("settings.timeoutMs")}
              </label>
              <input
                id="ai-timeout"
                className="app-modal__field"
                value={form.timeoutMs}
                onChange={(e) => setForm((f) => ({ ...f, timeoutMs: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <label className="ai-settings__label" htmlFor="ai-max-ctx">
                {t("settings.maxContextOptional")}
              </label>
              <input
                id="ai-max-ctx"
                className="app-modal__field"
                value={form.maxContextTokens}
                onChange={(e) => setForm((f) => ({ ...f, maxContextTokens: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
                placeholder={t("settings.phEmptyUnset")}
              />
              <label className="ai-settings__label" htmlFor="ai-temp">
                {t("settings.temperature")}
              </label>
              <input
                id="ai-temp"
                className="app-modal__field"
                value={form.temperature}
                onChange={(e) => setForm((f) => ({ ...f, temperature: e.target.value }))}
                inputMode="decimal"
                autoComplete="off"
              />
              <label className="ai-settings__label" htmlFor="ai-top-p">
                {t("settings.topPOptional")}
              </label>
              <input
                id="ai-top-p"
                className="app-modal__field"
                value={form.topP}
                onChange={(e) => setForm((f) => ({ ...f, topP: e.target.value }))}
                inputMode="decimal"
                autoComplete="off"
                placeholder={t("settings.phEmptyUnset")}
              />
            </fieldset>

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.featuresSection")}</legend>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.allowPrivateContentInLocalLlm}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, allowPrivateContentInLocalLlm: e.target.checked }))
                  }
                />
                {t("settings.allowPrivateLocal")}
              </label>
              <label className="ai-settings__check" title={t("settings.toolsEnabledHint")}>
                <input
                  type="checkbox"
                  checked={form.toolsEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, toolsEnabled: e.target.checked }))
                  }
                />
                {t("settings.toolsEnabled")}
              </label>
              <label className="ai-settings__check" title={t("settings.planningEnabledHint")}>
                <input
                  type="checkbox"
                  checked={form.planningEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, planningEnabled: e.target.checked }))
                  }
                />
                {t("settings.planningEnabled")}
              </label>
              <p className="app-modal__hint">{t("settings.planningEnabledHint")}</p>
              {form.planningEnabled && (
                <>
                  <label
                    className="ai-settings__check"
                    title={t("settings.planningApprovalEnabledHint")}
                  >
                    <input
                      type="checkbox"
                      checked={form.planningApprovalEnabled}
                      onChange={(e) =>
                        setForm((f) => ({
                          ...f,
                          planningApprovalEnabled: e.target.checked,
                        }))
                      }
                    />
                    {t("settings.planningApprovalEnabled")}
                  </label>
                  <p className="app-modal__hint">
                    {t("settings.planningApprovalEnabledHint")}
                  </p>
                </>
              )}
              <label className="ai-settings__check" title={t("settings.memoryEnabledHint")}>
                <input
                  type="checkbox"
                  checked={form.memoryEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, memoryEnabled: e.target.checked }))
                  }
                />
                {t("settings.memoryEnabled")}
              </label>
              <p className="app-modal__hint">{t("settings.memoryEnabledHint")}</p>
              {form.memoryEnabled && (
                <div className="ai-settings__reflection-mode">
                  <p className="app-modal__hint">{t("settings.memoryReflectionMode")}</p>
                  <div className="settings-lang-toggle">
                    {(["confirm", "auto", "off"] as const).map((mode) => (
                      <button
                        key={mode}
                        type="button"
                        className={`app-modal__btn settings-lang-toggle__btn${
                          form.memoryReflectionMode === mode
                            ? " settings-lang-toggle__btn--active"
                            : ""
                        }`}
                        onClick={() =>
                          setForm((f) => ({ ...f, memoryReflectionMode: mode }))
                        }
                      >
                        {t(`settings.memoryReflection.${mode}`)}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              <button
                type="button"
                className="app-modal__btn app-modal__btn--danger"
                disabled={!tauriRuntime || !workspaceReady}
                onClick={async () => {
                  const confirmed = isTauri()
                    ? await ask(t("settings.clearMemoryConfirm"), {
                        title: t("settings.clearMemory"),
                        kind: "warning",
                      })
                    : window.confirm(t("settings.clearMemoryConfirm"));
                  if (!confirmed) return;
                  try {
                    await invoke("clear_agent_memory");
                  } catch (e) {
                    setSaveError(String(e));
                  }
                }}
              >
                {t("settings.clearMemory")}
              </button>
            </fieldset>

              </div>
            </details>

            {/* ── Group: Search & Indexing ── */}
            <details className="ai-settings__group">
              <summary className="ai-settings__group-summary">{t("settings.searchGroup")}</summary>
              <div className="ai-settings__group-body">

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.semanticSearchSection")}</legend>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.semanticEnabled}
                  onChange={(e) => setForm((f) => ({ ...f, semanticEnabled: e.target.checked }))}
                />
                {t("settings.semanticEnable")}
              </label>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.semanticAutoIndex}
                  onChange={(e) => setForm((f) => ({ ...f, semanticAutoIndex: e.target.checked }))}
                />
                {t("settings.semanticAutoIndex")}
              </label>
              <label className="ai-settings__label" htmlFor="ai-semantic-weight">
                {t("settings.semanticSearchWeight")}
              </label>
              <input
                id="ai-semantic-weight"
                className="app-modal__field"
                value={form.semanticSearchWeight}
                onChange={(e) => setForm((f) => ({ ...f, semanticSearchWeight: e.target.value }))}
                inputMode="decimal"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.semanticSearchWeightHint")}</p>
              <SemanticIndexStatus workspaceReady={workspaceReady} tauriRuntime={tauriRuntime} />
            </fieldset>

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.searchSection")}</legend>
              <label className="ai-settings__label" htmlFor="ai-search-provider">
                {t("settings.searchProvider")}
              </label>
              <select
                id="ai-search-provider"
                className="app-modal__field ai-settings__field-grow ai-settings__model-select"
                value={form.searchProvider}
                onChange={(e) => setForm((f) => ({ ...f, searchProvider: e.target.value }))}
              >
                <option value="">{t("settings.searchProviderNone")}</option>
                <option value="searxng">SearXNG</option>
                <option value="tavily">Tavily</option>
                <option value="aliyun-opensearch">Aliyun OpenSearch</option>
              </select>

              {form.searchProvider === "searxng" && (
                <>
                  <label className="ai-settings__label" htmlFor="ai-search-searxng-url">
                    {t("settings.searchSearxngUrl")}
                  </label>
                  <input
                    id="ai-search-searxng-url"
                    className="app-modal__field"
                    type="text"
                    value={form.searchSearxngBaseUrl}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, searchSearxngBaseUrl: e.target.value }))
                    }
                    placeholder="http://localhost:8080"
                    autoComplete="off"
                  />
                  <p className="ai-settings__hint">{t("settings.searchSearxngHelp")}</p>
                </>
              )}

              {form.searchProvider === "tavily" && (
                <>
                  <label className="ai-settings__label" htmlFor="ai-search-tavily-key">
                    {t("settings.searchTavilyApiKey")}
                  </label>
                  <input
                    id="ai-search-tavily-key"
                    className="app-modal__field"
                    type="password"
                    value={form.searchTavilyApiKey}
                    onChange={(e) => setForm((f) => ({ ...f, searchTavilyApiKey: e.target.value }))}
                    autoComplete="off"
                  />
                  <p className="ai-settings__hint">{t("settings.searchTavilyHelp")}</p>
                </>
              )}

              {form.searchProvider === "aliyun-opensearch" && (
                <>
                  <label className="ai-settings__label" htmlFor="ai-search-aliyun-endpoint">
                    {t("settings.searchAliyunEndpoint")}
                  </label>
                  <input
                    id="ai-search-aliyun-endpoint"
                    className="app-modal__field"
                    type="text"
                    value={form.searchAliyunEndpoint}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, searchAliyunEndpoint: e.target.value }))
                    }
                    placeholder="http://xxxx-hangzhou.opensearch.aliyuncs.com/v3/openapi/workspaces/default/web-search/ops-web-search-001"
                    autoComplete="off"
                  />
                  <label className="ai-settings__label" htmlFor="ai-search-aliyun-key">
                    {t("settings.searchAliyunApiKey")}
                  </label>
                  <input
                    id="ai-search-aliyun-key"
                    className="app-modal__field"
                    type="password"
                    value={form.searchAliyunApiKey}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, searchAliyunApiKey: e.target.value }))
                    }
                    autoComplete="off"
                  />
                  <p className="ai-settings__hint">{t("settings.searchAliyunHelp")}</p>
                </>
              )}
            </fieldset>

              </div>
            </details>

            {/* ── Group: Cognitive ── */}
            <details className="ai-settings__group">
              <summary className="ai-settings__group-summary">{t("settings.cognitiveGroup")}</summary>
              <div className="ai-settings__group-body">

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.passiveHighlightSection")}</legend>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.passiveHighlightEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, passiveHighlightEnabled: e.target.checked }))
                  }
                />
                {t("settings.passiveHighlightEnable")}
              </label>
              <label className="ai-settings__label" htmlFor="ai-passive-min-conf">
                {t("settings.passiveHighlightMinConf")}
              </label>
              <input
                id="ai-passive-min-conf"
                className="app-modal__field"
                value={form.passiveHighlightConfidenceMin}
                onChange={(e) =>
                  setForm((f) => ({ ...f, passiveHighlightConfidenceMin: e.target.value }))
                }
                inputMode="decimal"
                autoComplete="off"
              />
            </fieldset>

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.writingCoachSection")}</legend>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.writingCoachEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, writingCoachEnabled: e.target.checked }))
                  }
                />
                {t("settings.writingCoachEnable")}
              </label>
              <label className="ai-settings__label" htmlFor="ai-wc-idle-sec">
                {t("settings.writingCoachIdleSeconds")}
              </label>
              <input
                id="ai-wc-idle-sec"
                className="app-modal__field"
                value={form.writingCoachIdleSeconds}
                onChange={(e) => setForm((f) => ({ ...f, writingCoachIdleSeconds: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.writingCoachIdleSecondsHint")}</p>
              <label className="ai-settings__label" htmlFor="ai-wc-depth-chars">
                {t("settings.writingCoachDepthMinChars")}
              </label>
              <input
                id="ai-wc-depth-chars"
                className="app-modal__field"
                value={form.writingCoachDepthMinChars}
                onChange={(e) => setForm((f) => ({ ...f, writingCoachDepthMinChars: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.writingCoachDepthMinCharsHint")}</p>
              <label className="ai-settings__label" htmlFor="ai-wc-term-chars">
                {t("settings.writingCoachTermMinChars")}
              </label>
              <input
                id="ai-wc-term-chars"
                className="app-modal__field"
                value={form.writingCoachTermMinChars}
                onChange={(e) => setForm((f) => ({ ...f, writingCoachTermMinChars: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.writingCoachTermMinCharsHint")}</p>
              <label className="ai-settings__label" htmlFor="ai-wc-bubble-sec">
                {t("settings.writingCoachBubbleSeconds")}
              </label>
              <input
                id="ai-wc-bubble-sec"
                className="app-modal__field"
                value={form.writingCoachBubbleSeconds}
                onChange={(e) => setForm((f) => ({ ...f, writingCoachBubbleSeconds: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.writingCoachBubbleSecondsHint")}</p>
              <label className="ai-settings__label" htmlFor="ai-wc-cooldown-min">
                {t("settings.writingCoachCooldownMinutes")}
              </label>
              <input
                id="ai-wc-cooldown-min"
                className="app-modal__field"
                value={form.writingCoachCooldownMinutes}
                onChange={(e) => setForm((f) => ({ ...f, writingCoachCooldownMinutes: e.target.value }))}
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.writingCoachCooldownMinutesHint")}</p>
            </fieldset>

            <fieldset className="ai-settings__fieldset" disabled={!tauriRuntime || !workspaceReady}>
              <legend className="ai-settings__legend">{t("settings.independentReviewSection")}</legend>
              <label className="ai-settings__check">
                <input
                  type="checkbox"
                  checked={form.independentReviewEnabled}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, independentReviewEnabled: e.target.checked }))
                  }
                />
                {t("settings.independentReviewEnable")}
              </label>
              <p className="ai-settings__hint">{t("settings.challengeReviewShortcutHint")}</p>
              <label className="ai-settings__label" htmlFor="ai-cr-cap-independent">
                {t("settings.challengeReviewDailyCapIndependent")}
              </label>
              <input
                id="ai-cr-cap-independent"
                className="app-modal__field"
                value={form.challengeReviewDailyCapIndependent}
                onChange={(e) =>
                  setForm((f) => ({ ...f, challengeReviewDailyCapIndependent: e.target.value }))
                }
                inputMode="numeric"
                autoComplete="off"
              />
              <label className="ai-settings__label" htmlFor="ai-cr-cap-inline">
                {t("settings.challengeReviewDailyCapInline")}
              </label>
              <input
                id="ai-cr-cap-inline"
                className="app-modal__field"
                value={form.challengeReviewDailyCapInline}
                onChange={(e) =>
                  setForm((f) => ({ ...f, challengeReviewDailyCapInline: e.target.value }))
                }
                inputMode="numeric"
                autoComplete="off"
              />
              <p className="ai-settings__hint">{t("settings.challengeReviewDailyCapHint")}</p>
            </fieldset>

              </div>
            </details>

            </div>
                      </>
                    )}

                    <div className="app-modal__actions ai-settings__actions">
                      <button
                        type="button"
                        className="app-modal__btn app-modal__btn--primary"
                        disabled={saving || loading || !tauriRuntime || !workspaceReady}
                        onClick={() => void handleSave()}
                      >
                        {saving ? t("settings.saving") : t("settings.save")}
                      </button>
                    </div>
                  </div>
                ) : (
                  <Suspense fallback={<p className="ai-settings__loading">{t("settings.loading")}</p>}>
                    <SkillManagementPanel
                      open={true}
                      onClose={() => {}}
                      embedded={true}
                      workspaceReady={workspaceReady}
                      tauriRuntime={tauriRuntime}
                      dragExcludeProps={dragExcludeProps}
                    />
                  </Suspense>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );

  return scrim;
}

export default AiLlmSettingsModal;
