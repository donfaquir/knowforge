/**
 * 邀请与挑战式回顾频控 Hook —— 管理 invite-after-answer 与通道二内联回顾的展示规则与持久化。
 *
 * 邀请频控（对照母文档 §5.D）：
 * - 单次"够了" -> 本会话不再展示（enoughForThisChat，由 SessionContext 管理）
 * - 连续 3 次 -> 每 3 轮展示
 * - 连续 5 次 -> 每 10 轮展示
 * - 接受率 < 15%（最近 30 次） -> 最低频
 * - snooze -> 到期前不展示
 *
 * 挑战回顾（迭代 4）：与邀请互斥由调用方组合；此处提供 cap 与同日 thought 去重。
 */
import { useCallback, useMemo, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  AutoResolvedDepth,
  ChallengeReviewDayStatsForUi,
  ChallengeReviewInlineDatesForUi,
  DepthMode,
  InviteStatsForUi,
  CognitiveConfigForUi,
} from "../types/cognitiveTypes";
import type { VaultConfigForUi, VaultConfigSavePatch } from "../types/vaultAiConfig";

// ---- 内部常量 ----

/** 连续"够了" >= 3 时，每 N 轮才展示 */
const INTERVAL_3 = 3;
/** 连续"够了" >= 5 时，每 N 轮才展示 */
const INTERVAL_5 = 10;
/** 接受率窗口大小 */
const ACCEPTANCE_WINDOW_SIZE = 30;
/** 接受率低于该阈值视为最低频 */
const LOW_ACCEPTANCE_THRESHOLD = 0.15;

// ---- 工具函数 ----

function acceptanceRate(window: boolean[]): number {
  if (window.length === 0) return 1; // 无历史，视为高接受率
  const accepted = window.filter(Boolean).length;
  return accepted / window.length;
}

function pushWindow(window: boolean[], value: boolean): boolean[] {
  const next = [...window, value];
  if (next.length > ACCEPTANCE_WINDOW_SIZE) {
    return next.slice(next.length - ACCEPTANCE_WINDOW_SIZE);
  }
  return next;
}

/** 本地日历日 `YYYY-MM-DD`（与 Rust 写回 `lastReviewedAt` 日期口径一致） */
export function localTodayKey(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function emptyDayStats(): ChallengeReviewDayStatsForUi {
  return {
    inlineShown: 0,
    thoughtIdsInline: [],
    independentShown: 0,
    thoughtIdsIndependent: [],
  };
}

// ---- 主 Hook ----

export type ChallengeInlineGateInput = {
  /** 若本轮将展示先答后邀，则禁止通道二 */
  inviteWillShow: boolean;
  /** 当前候选 thought id；未知时可不传，仅校验 cap */
  thoughtId?: string | null;
  /** Vault 内可排期 thought 总数（低于 3 不展示独立入口；内联可共用阈值） */
  vaultThoughtTotal: number;
};

export type CognitiveFrequencyControl = {
  shouldShowInvite: (
    depthMode: DepthMode,
    enoughForThisChat: boolean,
    turnIndex: number,
    autoResolved: AutoResolvedDepth | null,
  ) => boolean;
  /**
   * 是否允许在对话末尾尝试展示挑战式回顾（仍须调用方检索/LLM 成功后再渲染）。
   * 与 `shouldShowInvite` 组合：先算邀请，若邀请将展示则不应为 true。
   */
  shouldShowChallengeInline: (
    depthMode: DepthMode,
    autoResolved: AutoResolvedDepth | null,
    gate: ChallengeInlineGateInput,
  ) => boolean;
  recordEnough: () => Promise<void>;
  recordAccepted: () => Promise<void>;
  snoozeInvites: (days: number) => Promise<void>;
  isInSnooze: () => boolean;
  reload: () => Promise<void>;
  /** 通道二成功展示或用户进入作答流时调用，计入当日 cap 并防同日重复 */
  recordChallengeInlineShown: (thoughtId: string) => Promise<void>;
  /** 通道一完成一条回顾时调用 */
  recordChallengeIndependentShown: (thoughtId: string) => Promise<void>;
  /** 当日成功完成的独立回顾是否仍低于 cap（用于通道一门控） */
  canStartMoreIndependentReviewsToday: () => boolean;
  setIndependentReviewEnabled: (enabled: boolean) => Promise<void>;
  independentReviewEnabled: boolean;
  challengeReviewDailyCapIndependent: number;
  challengeReviewDailyCapInline: number;
  challengeReviewInlineDates: ChallengeReviewInlineDatesForUi;
};

export function useCognitiveFrequencyControl(): CognitiveFrequencyControl {
  const [stats, setStats] = useState<InviteStatsForUi>({
    consecutiveEnoughCount: 0,
    acceptanceWindow: [],
  });
  const [snoozeUntil, setSnoozeUntil] = useState<string | undefined>(undefined);
  const [independentReviewEnabled, setIndependentReviewEnabledState] = useState(false);
  const [challengeReviewDailyCapIndependent, setChallengeReviewDailyCapIndependent] = useState(3);
  const [challengeReviewDailyCapInline, setChallengeReviewDailyCapInline] = useState(3);
  const [challengeReviewInlineDates, setChallengeReviewInlineDates] =
    useState<ChallengeReviewInlineDatesForUi>({ byDay: {} });

  const statsRef = useRef(stats);
  statsRef.current = stats;
  const snoozeRef = useRef(snoozeUntil);
  snoozeRef.current = snoozeUntil;
  const challengeDatesRef = useRef(challengeReviewInlineDates);
  challengeDatesRef.current = challengeReviewInlineDates;
  const capIndRef = useRef(challengeReviewDailyCapIndependent);
  capIndRef.current = challengeReviewDailyCapIndependent;
  const capInlineRef = useRef(challengeReviewDailyCapInline);
  capInlineRef.current = challengeReviewDailyCapInline;

  const saveStats = useCallback(async (next: InviteStatsForUi) => {
    setStats(next);
    statsRef.current = next;
    if (isTauri()) {
      const patch: VaultConfigSavePatch = {
        cognitive: {
          inviteStats: {
            consecutiveEnoughCount: next.consecutiveEnoughCount,
            acceptanceWindow: next.acceptanceWindow,
            lastEnoughTimestamp: next.lastEnoughTimestamp ?? null,
          },
        },
      };
      await invoke("save_vault_config_patch", { patch }).catch(() => {});
    }
  }, []);

  const saveSnooze = useCallback(async (until: string | null) => {
    setSnoozeUntil(until ?? undefined);
    snoozeRef.current = until ?? undefined;
    if (isTauri()) {
      const patch: VaultConfigSavePatch = {
        cognitive: { snoozeInvitesUntil: until },
      };
      await invoke("save_vault_config_patch", { patch }).catch(() => {});
    }
  }, []);

  const saveChallengeDates = useCallback(async (next: ChallengeReviewInlineDatesForUi) => {
    setChallengeReviewInlineDates(next);
    challengeDatesRef.current = next;
    if (isTauri()) {
      const patch: VaultConfigSavePatch = {
        cognitive: { challengeReviewInlineDates: next },
      };
      await invoke("save_vault_config_patch", { patch }).catch(() => {});
    }
  }, []);

  const reload = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const cfg = await invoke<VaultConfigForUi>("get_vault_config_for_ui");
      const c: CognitiveConfigForUi = cfg.cognitive;
      setStats(c.inviteStats);
      statsRef.current = c.inviteStats;
      setSnoozeUntil(c.snoozeInvitesUntil);
      snoozeRef.current = c.snoozeInvitesUntil;
      setIndependentReviewEnabledState(c.independentReviewEnabled ?? false);
      setChallengeReviewDailyCapIndependent(c.challengeReviewDailyCapIndependent ?? 3);
      setChallengeReviewDailyCapInline(c.challengeReviewDailyCapInline ?? 2);
      capIndRef.current = c.challengeReviewDailyCapIndependent ?? 3;
      capInlineRef.current = c.challengeReviewDailyCapInline ?? 2;
      const dates = c.challengeReviewInlineDates ?? { byDay: {} };
      setChallengeReviewInlineDates(dates);
      challengeDatesRef.current = dates;
    } catch {
      // 忽略：config 不可用时保留默认值
    }
  }, []);

  const isInSnooze = useCallback((): boolean => {
    const until = snoozeRef.current;
    if (!until) return false;
    return new Date(until).getTime() > Date.now();
  }, []);

  const shouldShowInvite = useCallback(
    (
      depthMode: DepthMode,
      enoughForThisChat: boolean,
      turnIndex: number,
      autoResolved: AutoResolvedDepth | null,
    ): boolean => {
      if (depthMode === "shallow") return false;
      if (depthMode === "auto" && autoResolved === "shallow") return false;
      if (enoughForThisChat) return false;
      if (isInSnooze()) return false;

      const s = statsRef.current;

      if (
        s.acceptanceWindow.length >= 10 &&
        acceptanceRate(s.acceptanceWindow) < LOW_ACCEPTANCE_THRESHOLD
      ) {
        return turnIndex % INTERVAL_5 === 0;
      }

      if (s.consecutiveEnoughCount >= 5) {
        return turnIndex % INTERVAL_5 === 0;
      }

      if (s.consecutiveEnoughCount >= 3) {
        return turnIndex % INTERVAL_3 === 0;
      }

      return true;
    },
    [isInSnooze],
  );

  const shouldShowChallengeInline = useCallback(
    (depthMode: DepthMode, autoResolved: AutoResolvedDepth | null, gate: ChallengeInlineGateInput) => {
      if (gate.inviteWillShow) return false;
      if (depthMode === "shallow" || (depthMode === "auto" && autoResolved === "shallow")) {
        return false;
      }
      if (gate.vaultThoughtTotal < 3) return false;

      const day = localTodayKey();
      const st = challengeDatesRef.current.byDay[day] ?? emptyDayStats();
      if (st.inlineShown >= capInlineRef.current) return false;
      if (gate.thoughtId && st.thoughtIdsInline.includes(gate.thoughtId)) return false;
      return true;
    },
    [],
  );

  const recordEnough = useCallback(async () => {
    const s = statsRef.current;
    const next: InviteStatsForUi = {
      consecutiveEnoughCount: s.consecutiveEnoughCount + 1,
      acceptanceWindow: pushWindow(s.acceptanceWindow, false),
      lastEnoughTimestamp: new Date().toISOString(),
    };
    await saveStats(next);
  }, [saveStats]);

  const recordAccepted = useCallback(async () => {
    const s = statsRef.current;
    const next: InviteStatsForUi = {
      consecutiveEnoughCount: 0,
      acceptanceWindow: pushWindow(s.acceptanceWindow, true),
      lastEnoughTimestamp: s.lastEnoughTimestamp,
    };
    await saveStats(next);
  }, [saveStats]);

  const snoozeInvites = useCallback(
    async (days: number) => {
      const until = new Date(Date.now() + days * 24 * 60 * 60 * 1000).toISOString();
      await saveSnooze(until);
    },
    [saveSnooze],
  );

  const recordChallengeInlineShown = useCallback(
    async (thoughtId: string) => {
      const day = localTodayKey();
      const cur: ChallengeReviewInlineDatesForUi = {
        byDay: { ...challengeDatesRef.current.byDay },
      };
      const prev = cur.byDay[day] ?? emptyDayStats();
      cur.byDay[day] = {
        ...prev,
        inlineShown: prev.inlineShown + 1,
        thoughtIdsInline: prev.thoughtIdsInline.includes(thoughtId)
          ? prev.thoughtIdsInline
          : [...prev.thoughtIdsInline, thoughtId],
        independentShown: prev.independentShown,
        thoughtIdsIndependent: [...prev.thoughtIdsIndependent],
      };
      await saveChallengeDates(cur);
    },
    [saveChallengeDates],
  );

  const recordChallengeIndependentShown = useCallback(
    async (thoughtId: string) => {
      const day = localTodayKey();
      const cur: ChallengeReviewInlineDatesForUi = {
        byDay: { ...challengeDatesRef.current.byDay },
      };
      const prev = cur.byDay[day] ?? emptyDayStats();
      cur.byDay[day] = {
        ...prev,
        independentShown: prev.independentShown + 1,
        thoughtIdsIndependent: prev.thoughtIdsIndependent.includes(thoughtId)
          ? prev.thoughtIdsIndependent
          : [...prev.thoughtIdsIndependent, thoughtId],
        inlineShown: prev.inlineShown,
        thoughtIdsInline: [...prev.thoughtIdsInline],
      };
      await saveChallengeDates(cur);
    },
    [saveChallengeDates],
  );

  const setIndependentReviewEnabled = useCallback(async (enabled: boolean) => {
    setIndependentReviewEnabledState(enabled);
    if (isTauri()) {
      const patch: VaultConfigSavePatch = {
        cognitive: { independentReviewEnabled: enabled },
      };
      await invoke("save_vault_config_patch", { patch }).catch(() => {});
    }
  }, []);

  const canStartMoreIndependentReviewsToday = useCallback((): boolean => {
    const day = localTodayKey();
    const st = challengeDatesRef.current.byDay[day] ?? emptyDayStats();
    return st.independentShown < capIndRef.current;
  }, []);

  return useMemo(
    () => ({
      shouldShowInvite,
      shouldShowChallengeInline,
      recordEnough,
      recordAccepted,
      snoozeInvites,
      isInSnooze,
      reload,
      recordChallengeInlineShown,
      recordChallengeIndependentShown,
      canStartMoreIndependentReviewsToday,
      setIndependentReviewEnabled,
      independentReviewEnabled,
      challengeReviewDailyCapIndependent,
      challengeReviewDailyCapInline,
      challengeReviewInlineDates,
    }),
    [
      shouldShowInvite,
      shouldShowChallengeInline,
      recordEnough,
      recordAccepted,
      snoozeInvites,
      isInSnooze,
      reload,
      recordChallengeInlineShown,
      recordChallengeIndependentShown,
      canStartMoreIndependentReviewsToday,
      setIndependentReviewEnabled,
      independentReviewEnabled,
      challengeReviewDailyCapIndependent,
      challengeReviewDailyCapInline,
      challengeReviewInlineDates,
    ],
  );
}
