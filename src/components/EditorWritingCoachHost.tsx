import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getAppLocale } from "../i18n";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import type { DepthMode } from "../types/cognitiveTypes";
import type { VaultConfigForUi } from "../types/vaultAiConfig";
import type { AnalyzeWritingCoachResponse } from "../types/writingCoach";
import { dispatchVaultConfigUpdated, VAULT_CONFIG_UPDATED_EVENT } from "../utils/vaultConfigBroadcast";
import { nearestTextblockText, useWritingCoachTrigger } from "../hooks/useWritingCoachTrigger";
import type { CrepeMarkdownEditorApi } from "./CrepeMarkdownEditor";
import { useTranslation } from "react-i18next";
import { WritingCoachBubble } from "./WritingCoachBubble";
import { useAiConfigStatus } from "../hooks/useAiConfigStatus";
import AiNotConfiguredGuide from "./AiNotConfiguredGuide";
import { endPerfTrace, startPerfTrace } from "../utils/perfTrace";
import "./EditorWritingCoachHost.css";

const FADE_MS = 480;

function extractStreamedQuestions(text: string): string[] {
  const keyIdx = text.indexOf('"reasoningQuestions"');
  if (keyIdx === -1) return [];
  const bracketIdx = text.indexOf("[", keyIdx);
  if (bracketIdx === -1) return [];
  const after = text.slice(bracketIdx + 1);
  const questions: string[] = [];
  let pos = 0;
  while (pos < after.length) {
    const c = after[pos];
    if (c === "]") break;
    if (c === '"') {
      let end = pos + 1;
      let str = "";
      while (end < after.length) {
        if (after[end] === "\\" && end + 1 < after.length) {
          str += after[end + 1];
          end += 2;
          continue;
        }
        if (after[end] === '"') break;
        str += after[end];
        end++;
      }
      if (end >= after.length) break;
      questions.push(str);
      pos = end + 1;
    } else {
      pos++;
    }
  }
  return questions;
}

export type EditorWritingCoachHostHandle = {
  triggerManually: () => void;
};

type Props = {
  editorApiRef: React.MutableRefObject<CrepeMarkdownEditorApi | null>;
  activePath: string | null;
  workspaceReady: boolean;
  /** 源码/双栏：不触发（对应母文档「预览」映射） */
  showMarkdownSource: boolean;
  onOpenMarkdownPath: (
    relPath: string,
    meta?: { headingFragment?: string | null },
  ) => void | Promise<void>;
};

function cooldownActive(iso: string | undefined): boolean {
  if (!iso?.trim()) {
    return false;
  }
  const t = Date.parse(iso);
  return Number.isFinite(t) && t > Date.now();
}

export const EditorWritingCoachHost = forwardRef<EditorWritingCoachHostHandle, Props>(function EditorWritingCoachHost({
  editorApiRef,
  activePath,
  workspaceReady,
  showMarkdownSource,
  onOpenMarkdownPath,
}, ref) {
  const { t } = useTranslation();
  const { isConfigured: aiConfigured } = useAiConfigStatus(workspaceReady);
  const hostRef = useRef<HTMLDivElement>(null);
  const [depthMode, setDepthMode] = useState<DepthMode>("auto");
  const [writingCoachEnabled, setWritingCoachEnabled] = useState(true);
  const [cooldownUntil, setCooldownUntil] = useState<string | undefined>(undefined);
  const [wcIdleSeconds, setWcIdleSeconds] = useState(15);
  const [wcDepthMinChars, setWcDepthMinChars] = useState(500);
  const [wcTermMinChars, setWcTermMinChars] = useState(36);
  const [wcBubbleSeconds, setWcBubbleSeconds] = useState(30);
  const [wcCooldownMinutes, setWcCooldownMinutes] = useState(15);

  const reloadCfg = useCallback(async () => {
    if (!isTauri() || !workspaceReady) {
      return;
    }
    const cfgTrace = startPerfTrace("markdown.writing_coach.load_config");
    try {
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      endPerfTrace(cfgTrace, { status: "ok" });
      setDepthMode(cfg.cognitive.depthMode ?? "auto");
      setWritingCoachEnabled(cfg.cognitive.writingCoachEnabled !== false);
      setCooldownUntil(cfg.cognitive.writingCoachCooldownUntil);
      const c = cfg.cognitive;
      setWcIdleSeconds(c.writingCoachIdleSeconds ?? 15);
      setWcDepthMinChars(c.writingCoachDepthMinChars ?? 500);
      setWcTermMinChars(c.writingCoachTermMinChars ?? 36);
      setWcBubbleSeconds(c.writingCoachBubbleSeconds ?? 30);
      setWcCooldownMinutes(c.writingCoachCooldownMinutes ?? 15);
    } catch {
      endPerfTrace(cfgTrace, { status: "error" });
      /* 忽略 */
    }
  }, [workspaceReady]);

  useEffect(() => {
    void reloadCfg();
  }, [reloadCfg]);

  useEffect(() => {
    const onCfg = () => void reloadCfg();
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
    return () => window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, onCfg);
  }, [reloadCfg]);

  const depthBlocksCoach = depthMode === "shallow";
  const cdActive = cooldownActive(cooldownUntil);
  const gatesOk =
    workspaceReady &&
    writingCoachEnabled &&
    aiConfigured &&
    !depthBlocksCoach &&
    !cdActive &&
    !showMarkdownSource &&
    !!activePath;

  const getEditorView = useCallback(() => editorApiRef.current?.getEditorView() ?? null, [editorApiRef]);

  const [bubbleVisible, setBubbleVisible] = useState(false);
  const [bubbleFading, setBubbleFading] = useState(false);
  const [anchorTopPx, setAnchorTopPx] = useState(48);
  const [panelOpen, setPanelOpen] = useState(false);
  const [panelLoading, setPanelLoading] = useState(false);
  const [panelError, setPanelError] = useState<string | null>(null);
  const [coachData, setCoachData] = useState<AnalyzeWritingCoachResponse | null>(null);
  const paragraphRef = useRef("");
  /** 防止同一停顿内重复弹出（interval 与 setState 竞态） */
  const triggerLockRef = useRef(false);

  const bubbleTimerRef = useRef<number | null>(null);
  const fadeTimerRef = useRef<number | null>(null);

  const clearBubbleTimers = useCallback(() => {
    if (bubbleTimerRef.current != null) {
      window.clearTimeout(bubbleTimerRef.current);
      bubbleTimerRef.current = null;
    }
    if (fadeTimerRef.current != null) {
      window.clearTimeout(fadeTimerRef.current);
      fadeTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      clearBubbleTimers();
    };
  }, [clearBubbleTimers]);

  const saveCooldown = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    const until = new Date(Date.now() + wcCooldownMinutes * 60 * 1000).toISOString();
    try {
      await invoke("save_vault_config_patch", {
        patch: { cognitive: { writingCoachCooldownUntil: until } },
      });
      setCooldownUntil(until);
      dispatchVaultConfigUpdated();
    } catch {
      /* 忽略 */
    }
  }, [wcCooldownMinutes]);

  const clearCooldown = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    try {
      await invoke("save_vault_config_patch", {
        patch: { cognitive: { writingCoachCooldownUntil: null } },
      });
      setCooldownUntil(undefined);
      dispatchVaultConfigUpdated();
    } catch {
      /* 忽略 */
    }
  }, []);

  useEffect(() => {
    if (!bubbleVisible && !panelOpen) {
      triggerLockRef.current = false;
    }
  }, [bubbleVisible, panelOpen]);

  const onFire = useCallback(
    (p: { paragraphText: string; anchorTopPx: number }) => {
      if (triggerLockRef.current) {
        return;
      }
      triggerLockRef.current = true;
      paragraphRef.current = p.paragraphText;
      setAnchorTopPx(Math.max(24, p.anchorTopPx));
      setBubbleFading(false);
      setBubbleVisible(true);
      clearBubbleTimers();
      bubbleTimerRef.current = window.setTimeout(() => {
        bubbleTimerRef.current = null;
        setBubbleFading(true);
        fadeTimerRef.current = window.setTimeout(() => {
          fadeTimerRef.current = null;
          setBubbleVisible(false);
          setBubbleFading(false);
          void saveCooldown();
        }, FADE_MS);
      }, wcBubbleSeconds * 1000);
    },
    [clearBubbleTimers, saveCooldown, wcBubbleSeconds],
  );

  const triggerDisabled = useMemo(
    () => !gatesOk || bubbleVisible || panelOpen,
    [gatesOk, bubbleVisible, panelOpen],
  );

  useWritingCoachTrigger({
    hostRef: hostRef,
    getEditorView,
    docKey: activePath,
    disabled: triggerDisabled,
    idleMs: wcIdleSeconds * 1000,
    depthMinChars: wcDepthMinChars,
    termMinChars: wcTermMinChars,
    onFire,
  });

  useEffect(() => {
    setBubbleVisible(false);
    setBubbleFading(false);
    setPanelOpen(false);
    setCoachData(null);
    setPanelError(null);
    setShowAiGuide(false);
    clearBubbleTimers();
  }, [activePath, clearBubbleTimers]);

  const analyzeAndShowPanel = useCallback((text: string) => {
    setPanelOpen(true);
    setPanelLoading(true);
    setPanelError(null);
    setCoachData(null);
    const rel = activePath ?? "";
    if (!isTauri() || !rel) {
      setPanelLoading(false);
      setPanelError("Not available.");
      return;
    }
    const sessionId = `wc-${crypto.randomUUID()}`;
    let accumulated = "";
    const unlisteners: UnlistenFn[] = [];
    let cleaned = false;
    const cleanup = () => {
      if (cleaned) return;
      cleaned = true;
      for (const fn of unlisteners) fn();
      unlisteners.length = 0;
    };
    void (async () => {
      try {
        const chunkUn = await listen<{ sessionId: string; delta: string }>(
          "llm:stream-chunk",
          (e) => {
            if (e.payload.sessionId !== sessionId) return;
            accumulated += e.payload.delta;
            const questions = extractStreamedQuestions(accumulated);
            if (questions.length > 0) {
              setCoachData((prev) => ({
                reasoningQuestions: questions,
                links: prev?.links ?? [],
                knowledgeModuleSkipped: prev?.knowledgeModuleSkipped ?? false,
              }));
            }
          },
        );
        unlisteners.push(chunkUn);
        const resp = await invoke<AnalyzeWritingCoachResponse>("analyze_writing_coach", {
          args: { paragraphText: text, relPath: rel, uiLocale: getAppLocale(), sessionId },
        });
        setCoachData(resp);
      } catch (e) {
        setPanelError(e instanceof Error ? e.message : String(e));
      } finally {
        cleanup();
        setPanelLoading(false);
      }
    })();
  }, [activePath]);

  const onBubbleClick = useCallback(() => {
    clearBubbleTimers();
    setBubbleVisible(false);
    setBubbleFading(false);
    analyzeAndShowPanel(paragraphRef.current);
  }, [analyzeAndShowPanel, clearBubbleTimers]);

  const [showAiGuide, setShowAiGuide] = useState(false);

  const handleManualTrigger = useCallback(() => {
    if (!aiConfigured) {
      setShowAiGuide(true);
      return;
    }
    const view = editorApiRef.current?.getEditorView();
    if (!view || !activePath) return;
    const text = nearestTextblockText(view.state);
    if (!text.trim()) return;
    clearBubbleTimers();
    setBubbleVisible(false);
    setBubbleFading(false);
    triggerLockRef.current = true;
    analyzeAndShowPanel(text);
  }, [activePath, aiConfigured, analyzeAndShowPanel, clearBubbleTimers, editorApiRef]);

  useImperativeHandle(ref, () => ({
    triggerManually: handleManualTrigger,
  }), [handleManualTrigger]);

  const onCollapsePanel = useCallback(() => {
    setPanelOpen(false);
    setCoachData(null);
    setPanelError(null);
  }, []);

  const onHelpful = useCallback(() => {
    void clearCooldown();
    onCollapsePanel();
  }, [clearCooldown, onCollapsePanel]);

  if (!workspaceReady || !activePath) {
    return null;
  }

  const showTriggerBtn = !bubbleVisible && !panelOpen && !showMarkdownSource && !showAiGuide;

  return (
    <div ref={hostRef} className="editor-writing-coach-host" aria-hidden={!gatesOk && !panelOpen && !showTriggerBtn && !showAiGuide}>
      {showAiGuide ? (
        <div className="writing-coach-panel" role="dialog" aria-label={t("aiGuide.title", { feature: t("aiGuide.descWritingCoach") })}>
          <AiNotConfiguredGuide
            featureName={t("main.writingCoachPanelTitle")}
            featureDescription={t("aiGuide.descWritingCoach")}
          />
          <div className="writing-coach-panel__actions">
            <button type="button" className="writing-coach-panel__btn" onClick={() => setShowAiGuide(false)}>
              {t("main.writingCoachCollapse")}
            </button>
          </div>
        </div>
      ) : null}
      {showTriggerBtn ? (
        <button
          type="button"
          className="writing-coach-trigger"
          onClick={handleManualTrigger}
          aria-label={t("main.writingCoachTrigger")}
          title={t("main.writingCoachTrigger")}
        >
          <svg viewBox="0 0 24 24" width={14} height={14} aria-hidden>
            <path
              fill="currentColor"
              fillOpacity={0.22}
              stroke="currentColor"
              strokeWidth={1.15}
              strokeLinejoin="round"
              d="M12 5 17 12 12 19 7 12 12 5z"
            />
          </svg>
        </button>
      ) : null}
      <WritingCoachBubble
        bubbleVisible={bubbleVisible}
        bubbleFading={bubbleFading}
        anchorTopPx={anchorTopPx}
        panelOpen={panelOpen}
        panelLoading={panelLoading}
        panelError={panelError}
        reasoningQuestions={coachData?.reasoningQuestions ?? []}
        links={coachData?.links ?? []}
        knowledgeModuleSkipped={coachData?.knowledgeModuleSkipped ?? false}
        onBubbleClick={onBubbleClick}
        onCollapsePanel={onCollapsePanel}
        onHelpful={onHelpful}
        onOpenLink={onOpenMarkdownPath}
      />
    </div>
  );
});
