/** 迭代 3 认知内核共享类型 —— 与 Rust vault_config / thought_parser 的 JSON (camelCase) 对齐 */

// --- 深度模式 ---

export type DepthMode = "shallow" | "medium" | "deep" | "auto";
export type AutoResolvedDepth = "shallow" | "medium" | "deep";

// --- 认知配置（对齐 CognitiveConfigForUi） ---

export type InviteStatsForUi = {
  consecutiveEnoughCount: number;
  acceptanceWindow: boolean[];
  lastEnoughTimestamp?: string;
};

/** 与 Rust `PassiveHighlightInaccuracyCountsForUi` 对齐（camelCase） */
export type PassiveHighlightInaccuracyCountsForUi = {
  integrate: number;
  correct: number;
  crossDomain: number;
};

/** 与 Rust `ChallengeReviewDayStats` 对齐 */
export type ChallengeReviewDayStatsForUi = {
  inlineShown: number;
  thoughtIdsInline: string[];
  independentShown: number;
  thoughtIdsIndependent: string[];
};

/** 与 Rust `ChallengeReviewInlineDates` 对齐（按日历日聚合） */
export type ChallengeReviewInlineDatesForUi = {
  byDay: Record<string, ChallengeReviewDayStatsForUi>;
};

export type CognitiveConfigForUi = {
  depthMode: DepthMode;
  autoManualOverrideUntil?: string;
  inviteStats: InviteStatsForUi;
  snoozeInvitesUntil?: string;
  passiveHighlightEnabled: boolean;
  passiveHighlightConfidenceMin: number;
  passiveHighlightInaccuracyCounts: PassiveHighlightInaccuracyCountsForUi;
  /** 通道一：独立回顾总开关，默认关闭 */
  independentReviewEnabled: boolean;
  challengeReviewDailyCapIndependent: number;
  challengeReviewDailyCapInline: number;
  challengeReviewInlineDates: ChallengeReviewInlineDatesForUi;
  /** 写作教练总开关，默认 true */
  writingCoachEnabled: boolean;
  /** ISO 8601，冷却期内不触发气泡 */
  writingCoachCooldownUntil?: string;
  /** 无编辑停顿秒数，达到后才检测触发（默认 15） */
  writingCoachIdleSeconds: number;
  /** 深度写作：当前块最少 Unicode 码点（默认 500） */
  writingCoachDepthMinChars: number;
  /** 术语密度检测：段落最少码点（默认 36） */
  writingCoachTermMinChars: number;
  /** 气泡无点击自动消失秒数（默认 30） */
  writingCoachBubbleSeconds: number;
  /** 忽略气泡后的冷却分钟数（默认 15） */
  writingCoachCooldownMinutes: number;
};

// --- 认知配置保存载荷（对齐 CognitiveConfigPatch） ---

export type InviteStatsSavePatch = {
  consecutiveEnoughCount?: number;
  acceptanceWindow?: boolean[];
  lastEnoughTimestamp?: string | null;
};

export type CognitiveConfigSavePatch = {
  depthMode?: DepthMode;
  autoManualOverrideUntil?: string | null;
  inviteStats?: InviteStatsSavePatch;
  snoozeInvitesUntil?: string | null;
  passiveHighlightEnabled?: boolean;
  passiveHighlightConfidenceMin?: number;
  independentReviewEnabled?: boolean;
  challengeReviewDailyCapIndependent?: number;
  challengeReviewDailyCapInline?: number;
  challengeReviewInlineDates?: ChallengeReviewInlineDatesForUi;
  writingCoachEnabled?: boolean;
  writingCoachCooldownUntil?: string | null;
  writingCoachIdleSeconds?: number;
  writingCoachDepthMinChars?: number;
  writingCoachTermMinChars?: number;
  writingCoachBubbleSeconds?: number;
  writingCoachCooldownMinutes?: number;
};

// --- 理解区块（解析结果） ---

export type ThoughtMaturity = "seedling" | "growing" | "mature";

export type ThoughtBlockParsed = {
  id: string;
  maturity: ThoughtMaturity;
  excerpt: string;
  startLine: number;
  endLine: number;
  temporary: boolean;
};

// --- 理解检索结果 ---

export type ThoughtRetrievalResult = {
  relPath: string;
  thoughtId: string;
  excerpt: string;
  maturity: ThoughtMaturity;
  score: number;
  privateOmitted?: boolean;
};

/** 与 Rust `thought_retrieval::SearchThoughtMeta` 对齐（camelCase） */
export type SearchThoughtMetaForUi = {
  scannedFiles: number;
  stoppedEarly: boolean;
  elapsedMs: number;
  /** 非空表示侧车检索未正常执行（如 `sidecar_unavailable`），与「查过但无命中」区分 */
  errorCode?: string;
};

export type ThoughtRetrievalResponse = {
  thought: ThoughtRetrievalResult | null;
  /** 多候选检索结果（按 score 降序）；兼容旧客户端可只读 `thought` */
  thoughts?: ThoughtRetrievalResult[];
  meta: SearchThoughtMetaForUi;
};

// --- 邀请区载荷 ---

export type InvitePayload = {
  kind: "withThought" | "openEnded";
  thought?: ThoughtRetrievalResult;
  question: string;
};

// --- IPC 请求/响应类型（对齐 thought_parser.rs） ---

/** kf-thoughts `history[].type`；盘外 YAML 可能出现其它字符串，IPC 仍可能带回，此处收窄常见写入值 */
export type KfThoughtHistoryEntryType =
  | "created"
  | "substantial-change"
  | "challenge-review-pass";

export type KfThoughtHistoryEntry = {
  date: string;
  type: KfThoughtHistoryEntryType;
  source: string;
  diffSummary?: string;
};

export type KfThoughtReference = {
  date: string;
  context: string;
  relevance: string;
};

export type KfThoughtMeta = {
  id: string;
  maturity: ThoughtMaturity;
  created: string;
  updated: string;
  temporary: boolean;
  /** 挑战式回顾通过次数 */
  challengePassCount?: number;
  /** 上次成功回顾时间 ISO8601 */
  lastReviewedAt?: string;
  history: KfThoughtHistoryEntry[];
  references: KfThoughtReference[];
};

export type ParseNoteThoughtsResponse = {
  blocks: ThoughtBlockParsed[];
  meta: KfThoughtMeta[];
  /** 解析 kf-thoughts YAML 时的非致命告警（如结构错误）；无问题时可省略 */
  yamlWarnings?: string[];
};

export type InsertThoughtArgs = {
  relPath: string;
  content?: string;
  temporary?: boolean;
  afterLine?: number;
};

export type InsertThoughtResponse = {
  thoughtId: string;
  insertedAtLine: number;
};

/** `list_vault_thoughts` 单行（camelCase 与 Rust `VaultThoughtListRow` 对齐；maturity 盘内应为三态之一） */
export type VaultThoughtListRow = {
  relPath: string;
  thoughtId: string;
  excerpt: string;
  maturity: ThoughtMaturity;
  temporary: boolean;
  standalone: boolean;
  updatedAt: string;
};

/** `list_vault_thoughts` 分页响应（与 Rust `VaultThoughtListPage` 对齐） */
export type VaultThoughtListPage = {
  rows: VaultThoughtListRow[];
  total: number;
};

/** `get_thought_detail` 响应（camelCase 与 Rust `ThoughtDetail` 对齐） */
export type ThoughtDetail = {
  thoughtId: string;
  noteStableId: string;
  noteRelPath: string;
  body: string;
  summary: string | null;
  maturity: string;
  temporary: boolean;
  standalone: boolean;
  createdAt: string;
  updatedAt: string;
  challengePassCount: number;
  lastReviewedAt: string | null;
};

/** `generate_challenge_question` 请求（camelCase 与 Rust 对齐） */
export type GenerateChallengeQuestionArgs = {
  thoughtExcerpt: string;
  relPath: string;
  conversationQuery?: string;
  depthMode?: DepthMode;
  /** 与 Knowforge 设置一致：`en` | `zh`，驱动模型输出自然语言 */
  uiLocale?: "en" | "zh";
};

/** `evaluate_challenge_answer` 请求 */
export type EvaluateChallengeAnswerArgs = {
  question: string;
  userAnswer: string;
  thoughtExcerpt: string;
  depthMode?: DepthMode;
  uiLocale?: "en" | "zh";
};

/** `generate_challenge_question` 响应 */
export type GenerateChallengeQuestionResponse = {
  question: string;
  templateKind: string;
  degraded: boolean;
  shouldSkip: boolean;
};

/** `evaluate_challenge_answer` 响应 */
export type EvaluateChallengeAnswerResponse = {
  passed: boolean;
  sloppy: boolean;
  commentaryMd: string;
  templateKind?: string;
};

/** `get_feedback_stats` 响应 */
export type FeedbackTemplateStats = {
  template: string;
  total: number;
  helpful: number;
  notHelpful: number;
  helpfulRate: number;
};

export type FeedbackIssueCount = {
  reason: string;
  count: number;
};

export type FeedbackStats = {
  totalRatings: number;
  helpfulCount: number;
  notHelpfulCount: number;
  helpfulRate: number;
  byTemplate: FeedbackTemplateStats[];
  commonIssues: FeedbackIssueCount[];
};

/** `count_vault_thoughts_for_review` 响应 */
export type CountVaultThoughtsForReviewResponse = {
  totalThoughts: number;
  meta: SearchThoughtMetaForUi;
};

/** `list_review_queue` 单条 */
export type ReviewQueueItem = {
  relPath: string;
  thoughtId: string;
  excerpt: string;
  maturity: ThoughtMaturity;
  created: string;
  lastReviewedAt?: string;
  challengePassCount: number;
  nextDueAt: string;
  overdueDays: number;
  privateOmitted: boolean;
};

export type ListReviewQueueResponse = {
  items: ReviewQueueItem[];
  totalThoughts: number;
  totalDue: number;
  meta: SearchThoughtMetaForUi;
};
