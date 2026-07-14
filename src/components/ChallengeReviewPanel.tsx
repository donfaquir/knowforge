/**
 * 通道一：独立挑战回顾面板（队列 + 单条问答 + 写回）。
 */
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAiNoteContext } from "../contexts/AiNoteContext";
import { getAppLocale } from "../i18n";
import type {
  DepthMode,
  EvaluateChallengeAnswerResponse,
  GenerateChallengeQuestionResponse,
  ListReviewQueueResponse,
  ReviewQueueItem,
} from "../types/cognitiveTypes";
import type { VaultConfigForUi } from "../types/vaultAiConfig";
import { localTodayKey, useCognitiveFrequencyControl } from "../hooks/useCognitiveFrequencyControl";
import { trackKnowforgeEvent } from "../utils/knowforgeAnalytics";
import { dispatchOpenAiSettings, VAULT_CONFIG_UPDATED_EVENT } from "../utils/vaultConfigBroadcast";
import { useAiConfigStatus } from "../hooks/useAiConfigStatus";
import AiNotConfiguredGuide from "./AiNotConfiguredGuide";
import { AiAssistantMarkdown } from "./AiAssistantMarkdown";
import { CandidatePromoteCard } from "./CandidatePromoteCard";
import { ChallengeFeedbackBar } from "./ChallengeFeedbackBar";
import "./ChallengeReviewPanel.css";

type Props = {
  onClose: () => void;
  depthMode: DepthMode;
};

/** Strip .md extension and extract display name from rel path */
function displayName(relPath: string): string {
  const name = relPath.split("/").pop() ?? relPath;
  return name.replace(/\.md$/i, "");
}

/** Stable cache key for a review queue item */
function itemCacheKey(item: ReviewQueueItem): string {
  return item.candidateId || item.thoughtId || `${item.relPath}:${item.startLine ?? 0}`;
}

export function ChallengeReviewPanel({ onClose, depthMode }: Props) {
  const { t, i18n } = useTranslation();
  const { openMarkdownTab } = useAiNoteContext();
  const { isConfigured: aiConfigured } = useAiConfigStatus(true);
  const freqCtrl = useCognitiveFrequencyControl();
  const [queue, setQueue] = useState<ListReviewQueueResponse | null>(null);
  const [independent, setIndependent] = useState(false);
  const [cursor, setCursor] = useState(0);
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState("");
  const [phase, setPhase] = useState<"pick" | "qa" | "result">("pick");
  const [busy, setBusy] = useState(false);
  const [evalRes, setEvalRes] = useState<EvaluateChallengeAnswerResponse | null>(null);
  const [templateKind, setTemplateKind] = useState<string | undefined>();
  /** 当日独立回顾成功次数已达 cap */
  const [independentCapBlocked, setIndependentCapBlocked] = useState(false);

  // --- Pre-generation pipeline ---
  const questionCacheRef = useRef<Map<string, GenerateChallengeQuestionResponse>>(new Map());
  const inflightRef = useRef<Map<string, Promise<GenerateChallengeQuestionResponse>>>(new Map());

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

  /** Build invoke args for generate_challenge_question */
  const buildQuestionArgs = useCallback(
    (item: ReviewQueueItem) => {
      const isCandidate = item.sourceType === "candidate";
      return {
        thoughtExcerpt: item.excerpt || item.created,
        relPath: item.relPath,
        depthMode,
        uiLocale: getAppLocale(),
        ...(!isCandidate && item.thoughtId ? { thoughtId: item.thoughtId } : {}),
        ...(isCandidate && item.markingReason ? { markingReason: item.markingReason } : {}),
        ...(isCandidate && item.pairedExcerpt ? { pairedExcerpt: item.pairedExcerpt } : {}),
      };
    },
    [depthMode],
  );

  /** Fire-and-forget: prefetch question for an item, then chain to the next */
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
          // Chain: prefetch next item (max lookahead = 2)
          if (startIdx + 1 < items.length && questionCacheRef.current.size < 3) {
            prefetchQuestion(items, startIdx + 1);
          }
        })
        .catch(() => {
          // Ignore — will fall back to on-demand generation
        })
        .finally(() => {
          inflightRef.current.delete(key);
        });
    },
    [buildQuestionArgs],
  );

  const hydrateFromVault = useCallback(async () => {
    try {
      await freqCtrl.reload();
      const q = await reloadQueue();
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      setIndependent(cfg.cognitive.independentReviewEnabled === true);
      setIndependentCapBlocked(!freqCtrl.canStartMoreIndependentReviewsToday());
      // Start pre-generating question for the first item
      questionCacheRef.current.clear();
      inflightRef.current.clear();
      if (q && q.items.length > 0) {
        prefetchQuestion(q.items, 0);
      }
    } catch {
      setQueue(null);
    }
  }, [freqCtrl.reload, freqCtrl.canStartMoreIndependentReviewsToday, reloadQueue, prefetchQuestion]);

  useEffect(() => {
    void hydrateFromVault();
  }, [hydrateFromVault]);

  useEffect(() => {
    const onConfigUpdated = () => {
      void hydrateFromVault();
    };
    window.addEventListener(VAULT_CONFIG_UPDATED_EVENT, onConfigUpdated);
    return () => window.removeEventListener(VAULT_CONFIG_UPDATED_EVENT, onConfigUpdated);
  }, [hydrateFromVault]);

  const currentItem: ReviewQueueItem | undefined = queue?.items[cursor];

  /** Apply a generated question response to UI state */
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

  /** 仅用于本面板 onClick，不传入 memo 子组件；用 useCallback 也无法在 answer 变化时稳定引用，故保持为普通函数 */
  const startRound = async () => {
    if (!currentItem) return;
    const key = itemCacheKey(currentItem);

    // 1. Try cache — instant response
    const cached = questionCacheRef.current.get(key);
    if (cached) {
      questionCacheRef.current.delete(key);
      applyQuestion(cached);
      if (queue?.items) prefetchQuestion(queue.items, cursor + 1);
      return;
    }

    // 2. Prefetch in-flight — await the same Promise (no duplicate request)
    const inflight = inflightRef.current.get(key);
    if (inflight) {
      setBusy(true);
      try {
        const g = await inflight;
        applyQuestion(g);
        if (queue?.items) prefetchQuestion(queue.items, cursor + 1);
      } catch {
        setQuestion(t("challengeReview.fallbackQuestion"));
        setPhase("qa");
        setAnswer("");
        setEvalRes(null);
      } finally {
        setBusy(false);
      }
      return;
    }

    // 3. No cache, no inflight — generate on demand
    setBusy(true);
    try {
      const g = await invoke<GenerateChallengeQuestionResponse>("generate_challenge_question", {
        args: buildQuestionArgs(currentItem),
      });
      applyQuestion(g);
      if (queue?.items) prefetchQuestion(queue.items, cursor + 1);
    } finally {
      setBusy(false);
    }
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
          depthMode,
          uiLocale: getAppLocale(),
        },
      });
      setEvalRes(ev);
      setPhase("result");
      const isCandidate = currentItem.sourceType === "candidate";
      void trackKnowforgeEvent("review.panel_evaluated", {
        thoughtId: currentItem.thoughtId,
        passed: ev.passed,
        sloppy: ev.sloppy,
        sourceType: currentItem.sourceType,
      });
      if (!isCandidate) {
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
            setIndependentCapBlocked(true);
          }
        }
      }
    } catch {
      setEvalRes({
        passed: false,
        sloppy: false,
        commentaryMd: t("challengeReview.evaluateError"),
      });
      setPhase("result");
    } finally {
      setBusy(false);
    }
  };

  const goNext = async () => {
    // 须在清空 evalRes 之前读取，用于决定游标（通过后列表重排，应从 0 对齐本批首条）
    const passedLast = evalRes?.passed === true && evalRes?.sloppy !== true;
    const prevCursor = cursor;
    setPhase("pick");
    setQuestion("");
    setAnswer("");
    setEvalRes(null);
    const q = await reloadQueue();
    await freqCtrl.reload();
    if (!freqCtrl.canStartMoreIndependentReviewsToday()) {
      setIndependentCapBlocked(true);
    }
    if (!q || q.items.length === 0) {
      setCursor(0);
      return;
    }
    const nextCursor = passedLast ? 0 : Math.min(prevCursor + 1, q.items.length - 1);
    setCursor(nextCursor);
    // Prefetch for the upcoming item (it may already be cached from earlier)
    prefetchQuestion(q.items, nextCursor);
  };

  const createdDisplay = useCallback(
    (createdRaw: string) => {
      const d = new Date(createdRaw);
      if (Number.isNaN(d.getTime())) return createdRaw;
      return d.toLocaleString(i18n.language, { dateStyle: "short", timeStyle: "short" });
    },
    [i18n.language],
  );

  const statsBlock = (q: ListReviewQueueResponse | null) => (
    <div className="challenge-review-panel__stats" role="status">
      <div className="challenge-review-panel__stats-row">
        {t("challengeReview.panelStatsIndependent", {
          done: todayDayStats.independentShown,
          cap: freqCtrl.challengeReviewDailyCapIndependent,
        })}
      </div>
      <div className="challenge-review-panel__stats-row">
        {t("challengeReview.panelStatsInline", {
          done: todayDayStats.inlineShown,
          cap: freqCtrl.challengeReviewDailyCapInline,
        })}
      </div>
      {q ? (
        <>
          <div className="challenge-review-panel__stats-row">
            {t("challengeReview.panelStatsDue", { total: q.totalDue })}
          </div>
          <div className="challenge-review-panel__stats-row">
            {t("challengeReview.panelStatsTracked", { total: q.totalThoughts })}
          </div>
        </>
      ) : (
        <div className="challenge-review-panel__stats-row challenge-review-panel__stats-row--muted">
          {t("challengeReview.panelStatsLoading")}
        </div>
      )}
    </div>
  );

  if (!independent || !aiConfigured) {
    return (
      <div className="challenge-review-panel">
        <div className="challenge-review-panel__header">
          <span>{t("challengeReview.panelTitle")}</span>
          <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
            {t("challengeReview.close")}
          </button>
        </div>
        <AiNotConfiguredGuide
          featureName={t("challengeReview.panelTitle")}
          featureDescription={t("aiGuide.descChallengeReview")}
          compact
        />
      </div>
    );
  }

  if (independentCapBlocked) {
    return (
      <div className="challenge-review-panel">
        <div className="challenge-review-panel__header">
          <span>{t("challengeReview.panelTitle")}</span>
          <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
            {t("challengeReview.endReview")}
          </button>
        </div>
        {statsBlock(queue)}
        <p className="challenge-review-panel__hint">{t("challengeReview.panelIndependentCap")}</p>
        <p className="challenge-review-panel__hint challenge-review-panel__hint--compact">
          {t("challengeReview.panelIndependentCapHint")}
        </p>
        <div className="challenge-review-panel__footer-actions">
          <button type="button" className="challenge-review-panel__linkish" onClick={() => dispatchOpenAiSettings()}>
            {t("challengeReview.openAiSettings")}
          </button>
        </div>
      </div>
    );
  }

  const items = queue?.items ?? [];
  if (items.length === 0) {
    return (
      <div className="challenge-review-panel">
        <div className="challenge-review-panel__header">
          <span>{t("challengeReview.panelTitle")}</span>
          <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
            {t("challengeReview.close")}
          </button>
        </div>
        {statsBlock(queue)}
        <p className="challenge-review-panel__hint">{t("challengeReview.panelEmpty")}</p>
        <p className="challenge-review-panel__hint challenge-review-panel__hint--compact">
          {t("challengeReview.panelEmptySecondary")}
        </p>
        <div className="challenge-review-panel__footer-actions">
          <button type="button" className="challenge-review-panel__linkish" onClick={() => dispatchOpenAiSettings()}>
            {t("challengeReview.openAiSettings")}
          </button>
        </div>
      </div>
    );
  }

  // 浅档仅禁用通道二；通道一仍可回顾（与迭代 4 文档 §14 自测一致）

  if (!currentItem) {
    return (
      <div className="challenge-review-panel">
        <p className="challenge-review-panel__hint">{t("challengeReview.panelNoMore")}</p>
        <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
          {t("challengeReview.endReview")}
        </button>
      </div>
    );
  }

  return (
    <div className="challenge-review-panel">
      <div className="challenge-review-panel__header">
        <span>{t("challengeReview.panelTitle")}</span>
        <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
          {t("challengeReview.endReview")}
        </button>
      </div>

      {statsBlock(queue)}

      {queue && queue.totalDue > items.length ? (
        <p className="challenge-review-panel__batch-hint">
          {t("challengeReview.panelDailyBatchHint", {
            shown: items.length,
            total: queue.totalDue,
          })}
        </p>
      ) : null}

      <div className="challenge-review-panel__queue" role="region" aria-label={t("challengeReview.panelBatchListTitle", { count: items.length })}>
        <div className="challenge-review-panel__queue-title">{t("challengeReview.panelBatchListTitle", { count: items.length })}</div>
        <ul className="challenge-review-panel__queue-list" role="list">
          {items.map((it, i) => (
            <li key={it.candidateId || it.thoughtId || `item-${i}`} className="challenge-review-panel__queue-li">
              <button
                type="button"
                className={`challenge-review-panel__queue-row${i === cursor ? " is-active" : ""}`}
                disabled={phase !== "pick" || busy}
                aria-current={i === cursor ? "true" : undefined}
                onClick={() => {
                  if (phase !== "pick" || busy) return;
                  setCursor(i);
                }}
              >
                <div className="challenge-review-panel__queue-row-top">
                  <span className="challenge-review-panel__queue-idx">{i + 1}</span>
                  {it.sourceType === "candidate" ? (
                    <span className="challenge-review-panel__candidate-tag">
                      {t("challengeReview.candidateLabel", { file: displayName(it.relPath) })}
                    </span>
                  ) : (
                    <span className="challenge-review-panel__queue-path" title={it.relPath}>
                      {it.relPath}
                    </span>
                  )}
                  {it.sourceType !== "candidate" ? (
                    <span className="challenge-review-panel__queue-due">
                      {t("challengeReview.dueLabel", { days: it.overdueDays })}
                    </span>
                  ) : null}
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

      {phase === "pick" && items.length > 1 ? (
        <p className="challenge-review-panel__queue-pick-hint">{t("challengeReview.panelBatchListPickHint")}</p>
      ) : null}

      {phase === "pick" ? (
        <>
          <div className="challenge-review-panel__meta">
            <span>{currentItem.sourceType === "candidate"
              ? t("challengeReview.candidateLabel", { file: displayName(currentItem.relPath) })
              : currentItem.relPath}</span>
            {currentItem.sourceType !== "candidate" ? (
              <span className="challenge-review-panel__due">
                {t("challengeReview.dueLabel", { days: currentItem.overdueDays })}
              </span>
            ) : null}
          </div>
          {currentItem.sourceType !== "candidate" ? (
            <div className="challenge-review-panel__created">
              {t("challengeReview.createdLabel", { time: createdDisplay(currentItem.created) })}
            </div>
          ) : currentItem.markingReason ? (
            <>
              <div className="challenge-review-panel__created">
                {currentItem.markingReason === "high_similarity"
                  ? t("challengeReview.reasonHighSimilarity")
                  : currentItem.markingReason === "semantic_isolated"
                  ? t("challengeReview.reasonSemanticIsolated")
                  : currentItem.markingReason === "cross_doc_recurrence"
                  ? t("challengeReview.reasonCrossDocRecurrence")
                  : currentItem.markingReason}
              </div>
              {currentItem.pairedExcerpt ? (
                <div className="challenge-review-panel__related-docs">
                  {t("challengeReview.relatedDocs")}
                  {currentItem.pairedExcerpt.split(",").map((p) => (
                    <span key={p} className="challenge-review-panel__related-doc-tag">
                      {displayName(p.trim())}
                    </span>
                  ))}
                </div>
              ) : null}
            </>
          ) : null}
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
            {currentItem.sourceType === "candidate" && currentItem.candidateId ? (
              <button
                type="button"
                className="challenge-review-panel__btn"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  invoke("dismiss_latent_candidate", { candidateId: currentItem.candidateId })
                    .catch(() => {})
                    .finally(() => {
                      questionCacheRef.current.clear();
                      inflightRef.current.clear();
                      void reloadQueue().then((q) => {
                        if (!q || q.items.length === 0) {
                          setCursor(0);
                        } else {
                          const next = Math.min(cursor, q.items.length - 1);
                          setCursor(next);
                          prefetchQuestion(q.items, next);
                        }
                        setBusy(false);
                      });
                    });
                }}
              >
                {t("challengeReview.skipItem")}
              </button>
            ) : items.length > 1 ? (
              <button
                type="button"
                className="challenge-review-panel__btn"
                disabled={busy}
                onClick={() => setCursor((c) => (c + 1) % items.length)}
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

      {phase === "qa" ? (
        <>
          <div className="challenge-review-panel__question">{question}</div>
          <textarea
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
              onClick={() => {
                setPhase("pick");
                setQuestion("");
                setAnswer("");
                setEvalRes(null);
              }}
            >
              {t("challengeReview.abandon")}
            </button>
          </div>
        </>
      ) : null}

      {phase === "result" && evalRes ? (
        <div className="challenge-review-panel__result">
          {evalRes.sloppy ? <p className="challenge-review-panel__sloppy">{t("challengeReview.sloppyHint")}</p> : null}
          {evalRes.passed && currentItem?.sourceType !== "candidate" ? (
            <>
              <p className="challenge-review-panel__pass">{t("challengeReview.passed")}</p>
              {!evalRes.sloppy ? (
                <p className="challenge-review-panel__maturity">{t("challengeReview.maturitySignalHint")}</p>
              ) : null}
            </>
          ) : null}
          <AiAssistantMarkdown content={evalRes.commentaryMd} className="challenge-review-panel__md" />
          {currentItem?.sourceType === "candidate" && currentItem.candidateId ? (
            <CandidatePromoteCard
              candidateId={currentItem.candidateId}
              onDone={() => void goNext().catch(() => {})}
            />
          ) : (
            <>
              <ChallengeFeedbackBar
                thoughtId={currentItem?.thoughtId}
                questionText={question}
                questionTemplate={templateKind}
              />
              <div className="challenge-review-panel__actions">
                <button type="button" className="challenge-review-panel__btn" onClick={() => void goNext().catch(() => {})}>
                  {t("challengeReview.continueNext")}
                </button>
                <button type="button" className="challenge-review-panel__linkish" onClick={onClose}>
                  {t("challengeReview.endReview")}
                </button>
              </div>
            </>
          )}
        </div>
      ) : null}
    </div>
  );
}
