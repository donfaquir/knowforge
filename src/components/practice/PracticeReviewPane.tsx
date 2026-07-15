/**
 * Practice Mode — Review sub-tab (purified: thoughts only, no candidates).
 * Based on ChallengeReviewPanel, with candidate branches removed.
 */
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAiNoteContext } from "../../contexts/AiNoteContext";
import { getAppLocale } from "../../i18n";
import type {
  EvaluateChallengeAnswerResponse,
  GenerateChallengeQuestionResponse,
  ListReviewQueueResponse,
  ReviewQueueItem,
} from "../../types/cognitiveTypes";
import { localTodayKey, useCognitiveFrequencyControl } from "../../hooks/useCognitiveFrequencyControl";
import { trackKnowforgeEvent } from "../../utils/knowforgeAnalytics";
import { VAULT_CONFIG_UPDATED_EVENT } from "../../utils/vaultConfigBroadcast";
import { useAiConfigStatus } from "../../hooks/useAiConfigStatus";
import AiNotConfiguredGuide from "../AiNotConfiguredGuide";
import { AiAssistantMarkdown } from "../AiAssistantMarkdown";
import { ChallengeFeedbackBar } from "../ChallengeFeedbackBar";
import "../ChallengeReviewPanel.css";

export interface PracticeReviewPaneProps {
  workspaceReady: boolean;
  tauriRuntime: boolean;
  onReviewCompleted: () => void;
  onFocusThought?: (item: ReviewQueueItem | null) => void;
}

function displayName(relPath: string): string {
  const name = relPath.split("/").pop() ?? relPath;
  return name.replace(/\.md$/i, "");
}

function itemCacheKey(item: ReviewQueueItem): string {
  return item.thoughtId || `${item.relPath}:${item.startLine ?? 0}`;
}

export function PracticeReviewPane({
  onReviewCompleted,
  onFocusThought,
}: PracticeReviewPaneProps) {
  const { t, i18n } = useTranslation();
  const { openMarkdownTab } = useAiNoteContext();
  const { isConfigured: aiConfigured } = useAiConfigStatus(true);
  const freqCtrl = useCognitiveFrequencyControl();
  const [queue, setQueue] = useState<ListReviewQueueResponse | null>(null);
  const [cursor, setCursor] = useState(0);
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState("");
  const [phase, setPhase] = useState<"pick" | "qa" | "result">("pick");
  const [busy, setBusy] = useState(false);
  const [evalRes, setEvalRes] = useState<EvaluateChallengeAnswerResponse | null>(null);
  const [templateKind, setTemplateKind] = useState<string | undefined>();
  const [capBlocked, setCapBlocked] = useState(false);

  // --- Pre-generation pipeline ---
  const questionCacheRef = useRef<Map<string, GenerateChallengeQuestionResponse>>(new Map());
  const inflightRef = useRef<Map<string, Promise<GenerateChallengeQuestionResponse>>>(new Map());
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const todayDayStats = useMemo(() => {
    const k = localTodayKey();
    return (
      freqCtrl.challengeReviewInlineDates.byDay[k] ?? {
        inlineShown: 0,
        thoughtIdsInline: [] as string[],
        independentShown: 0,
        thoughtIdsIndependent: [] as string[],
      }
    );
  }, [freqCtrl.challengeReviewInlineDates]);

  /** Filter to thoughts only */
  const thoughtItems = useMemo(
    () => (queue?.items ?? []).filter((i) => i.sourceType === "thought"),
    [queue],
  );

  const currentItem: ReviewQueueItem | undefined = thoughtItems[cursor];

  // Notify parent when focused thought changes
  useEffect(() => {
    onFocusThought?.(currentItem ?? null);
  }, [currentItem, onFocusThought]);

  const reloadQueue = useCallback(async (): Promise<ListReviewQueueResponse | null> => {
    try {
      const q = await invoke<ListReviewQueueResponse>("list_review_queue");
      setQueue(q);
      return q;
    } catch {
      setQueue(null);
      return null;
    }
  }, []);

  const buildQuestionArgs = useCallback(
    (item: ReviewQueueItem) => ({
      thoughtExcerpt: item.excerpt || item.created,
      relPath: item.relPath,
      depthMode: "medium" as const,
      uiLocale: getAppLocale(),
      ...(item.thoughtId ? { thoughtId: item.thoughtId } : {}),
    }),
    [],
  );

  const prefetchQuestion = useCallback(
    (items: ReviewQueueItem[], startIdx: number) => {
      const item = items[startIdx];
      if (!item) return;
      const key = itemCacheKey(item);
      if (questionCacheRef.current.has(key) || inflightRef.current.has(key)) return;
      const promise = invoke<GenerateChallengeQuestionResponse>("generate_challenge_question", {
        args: buildQuestionArgs(item),
      });
      inflightRef.current.set(key, promise);
      promise
        .then((g) => {
          questionCacheRef.current.set(key, g);
          if (startIdx + 1 < items.length && questionCacheRef.current.size < 3) {
            prefetchQuestion(items, startIdx + 1);
          }
        })
        .catch(() => {})
        .finally(() => { inflightRef.current.delete(key); });
    },
    [buildQuestionArgs],
  );

  const hydrateFromVault = useCallback(async () => {
    try {
      await freqCtrl.reload();
      const q = await reloadQueue();
      setCapBlocked(!freqCtrl.canStartMoreIndependentReviewsToday());
      questionCacheRef.current.clear();
      inflightRef.current.clear();
      if (q) {
        const thoughts = q.items.filter((i) => i.sourceType === "thought");
        if (thoughts.length > 0) {
          prefetchQuestion(thoughts, 0);
        }
      }
    } catch {
      setQueue(null);
    }
  }, [freqCtrl.reload, freqCtrl.canStartMoreIndependentReviewsToday, reloadQueue, prefetchQuestion]);

  useEffect(() => { void hydrateFromVault(); }, [hydrateFromVault]);

  useEffect(() => {
    const onConfigUpdated = () => { void hydrateFromVault(); };
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, onConfigUpdated);
    return () => window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, onConfigUpdated);
  }, [hydrateFromVault]);

  const applyQuestion = (g: GenerateChallengeQuestionResponse) => {
    setTemplateKind(g.templateKind || undefined);
    if (g.shouldSkip || !g.question.trim()) {
      setQuestion(t("challengeReview.fallbackQuestion"));
    } else {
      setQuestion(g.question);
    }
    setPhase("qa");
    setAnswer("");
    setEvalRes(null);
  };

  const startRound = async () => {
    if (!currentItem) return;
    const key = itemCacheKey(currentItem);

    const cached = questionCacheRef.current.get(key);
    if (cached) {
      questionCacheRef.current.delete(key);
      applyQuestion(cached);
      prefetchQuestion(thoughtItems, cursor + 1);
      return;
    }

    const inflight = inflightRef.current.get(key);
    if (inflight) {
      setBusy(true);
      try {
        const g = await inflight;
        applyQuestion(g);
        prefetchQuestion(thoughtItems, cursor + 1);
      } catch {
        setQuestion(t("challengeReview.fallbackQuestion"));
        setPhase("qa");
        setAnswer("");
        setEvalRes(null);
      } finally { setBusy(false); }
      return;
    }

    setBusy(true);
    try {
      const g = await invoke<GenerateChallengeQuestionResponse>("generate_challenge_question", {
        args: buildQuestionArgs(currentItem),
      });
      applyQuestion(g);
      prefetchQuestion(thoughtItems, cursor + 1);
    } finally { setBusy(false); }
  };

  const submitAnswer = async () => {
    if (!currentItem || !answer.trim()) return;
    setBusy(true);
    try {
      const ev = await invoke<EvaluateChallengeAnswerResponse>("evaluate_challenge_answer", {
        args: {
          question,
          userAnswer: answer.trim(),
          thoughtExcerpt: currentItem.excerpt || "",
          depthMode: "medium",
          uiLocale: getAppLocale(),
        },
      });
      setEvalRes(ev);
      setPhase("result");
      void trackKnowforgeEvent("review.practice_evaluated", {
        thoughtId: currentItem.thoughtId,
        passed: ev.passed,
        sloppy: ev.sloppy,
      });
      await invoke("apply_challenge_pass_to_thought", {
        args: {
          relPath: currentItem.relPath,
          thoughtId: currentItem.thoughtId,
          passed: ev.passed && !ev.sloppy,
          sloppy: ev.sloppy,
        },
      });
      if (ev.passed && !ev.sloppy) {
        await freqCtrl.recordChallengeIndependentShown(currentItem.thoughtId);
        await freqCtrl.reload();
        if (!freqCtrl.canStartMoreIndependentReviewsToday()) {
          setCapBlocked(true);
        }
      }
    } catch {
      setEvalRes({
        passed: false,
        sloppy: false,
        commentaryMd: t("challengeReview.evaluateError"),
      });
      setPhase("result");
    } finally { setBusy(false); }
  };

  const goNext = async () => {
    const passedLast = evalRes?.passed === true && evalRes?.sloppy !== true;
    const prevCursor = cursor;
    setPhase("pick");
    setQuestion("");
    setAnswer("");
    setEvalRes(null);
    const q = await reloadQueue();
    await freqCtrl.reload();
    if (!freqCtrl.canStartMoreIndependentReviewsToday()) {
      setCapBlocked(true);
    }
    if (!q) { setCursor(0); return; }
    const thoughts = q.items.filter((i) => i.sourceType === "thought");
    if (thoughts.length === 0) {
      setCursor(0);
      onReviewCompleted();
      return;
    }
    const nextCursor = passedLast ? 0 : Math.min(prevCursor + 1, thoughts.length - 1);
    setCursor(nextCursor);
    prefetchQuestion(thoughts, nextCursor);
  };

  // Keyboard flow
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (phase === "qa" && e.key === "Enter" && !e.shiftKey && answer.trim() && !busy) {
        const active = document.activeElement;
        if (active === textareaRef.current) {
          e.preventDefault();
          void submitAnswer();
        }
      }
      if (phase === "result" && (e.key === "Tab" || e.key === "ArrowRight")) {
        e.preventDefault();
        void goNext().catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, answer, busy, evalRes]);

  const createdDisplay = useCallback(
    (createdRaw: string) => {
      const d = new Date(createdRaw);
      if (Number.isNaN(d.getTime())) return createdRaw;
      return d.toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" });
    },
    [i18n.language],
  );

  // --- Simplified stats ---
  const statsLine = useMemo(() => {
    const done = todayDayStats.independentShown;
    const cap = freqCtrl.challengeReviewDailyCapIndependent;
    const due = queue?.totalDue ?? 0;
    return t("practice.reviewStats", {
      defaultValue: "Today {{done}}/{{cap}} · {{due}} thoughts due",
      done,
      cap,
      due,
    });
  }, [todayDayStats.independentShown, freqCtrl.challengeReviewDailyCapIndependent, queue?.totalDue, t]);

  // --- Early returns ---
  if (!aiConfigured) {
    return (
      <div className="challenge-review-panel">
        <AiNotConfiguredGuide
          featureName={t("challengeReview.panelTitle")}
          featureDescription={t("aiGuide.descChallengeReview")}
          compact
        />
      </div>
    );
  }

  if (capBlocked) {
    return (
      <div className="challenge-review-panel">
        <div className="challenge-review-panel__stats" role="status">
          <div className="challenge-review-panel__stats-row">{statsLine}</div>
        </div>
        <p className="challenge-review-panel__hint">{t("challengeReview.panelIndependentCap")}</p>
        <p className="challenge-review-panel__hint challenge-review-panel__hint--compact">
          {t("challengeReview.panelIndependentCapHint")}
        </p>
      </div>
    );
  }

  if (thoughtItems.length === 0) {
    return (
      <div className="challenge-review-panel">
        <div className="challenge-review-panel__stats" role="status">
          <div className="challenge-review-panel__stats-row">{statsLine}</div>
        </div>
        <p className="challenge-review-panel__hint">{t("challengeReview.panelEmpty")}</p>
      </div>
    );
  }

  if (!currentItem) {
    onReviewCompleted();
    return (
      <div className="challenge-review-panel">
        <p className="challenge-review-panel__hint">{t("challengeReview.panelNoMore")}</p>
      </div>
    );
  }

  return (
    <div className="challenge-review-panel">
      <div className="challenge-review-panel__stats" role="status">
        <div className="challenge-review-panel__stats-row">{statsLine}</div>
      </div>

      {/* Queue list */}
      <div className="challenge-review-panel__queue" role="region" aria-label={t("challengeReview.panelBatchListTitle", { count: thoughtItems.length })}>
        <div className="challenge-review-panel__queue-title">{t("challengeReview.panelBatchListTitle", { count: thoughtItems.length })}</div>
        <ul className="challenge-review-panel__queue-list" role="list">
          {thoughtItems.map((it, i) => (
            <li key={it.thoughtId || `item-${i}`} className="challenge-review-panel__queue-li">
              <button
                type="button"
                className={`challenge-review-panel__queue-row${i === cursor ? " is-active" : ""}`}
                disabled={phase !== "pick" || busy}
                aria-current={i === cursor ? "true" : undefined}
                onClick={() => { if (phase === "pick" && !busy) setCursor(i); }}
              >
                <div className="challenge-review-panel__queue-row-top">
                  <span className="challenge-review-panel__queue-idx">{i + 1}</span>
                  <span className="challenge-review-panel__queue-path" title={it.relPath}>
                    {displayName(it.relPath)}
                  </span>
                  <span className="challenge-review-panel__queue-due">
                    {t("challengeReview.dueLabel", { days: it.overdueDays })}
                  </span>
                </div>
                {it.privateOmitted ? (
                  <div className="challenge-review-panel__queue-excerpt challenge-review-panel__queue-excerpt--muted">
                    {t("challengeReview.queueExcerptPrivate")}
                  </div>
                ) : it.excerpt ? (
                  <div className="challenge-review-panel__queue-excerpt">{it.excerpt}</div>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      </div>

      {/* Pick phase */}
      {phase === "pick" ? (
        <>
          <div className="challenge-review-panel__meta">
            <span>{currentItem.relPath}</span>
            <span className="challenge-review-panel__due">
              {t("challengeReview.dueLabel", { days: currentItem.overdueDays })}
            </span>
          </div>
          <div className="challenge-review-panel__created">
            {t("challengeReview.createdLabel", { time: createdDisplay(currentItem.created) })}
          </div>
          {currentItem.excerpt && !currentItem.privateOmitted ? (
            <div className="challenge-review-panel__excerpt">{currentItem.excerpt}</div>
          ) : null}
          <div className="challenge-review-panel__actions challenge-review-panel__actions--spread">
            <button
              type="button"
              className="challenge-review-panel__btn challenge-review-panel__btn--primary"
              disabled={busy}
              onClick={() => void startRound()}
            >
              {busy ? t("challengeReview.generating") : t("challengeReview.startRound")}
            </button>
            {thoughtItems.length > 1 ? (
              <button
                type="button"
                className="challenge-review-panel__btn"
                disabled={busy}
                onClick={() => setCursor((c) => (c + 1) % thoughtItems.length)}
              >
                {t("challengeReview.skipItem")}
              </button>
            ) : null}
            {openMarkdownTab ? (
              <button
                type="button"
                className="challenge-review-panel__btn"
                disabled={busy}
                onClick={() => openMarkdownTab(currentItem.relPath)}
              >
                {t("challengeReview.openSourceNote")}
              </button>
            ) : null}
          </div>
        </>
      ) : null}

      {/* QA phase */}
      {phase === "qa" ? (
        <>
          <div className="challenge-review-panel__question">{question}</div>
          <textarea
            ref={textareaRef}
            className="challenge-review-panel__textarea"
            value={answer}
            onChange={(e) => setAnswer(e.target.value)}
            rows={4}
            disabled={busy}
            placeholder={t("challengeReview.answerPlaceholder")}
          />
          <div className="challenge-review-panel__actions challenge-review-panel__actions--spread">
            <button
              type="button"
              className="challenge-review-panel__btn challenge-review-panel__btn--primary"
              disabled={busy || !answer.trim()}
              onClick={() => void submitAnswer()}
            >
              {t("challengeReview.submit")}
            </button>
            <button
              type="button"
              className="challenge-review-panel__btn"
              disabled={busy}
              onClick={() => { setPhase("pick"); setQuestion(""); setAnswer(""); setEvalRes(null); }}
            >
              {t("challengeReview.abandon")}
            </button>
          </div>
        </>
      ) : null}

      {/* Result phase */}
      {phase === "result" && evalRes ? (
        <div className="challenge-review-panel__result">
          {evalRes.sloppy ? <p className="challenge-review-panel__sloppy">{t("challengeReview.sloppyHint")}</p> : null}
          {evalRes.passed ? (
            <>
              <p className="challenge-review-panel__pass">{t("challengeReview.passed")}</p>
              {!evalRes.sloppy ? (
                <p className="challenge-review-panel__maturity">{t("challengeReview.maturitySignalHint")}</p>
              ) : null}
            </>
          ) : null}
          <AiAssistantMarkdown content={evalRes.commentaryMd} className="challenge-review-panel__md" />
          <ChallengeFeedbackBar
            thoughtId={currentItem?.thoughtId}
            questionText={question}
            questionTemplate={templateKind}
          />
          <div className="challenge-review-panel__actions">
            <button type="button" className="challenge-review-panel__btn" onClick={() => void goNext().catch(() => {})}>
              {t("challengeReview.continueNext")}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
