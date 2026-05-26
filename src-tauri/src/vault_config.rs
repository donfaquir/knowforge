//! Vault 根目录下 `.knowforge/config.json`：AI 配置读写、合并默认值、脱敏 IPC。
//! 不在日志中输出 OpenAI apiKey。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

// --- 常量 ---

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

// --- 语义索引（迭代 6.2）---

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticConfig {
    #[serde(default = "default_semantic_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default = "default_auto_index_on_save")]
    pub auto_index_on_save: bool,
    #[serde(default = "default_semantic_search_weight")]
    pub search_weight: f64,
}

fn default_semantic_enabled() -> bool {
    true
}

fn default_auto_index_on_save() -> bool {
    true
}

fn default_semantic_search_weight() -> f64 {
    0.6
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: default_semantic_enabled(),
            embedding_model: None,
            auto_index_on_save: default_auto_index_on_save(),
            search_weight: default_semantic_search_weight(),
        }
    }
}

fn normalize_semantic(cfg: &mut SemanticConfig) {
    cfg.search_weight = cfg.search_weight.clamp(0.0, 1.0);
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SemanticDiskPartial {
    enabled: Option<bool>,
    embedding_model: Option<String>,
    auto_index_on_save: Option<bool>,
    search_weight: Option<f64>,
}

fn merge_semantic_from_disk_partial(mut cfg: SemanticConfig, partial: SemanticDiskPartial) -> SemanticConfig {
    if let Some(v) = partial.enabled {
        cfg.enabled = v;
    }
    if partial.embedding_model.is_some() {
        cfg.embedding_model = partial.embedding_model;
    }
    if let Some(v) = partial.auto_index_on_save {
        cfg.auto_index_on_save = v;
    }
    if let Some(w) = partial.search_weight {
        cfg.search_weight = w;
    }
    normalize_semantic(&mut cfg);
    cfg
}

fn load_merged_semantic(v: &Value) -> SemanticConfig {
    let s = v.get("semantic").cloned().unwrap_or(json!({}));
    let partial: SemanticDiskPartial = serde_json::from_value(s).unwrap_or_default();
    merge_semantic_from_disk_partial(SemanticConfig::default(), partial)
}

/// 供语义索引 / LLM 读取合并后的语义配置
pub fn load_semantic_merged(root: &Path) -> Result<SemanticConfig, String> {
    let path = config_path(root);
    let v = read_root_value(&path)?;
    Ok(load_merged_semantic(&v))
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SemanticConfigPatch {
    pub enabled: Option<bool>,
    pub embedding_model: Option<Option<String>>,
    pub auto_index_on_save: Option<bool>,
    pub search_weight: Option<f64>,
}

fn apply_semantic_patch(cfg: &mut SemanticConfig, patch: SemanticConfigPatch) {
    if let Some(v) = patch.enabled {
        cfg.enabled = v;
    }
    if let Some(m) = patch.embedding_model {
        cfg.embedding_model = m;
    }
    if let Some(v) = patch.auto_index_on_save {
        cfg.auto_index_on_save = v;
    }
    if let Some(w) = patch.search_weight {
        cfg.search_weight = w;
    }
}
const KNOWFORGE_DIR: &str = ".knowforge";
const CONFIG_FILE: &str = "config.json";

const DEFAULT_OLLAMA_BASE: &str = "http://127.0.0.1:11434";
const DEFAULT_OPENAI_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

const TIMEOUT_MS_MIN: u64 = 1_000;
const TIMEOUT_MS_MAX: u64 = 600_000;
const TEMP_MIN: f64 = 0.0;
const TEMP_MAX: f64 = 2.0;
const TOP_P_MIN: f64 = 0.0;
const TOP_P_MAX: f64 = 1.0;

// --- 内部完整配置（合并后） ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ActiveProvider {
    Ollama,
    Openai,
}

impl Default for ActiveProvider {
    fn default() -> Self {
        Self::Ollama
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OllamaProfile {
    pub base_url: String,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatibleProfile {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_model: Option<String>,
}

impl Default for OpenAiCompatibleProfile {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_OPENAI_BASE.to_string(),
            api_key: String::new(),
            default_model: DEFAULT_OPENAI_MODEL.to_string(),
            organization_id: None,
            last_used_model: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u64>,
}

fn default_timeout_ms() -> u64 {
    120_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiParameters {
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
}

fn default_temperature() -> f64 {
    0.7
}

impl Default for AiParameters {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            top_p: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPrivacy {
    #[serde(default)]
    pub allow_private_content_in_local_llm: bool,
}

impl Default for AiPrivacy {
    fn default() -> Self {
        Self {
            allow_private_content_in_local_llm: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    #[serde(default)]
    pub active_provider: ActiveProvider,
    #[serde(default)]
    pub ollama: OllamaProfile,
    #[serde(default)]
    pub openai_compatible: OpenAiCompatibleProfile,
    #[serde(default)]
    pub request: AiRequest,
    #[serde(default)]
    pub parameters: AiParameters,
    #[serde(default)]
    pub privacy: AiPrivacy,
    /// Iter 5 #4: 主对话工具调用总开关。默认 true,使内置 skills (`skill.<id>`) 与
    /// 其它工具对主 LLM 可见。旧 vault 缺该字段时通过 disk partial 显式默认 true。
    #[serde(default = "default_tools_enabled")]
    pub tools_enabled: bool,
}

fn default_tools_enabled() -> bool {
    true
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            active_provider: ActiveProvider::Ollama,
            ollama: OllamaProfile {
                base_url: DEFAULT_OLLAMA_BASE.to_string(),
                default_model: String::new(),
                last_used_model: None,
            },
            openai_compatible: OpenAiCompatibleProfile::default(),
            request: AiRequest {
                timeout_ms: default_timeout_ms(),
                max_context_tokens: None,
            },
            parameters: AiParameters::default(),
            privacy: AiPrivacy::default(),
            tools_enabled: true,
        }
    }
}

// --- 认知配置（深度控制、邀请频控） ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DepthMode {
    Shallow,
    Medium,
    Deep,
    Auto,
}

impl Default for DepthMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteStats {
    #[serde(default)]
    pub consecutive_enough_count: u32,
    /// 最近 30 次邀请的接受/拒绝记录（true=接受）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_window: Vec<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enough_timestamp: Option<String>,
}

impl Default for InviteStats {
    fn default() -> Self {
        Self {
            consecutive_enough_count: 0,
            acceptance_window: Vec::new(),
            last_enough_timestamp: None,
        }
    }
}

/// 被动高亮「不准确」按 kind 累计（用于动态抬高置信度门槛）
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PassiveHighlightInaccuracyCounts {
    #[serde(default)]
    pub integrate: u32,
    #[serde(default)]
    pub correct: u32,
    #[serde(default)]
    pub cross_domain: u32,
}

/// 单日挑战回顾频控（按通道分别计数 + 已触发的 thought id，防同日重复）
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeReviewDayStats {
    #[serde(default)]
    pub inline_shown: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thought_ids_inline: Vec<String>,
    #[serde(default)]
    pub independent_shown: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thought_ids_independent: Vec<String>,
}

/// `YYYY-MM-DD` -> 当日统计（迭代 4 挑战回顾双通道）
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeReviewInlineDates {
    #[serde(default)]
    pub by_day: HashMap<String, ChallengeReviewDayStats>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CognitiveConfig {
    #[serde(default)]
    pub depth_mode: DepthMode,
    /// 自愈：自动档被手动覆盖过多时，切到手动并记录恢复时间
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_manual_override_until: Option<String>,
    #[serde(default)]
    pub invite_stats: InviteStats,
    /// "最近不需要"到期日期（ISO 8601）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_invites_until: Option<String>,
    /// 被动高亮总开关
    #[serde(default = "default_passive_highlight_enabled")]
    pub passive_highlight_enabled: bool,
    /// 全局置信度下限 \([0,1]\)
    #[serde(default = "default_passive_highlight_confidence_min")]
    pub passive_highlight_confidence_min: f64,
    #[serde(default)]
    pub passive_highlight_inaccuracy_counts: PassiveHighlightInaccuracyCounts,
    /// 通道一：独立回顾总开关（默认关）
    #[serde(default)]
    pub independent_review_enabled: bool,
    /// 通道一每日 cap（仅展示/调度前 N 条）
    #[serde(default = "default_challenge_review_cap_independent")]
    pub challenge_review_daily_cap_independent: u32,
    /// 通道二每日 cap（对话末尾内联）
    #[serde(default = "default_challenge_review_cap_inline")]
    pub challenge_review_daily_cap_inline: u32,
    #[serde(default)]
    pub challenge_review_inline_dates: ChallengeReviewInlineDates,
    /// 写作教练总开关（默认开）
    #[serde(default = "default_writing_coach_enabled")]
    pub writing_coach_enabled: bool,
    /// 用户忽略气泡后的冷却截止（ISO 8601）；到期前不再触发
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing_coach_cooldown_until: Option<String>,
    /// 无编辑停顿达到该秒数后才允许检测触发（默认 15）
    #[serde(default = "default_writing_coach_idle_seconds")]
    pub writing_coach_idle_seconds: u32,
    /// 深度写作：当前块至少多少 Unicode 码点才满足（默认 500）
    #[serde(default = "default_writing_coach_depth_min_chars")]
    pub writing_coach_depth_min_chars: u32,
    /// 术语密度：参与检测的段落最少码点（默认 36）
    #[serde(default = "default_writing_coach_term_min_chars")]
    pub writing_coach_term_min_chars: u32,
    /// 气泡无点击自动渐隐的秒数（默认 30）
    #[serde(default = "default_writing_coach_bubble_seconds")]
    pub writing_coach_bubble_seconds: u32,
    /// 忽略气泡后的冷却分钟数（默认 15）
    #[serde(default = "default_writing_coach_cooldown_minutes")]
    pub writing_coach_cooldown_minutes: u32,
}

fn default_challenge_review_cap_independent() -> u32 {
    3
}

fn default_challenge_review_cap_inline() -> u32 {
    2
}

fn default_passive_highlight_enabled() -> bool {
    true
}

fn default_passive_highlight_confidence_min() -> f64 {
    0.55
}

fn default_writing_coach_enabled() -> bool {
    true
}

fn default_writing_coach_idle_seconds() -> u32 {
    15
}

fn default_writing_coach_depth_min_chars() -> u32 {
    500
}

fn default_writing_coach_term_min_chars() -> u32 {
    36
}

fn default_writing_coach_bubble_seconds() -> u32 {
    30
}

fn default_writing_coach_cooldown_minutes() -> u32 {
    15
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            depth_mode: DepthMode::Auto,
            auto_manual_override_until: None,
            invite_stats: InviteStats::default(),
            snooze_invites_until: None,
            passive_highlight_enabled: default_passive_highlight_enabled(),
            passive_highlight_confidence_min: default_passive_highlight_confidence_min(),
            passive_highlight_inaccuracy_counts: PassiveHighlightInaccuracyCounts::default(),
            independent_review_enabled: false,
            challenge_review_daily_cap_independent: default_challenge_review_cap_independent(),
            challenge_review_daily_cap_inline: default_challenge_review_cap_inline(),
            challenge_review_inline_dates: ChallengeReviewInlineDates::default(),
            writing_coach_enabled: default_writing_coach_enabled(),
            writing_coach_cooldown_until: None,
            writing_coach_idle_seconds: default_writing_coach_idle_seconds(),
            writing_coach_depth_min_chars: default_writing_coach_depth_min_chars(),
            writing_coach_term_min_chars: default_writing_coach_term_min_chars(),
            writing_coach_bubble_seconds: default_writing_coach_bubble_seconds(),
            writing_coach_cooldown_minutes: default_writing_coach_cooldown_minutes(),
        }
    }
}

// --- 磁盘部分 JSON（仅用于合并缺失字段） ---

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct OllamaPartial {
    base_url: Option<String>,
    default_model: Option<String>,
    last_used_model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct OpenAiPartial {
    base_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    organization_id: Option<String>,
    last_used_model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AiRequestPartial {
    timeout_ms: Option<u64>,
    max_context_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AiParametersPartial {
    temperature: Option<f64>,
    top_p: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AiPrivacyPartial {
    allow_private_content_in_local_llm: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AiDiskPartial {
    active_provider: Option<String>,
    ollama: Option<OllamaPartial>,
    openai_compatible: Option<OpenAiPartial>,
    request: Option<AiRequestPartial>,
    parameters: Option<AiParametersPartial>,
    privacy: Option<AiPrivacyPartial>,
    tools_enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct InviteStatsPartial {
    consecutive_enough_count: Option<u32>,
    acceptance_window: Option<Vec<bool>>,
    last_enough_timestamp: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PassiveHighlightInaccuracyCountsDisk {
    integrate: Option<u32>,
    correct: Option<u32>,
    cross_domain: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CognitiveDiskPartial {
    depth_mode: Option<String>,
    auto_manual_override_until: Option<String>,
    invite_stats: Option<InviteStatsPartial>,
    snooze_invites_until: Option<String>,
    passive_highlight_enabled: Option<bool>,
    passive_highlight_confidence_min: Option<f64>,
    passive_highlight_inaccuracy_counts: Option<PassiveHighlightInaccuracyCountsDisk>,
    independent_review_enabled: Option<bool>,
    challenge_review_daily_cap_independent: Option<u32>,
    challenge_review_daily_cap_inline: Option<u32>,
    challenge_review_inline_dates: Option<ChallengeReviewInlineDates>,
    writing_coach_enabled: Option<bool>,
    writing_coach_cooldown_until: Option<String>,
    writing_coach_idle_seconds: Option<u32>,
    writing_coach_depth_min_chars: Option<u32>,
    writing_coach_term_min_chars: Option<u32>,
    writing_coach_bubble_seconds: Option<u32>,
    writing_coach_cooldown_minutes: Option<u32>,
}

// --- 前端补丁 ---

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VaultConfigPatch {
    pub ai: Option<AiConfigPatch>,
    pub cognitive: Option<CognitiveConfigPatch>,
    pub semantic: Option<SemanticConfigPatch>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiConfigPatch {
    pub active_provider: Option<ActiveProvider>,
    pub ollama: Option<OllamaPatch>,
    pub openai_compatible: Option<OpenAiCompatiblePatch>,
    pub request: Option<AiRequestPatch>,
    pub parameters: Option<AiParametersPatch>,
    pub privacy: Option<AiPrivacyPatch>,
    pub tools_enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPatch {
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub last_used_model: Option<Option<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatiblePatch {
    pub base_url: Option<String>,
    /// `None`：JSON 未传该字段，不修改密钥；`Some(x)`：写入（`""` 表示清除）
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub organization_id: Option<Option<String>>,
    pub last_used_model: Option<Option<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiRequestPatch {
    pub timeout_ms: Option<u64>,
    pub max_context_tokens: Option<Option<u64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiParametersPatch {
    pub temperature: Option<f64>,
    pub top_p: Option<Option<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiPrivacyPatch {
    pub allow_private_content_in_local_llm: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CognitiveConfigPatch {
    pub depth_mode: Option<DepthMode>,
    pub auto_manual_override_until: Option<Option<String>>,
    pub invite_stats: Option<InviteStatsPatch>,
    pub snooze_invites_until: Option<Option<String>>,
    pub passive_highlight_enabled: Option<bool>,
    pub passive_highlight_confidence_min: Option<f64>,
    pub independent_review_enabled: Option<bool>,
    pub challenge_review_daily_cap_independent: Option<u32>,
    pub challenge_review_daily_cap_inline: Option<u32>,
    pub challenge_review_inline_dates: Option<ChallengeReviewInlineDates>,
    pub writing_coach_enabled: Option<bool>,
    pub writing_coach_cooldown_until: Option<Option<String>>,
    pub writing_coach_idle_seconds: Option<u32>,
    pub writing_coach_depth_min_chars: Option<u32>,
    pub writing_coach_term_min_chars: Option<u32>,
    pub writing_coach_bubble_seconds: Option<u32>,
    pub writing_coach_cooldown_minutes: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InviteStatsPatch {
    pub consecutive_enough_count: Option<u32>,
    pub acceptance_window: Option<Vec<bool>>,
    pub last_enough_timestamp: Option<Option<String>>,
}

// --- IPC 脱敏响应 ---

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultConfigForUi {
    #[serde(rename = "$schemaVersion")]
    pub schema_version: u32,
    pub ai: AiConfigForUi,
    pub cognitive: CognitiveConfigForUi,
    pub semantic: SemanticConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassiveHighlightInaccuracyCountsForUi {
    pub integrate: u32,
    pub correct: u32,
    pub cross_domain: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CognitiveConfigForUi {
    pub depth_mode: DepthMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_manual_override_until: Option<String>,
    pub invite_stats: InviteStatsForUi,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snooze_invites_until: Option<String>,
    pub passive_highlight_enabled: bool,
    pub passive_highlight_confidence_min: f64,
    pub passive_highlight_inaccuracy_counts: PassiveHighlightInaccuracyCountsForUi,
    pub independent_review_enabled: bool,
    pub challenge_review_daily_cap_independent: u32,
    pub challenge_review_daily_cap_inline: u32,
    pub challenge_review_inline_dates: ChallengeReviewInlineDates,
    pub writing_coach_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writing_coach_cooldown_until: Option<String>,
    pub writing_coach_idle_seconds: u32,
    pub writing_coach_depth_min_chars: u32,
    pub writing_coach_term_min_chars: u32,
    pub writing_coach_bubble_seconds: u32,
    pub writing_coach_cooldown_minutes: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteStatsForUi {
    pub consecutive_enough_count: u32,
    pub acceptance_window: Vec<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_enough_timestamp: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfigForUi {
    pub active_provider: ActiveProvider,
    pub ollama: OllamaProfile,
    pub openai_compatible: OpenAiCompatibleForUi,
    pub request: AiRequest,
    pub parameters: AiParameters,
    pub privacy: AiPrivacy,
    pub tools_enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatibleForUi {
    pub base_url: String,
    pub api_key_present: bool,
    pub default_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_model: Option<String>,
}

// --- 路径与 I/O ---

fn knowforge_dir(root: &Path) -> PathBuf {
    root.join(KNOWFORGE_DIR)
}

/// `.knowforge/analytics.jsonl` 本地埋点路径
pub fn analytics_jsonl_path(root: &Path) -> PathBuf {
    knowforge_dir(root).join("analytics.jsonl")
}

fn config_path(root: &Path) -> PathBuf {
    knowforge_dir(root).join(CONFIG_FILE)
}

fn backup_corrupt_config(path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("failed to read corrupt config: {e}"))?;
    let parent = path.parent().ok_or_else(|| "config path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("failed to create .knowforge: {e}"))?;
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let bak = parent.join(format!("config.json.broken.{ms}"));
    fs::write(&bak, raw).map_err(|e| format!("failed to write config backup: {e}"))?;
    Ok(())
}

/// 供设置页 / IPC：将 Ollama base URL 规范为合法 origin（非法则回退默认本机地址）
pub fn normalize_ollama_base_url(raw: &str) -> String {
    normalize_http_base(raw, DEFAULT_OLLAMA_BASE)
}

/// 将 baseUrl 规范为 http(s) 的 origin（无路径后缀）；非法则回退 default_url
fn normalize_http_base(raw: &str, default_url: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return default_url.to_string();
    }
    let Ok(u) = Url::parse(t) else {
        return default_url.to_string();
    };
    if u.scheme() != "http" && u.scheme() != "https" {
        return default_url.to_string();
    }
    let origin = u.origin();
    if !origin.is_tuple() {
        return default_url.to_string();
    }
    let s = origin.ascii_serialization();
    if s == "null" || s.is_empty() {
        default_url.to_string()
    } else {
        s
    }
}

fn parse_active_provider(s: &str) -> ActiveProvider {
    match s.to_ascii_lowercase().as_str() {
        "openai" => ActiveProvider::Openai,
        _ => ActiveProvider::Ollama,
    }
}

fn merge_ai_from_disk_partial(mut cfg: AiConfig, partial: AiDiskPartial) -> AiConfig {
    if let Some(s) = partial.active_provider {
        cfg.active_provider = parse_active_provider(&s);
    }
    if let Some(o) = partial.ollama {
        if let Some(u) = o.base_url {
            cfg.ollama.base_url = u;
        }
        if let Some(m) = o.default_model {
            cfg.ollama.default_model = m;
        }
        if o.last_used_model.is_some() {
            cfg.ollama.last_used_model = o.last_used_model;
        }
    }
    if let Some(o) = partial.openai_compatible {
        if let Some(u) = o.base_url {
            cfg.openai_compatible.base_url = u;
        }
        if let Some(k) = o.api_key {
            cfg.openai_compatible.api_key = k;
        }
        if let Some(m) = o.default_model {
            cfg.openai_compatible.default_model = m;
        }
        if o.organization_id.is_some() {
            cfg.openai_compatible.organization_id = o.organization_id;
        }
        if o.last_used_model.is_some() {
            cfg.openai_compatible.last_used_model = o.last_used_model;
        }
    }
    if let Some(r) = partial.request {
        if let Some(t) = r.timeout_ms {
            cfg.request.timeout_ms = t;
        }
        if r.max_context_tokens.is_some() {
            cfg.request.max_context_tokens = r.max_context_tokens;
        }
    }
    if let Some(p) = partial.parameters {
        if let Some(t) = p.temperature {
            cfg.parameters.temperature = t;
        }
        if p.top_p.is_some() {
            cfg.parameters.top_p = p.top_p;
        }
    }
    if let Some(p) = partial.privacy {
        if let Some(v) = p.allow_private_content_in_local_llm {
            cfg.privacy.allow_private_content_in_local_llm = v;
        }
    }
    if let Some(v) = partial.tools_enabled {
        cfg.tools_enabled = v;
    }
    normalize_ai(&mut cfg);
    cfg
}

fn normalize_ai(cfg: &mut AiConfig) {
    cfg.active_provider = match cfg.active_provider {
        ActiveProvider::Openai | ActiveProvider::Ollama => cfg.active_provider,
    };
    cfg.ollama.base_url = normalize_http_base(&cfg.ollama.base_url, DEFAULT_OLLAMA_BASE);
    cfg.openai_compatible.base_url =
        normalize_http_base(&cfg.openai_compatible.base_url, DEFAULT_OPENAI_BASE);
    cfg.request.timeout_ms = cfg.request.timeout_ms.clamp(TIMEOUT_MS_MIN, TIMEOUT_MS_MAX);
    cfg.parameters.temperature = cfg
        .parameters
        .temperature
        .clamp(TEMP_MIN, TEMP_MAX);
    if let Some(tp) = cfg.parameters.top_p.as_mut() {
        *tp = (*tp).clamp(TOP_P_MIN, TOP_P_MAX);
    }
}

fn read_root_value(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let s = fs::read_to_string(path).map_err(|e| format!("failed to read config: {e}"))?;
    match serde_json::from_str::<Value>(&s) {
        Ok(v) => Ok(v),
        Err(_) => {
            backup_corrupt_config(path)?;
            Ok(json!({}))
        }
    }
}

fn read_schema_version(v: &Value) -> u32 {
    v.get("$schemaVersion")
        .or_else(|| v.get("schemaVersion"))
        .and_then(|x| x.as_u64())
        .map(|u| u as u32)
        .unwrap_or(0)
}

fn effective_schema_version(v: &Value) -> u32 {
    let raw = read_schema_version(v);
    if raw == 0 {
        CURRENT_SCHEMA_VERSION
    } else {
        raw
    }
}

fn load_merged_ai(v: &Value) -> Result<AiConfig, String> {
    let ai_value = v.get("ai").cloned().unwrap_or(json!({}));
    let partial: AiDiskPartial = serde_json::from_value(ai_value).unwrap_or_default();
    Ok(merge_ai_from_disk_partial(AiConfig::default(), partial))
}

fn to_ai_for_ui(ai: AiConfig) -> AiConfigForUi {
    AiConfigForUi {
        active_provider: ai.active_provider,
        ollama: ai.ollama,
        openai_compatible: OpenAiCompatibleForUi {
            base_url: ai.openai_compatible.base_url,
            api_key_present: !ai.openai_compatible.api_key.is_empty(),
            default_model: ai.openai_compatible.default_model,
            organization_id: ai.openai_compatible.organization_id,
            last_used_model: ai.openai_compatible.last_used_model,
        },
        request: ai.request,
        parameters: ai.parameters,
        privacy: ai.privacy,
        tools_enabled: ai.tools_enabled,
    }
}

// --- Cognitive 合并 / 补丁 / 转 UI ---

fn parse_depth_mode(s: &str) -> DepthMode {
    match s.to_ascii_lowercase().as_str() {
        "shallow" => DepthMode::Shallow,
        "medium" => DepthMode::Medium,
        "deep" => DepthMode::Deep,
        _ => DepthMode::Auto,
    }
}

const ACCEPTANCE_WINDOW_CAP: usize = 30;
const PASSIVE_INACCURACY_CAP: u32 = 10_000;

fn prune_challenge_review_dates(dates: &mut ChallengeReviewInlineDates) {
    if dates.by_day.len() <= 120 {
        return;
    }
    let mut keys: Vec<String> = dates.by_day.keys().cloned().collect();
    keys.sort();
    let excess = dates.by_day.len().saturating_sub(120);
    for k in keys.into_iter().take(excess) {
        dates.by_day.remove(&k);
    }
}

fn normalize_cognitive(cfg: &mut CognitiveConfig) {
    if cfg.invite_stats.acceptance_window.len() > ACCEPTANCE_WINDOW_CAP {
        let start = cfg.invite_stats.acceptance_window.len() - ACCEPTANCE_WINDOW_CAP;
        cfg.invite_stats.acceptance_window = cfg.invite_stats.acceptance_window[start..].to_vec();
    }
    cfg.passive_highlight_confidence_min = cfg.passive_highlight_confidence_min.clamp(0.0, 1.0);
    let c = &mut cfg.passive_highlight_inaccuracy_counts;
    c.integrate = c.integrate.min(PASSIVE_INACCURACY_CAP);
    c.correct = c.correct.min(PASSIVE_INACCURACY_CAP);
    c.cross_domain = c.cross_domain.min(PASSIVE_INACCURACY_CAP);
    cfg.challenge_review_daily_cap_independent = cfg
        .challenge_review_daily_cap_independent
        .clamp(1, 20);
    cfg.challenge_review_daily_cap_inline = cfg.challenge_review_daily_cap_inline.clamp(1, 20);
    prune_challenge_review_dates(&mut cfg.challenge_review_inline_dates);
    cfg.writing_coach_idle_seconds = cfg.writing_coach_idle_seconds.clamp(5, 600);
    cfg.writing_coach_depth_min_chars = cfg.writing_coach_depth_min_chars.clamp(10, 20_000);
    cfg.writing_coach_term_min_chars = cfg.writing_coach_term_min_chars.clamp(8, 2000);
    cfg.writing_coach_bubble_seconds = cfg.writing_coach_bubble_seconds.clamp(5, 600);
    cfg.writing_coach_cooldown_minutes = cfg.writing_coach_cooldown_minutes.clamp(1, 1440);
}

fn merge_cognitive_from_disk_partial(
    mut cfg: CognitiveConfig,
    partial: CognitiveDiskPartial,
) -> CognitiveConfig {
    if let Some(s) = partial.depth_mode {
        cfg.depth_mode = parse_depth_mode(&s);
    }
    if partial.auto_manual_override_until.is_some() {
        cfg.auto_manual_override_until = partial.auto_manual_override_until;
    }
    if let Some(is) = partial.invite_stats {
        if let Some(c) = is.consecutive_enough_count {
            cfg.invite_stats.consecutive_enough_count = c;
        }
        if let Some(w) = is.acceptance_window {
            cfg.invite_stats.acceptance_window = w;
        }
        if is.last_enough_timestamp.is_some() {
            cfg.invite_stats.last_enough_timestamp = is.last_enough_timestamp;
        }
    }
    if partial.snooze_invites_until.is_some() {
        cfg.snooze_invites_until = partial.snooze_invites_until;
    }
    if let Some(e) = partial.passive_highlight_enabled {
        cfg.passive_highlight_enabled = e;
    }
    if let Some(m) = partial.passive_highlight_confidence_min {
        cfg.passive_highlight_confidence_min = m;
    }
    if let Some(ic) = partial.passive_highlight_inaccuracy_counts {
        if let Some(x) = ic.integrate {
            cfg.passive_highlight_inaccuracy_counts.integrate = x;
        }
        if let Some(x) = ic.correct {
            cfg.passive_highlight_inaccuracy_counts.correct = x;
        }
        if let Some(x) = ic.cross_domain {
            cfg.passive_highlight_inaccuracy_counts.cross_domain = x;
        }
    }
    if let Some(v) = partial.independent_review_enabled {
        cfg.independent_review_enabled = v;
    }
    if let Some(v) = partial.challenge_review_daily_cap_independent {
        cfg.challenge_review_daily_cap_independent = v;
    }
    if let Some(v) = partial.challenge_review_daily_cap_inline {
        cfg.challenge_review_daily_cap_inline = v;
    }
    if let Some(v) = partial.challenge_review_inline_dates {
        cfg.challenge_review_inline_dates = v;
    }
    if let Some(v) = partial.writing_coach_enabled {
        cfg.writing_coach_enabled = v;
    }
    if partial.writing_coach_cooldown_until.is_some() {
        cfg.writing_coach_cooldown_until = partial.writing_coach_cooldown_until.clone();
    }
    if let Some(v) = partial.writing_coach_idle_seconds {
        cfg.writing_coach_idle_seconds = v;
    }
    if let Some(v) = partial.writing_coach_depth_min_chars {
        cfg.writing_coach_depth_min_chars = v;
    }
    if let Some(v) = partial.writing_coach_term_min_chars {
        cfg.writing_coach_term_min_chars = v;
    }
    if let Some(v) = partial.writing_coach_bubble_seconds {
        cfg.writing_coach_bubble_seconds = v;
    }
    if let Some(v) = partial.writing_coach_cooldown_minutes {
        cfg.writing_coach_cooldown_minutes = v;
    }
    normalize_cognitive(&mut cfg);
    cfg
}

fn load_merged_cognitive(v: &Value) -> CognitiveConfig {
    let cog_value = v.get("cognitive").cloned().unwrap_or(json!({}));
    let partial: CognitiveDiskPartial = serde_json::from_value(cog_value).unwrap_or_default();
    merge_cognitive_from_disk_partial(CognitiveConfig::default(), partial)
}

fn to_cognitive_for_ui(cfg: CognitiveConfig) -> CognitiveConfigForUi {
    CognitiveConfigForUi {
        depth_mode: cfg.depth_mode,
        auto_manual_override_until: cfg.auto_manual_override_until,
        invite_stats: InviteStatsForUi {
            consecutive_enough_count: cfg.invite_stats.consecutive_enough_count,
            acceptance_window: cfg.invite_stats.acceptance_window,
            last_enough_timestamp: cfg.invite_stats.last_enough_timestamp,
        },
        snooze_invites_until: cfg.snooze_invites_until,
        passive_highlight_enabled: cfg.passive_highlight_enabled,
        passive_highlight_confidence_min: cfg.passive_highlight_confidence_min,
        passive_highlight_inaccuracy_counts: PassiveHighlightInaccuracyCountsForUi {
            integrate: cfg.passive_highlight_inaccuracy_counts.integrate,
            correct: cfg.passive_highlight_inaccuracy_counts.correct,
            cross_domain: cfg.passive_highlight_inaccuracy_counts.cross_domain,
        },
        independent_review_enabled: cfg.independent_review_enabled,
        challenge_review_daily_cap_independent: cfg.challenge_review_daily_cap_independent,
        challenge_review_daily_cap_inline: cfg.challenge_review_daily_cap_inline,
        challenge_review_inline_dates: cfg.challenge_review_inline_dates,
        writing_coach_enabled: cfg.writing_coach_enabled,
        writing_coach_cooldown_until: cfg.writing_coach_cooldown_until.clone(),
        writing_coach_idle_seconds: cfg.writing_coach_idle_seconds,
        writing_coach_depth_min_chars: cfg.writing_coach_depth_min_chars,
        writing_coach_term_min_chars: cfg.writing_coach_term_min_chars,
        writing_coach_bubble_seconds: cfg.writing_coach_bubble_seconds,
        writing_coach_cooldown_minutes: cfg.writing_coach_cooldown_minutes,
    }
}

fn apply_cognitive_patch(cfg: &mut CognitiveConfig, patch: CognitiveConfigPatch) {
    if let Some(d) = patch.depth_mode {
        cfg.depth_mode = d;
    }
    if let Some(o) = patch.auto_manual_override_until {
        cfg.auto_manual_override_until = o;
    }
    if let Some(is) = patch.invite_stats {
        if let Some(c) = is.consecutive_enough_count {
            cfg.invite_stats.consecutive_enough_count = c;
        }
        if let Some(w) = is.acceptance_window {
            cfg.invite_stats.acceptance_window = w;
        }
        if let Some(t) = is.last_enough_timestamp {
            cfg.invite_stats.last_enough_timestamp = t;
        }
    }
    if let Some(s) = patch.snooze_invites_until {
        cfg.snooze_invites_until = s;
    }
    if let Some(e) = patch.passive_highlight_enabled {
        cfg.passive_highlight_enabled = e;
    }
    if let Some(m) = patch.passive_highlight_confidence_min {
        cfg.passive_highlight_confidence_min = m;
    }
    if let Some(v) = patch.independent_review_enabled {
        cfg.independent_review_enabled = v;
    }
    if let Some(v) = patch.challenge_review_daily_cap_independent {
        cfg.challenge_review_daily_cap_independent = v;
    }
    if let Some(v) = patch.challenge_review_daily_cap_inline {
        cfg.challenge_review_daily_cap_inline = v;
    }
    if let Some(v) = patch.challenge_review_inline_dates {
        cfg.challenge_review_inline_dates = v;
    }
    if let Some(v) = patch.writing_coach_enabled {
        cfg.writing_coach_enabled = v;
    }
    if let Some(v) = patch.writing_coach_cooldown_until {
        cfg.writing_coach_cooldown_until = v;
    }
    if let Some(v) = patch.writing_coach_idle_seconds {
        cfg.writing_coach_idle_seconds = v;
    }
    if let Some(v) = patch.writing_coach_depth_min_chars {
        cfg.writing_coach_depth_min_chars = v;
    }
    if let Some(v) = patch.writing_coach_term_min_chars {
        cfg.writing_coach_term_min_chars = v;
    }
    if let Some(v) = patch.writing_coach_bubble_seconds {
        cfg.writing_coach_bubble_seconds = v;
    }
    if let Some(v) = patch.writing_coach_cooldown_minutes {
        cfg.writing_coach_cooldown_minutes = v;
    }
}

fn apply_ai_patch(cfg: &mut AiConfig, patch: AiConfigPatch) {
    if let Some(p) = patch.active_provider {
        cfg.active_provider = p;
    }
    if let Some(o) = patch.ollama {
        if let Some(u) = o.base_url {
            cfg.ollama.base_url = u;
        }
        if let Some(m) = o.default_model {
            cfg.ollama.default_model = m;
        }
        if let Some(l) = o.last_used_model {
            cfg.ollama.last_used_model = l;
        }
    }
    if let Some(o) = patch.openai_compatible {
        if let Some(u) = o.base_url {
            cfg.openai_compatible.base_url = u;
        }
        if let Some(k) = o.api_key {
            cfg.openai_compatible.api_key = k;
        }
        if let Some(m) = o.default_model {
            cfg.openai_compatible.default_model = m;
        }
        if let Some(org) = o.organization_id {
            cfg.openai_compatible.organization_id = org;
        }
        if let Some(l) = o.last_used_model {
            cfg.openai_compatible.last_used_model = l;
        }
    }
    if let Some(r) = patch.request {
        if let Some(t) = r.timeout_ms {
            cfg.request.timeout_ms = t;
        }
        if let Some(m) = r.max_context_tokens {
            cfg.request.max_context_tokens = m;
        }
    }
    if let Some(p) = patch.parameters {
        if let Some(t) = p.temperature {
            cfg.parameters.temperature = t;
        }
        if let Some(tp) = p.top_p {
            cfg.parameters.top_p = tp;
        }
    }
    if let Some(p) = patch.privacy {
        if let Some(v) = p.allow_private_content_in_local_llm {
            cfg.privacy.allow_private_content_in_local_llm = v;
        }
    }
    if let Some(v) = patch.tools_enabled {
        cfg.tools_enabled = v;
    }
}

fn atomic_write_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "invalid config path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("failed to create config dir: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    let pretty = serde_json::to_string_pretty(value).map_err(|e| format!("failed to serialize config: {e}"))?;
    let contents = format!("{pretty}\n");
    fs::write(&tmp, contents).map_err(|e| format!("failed to write temp config: {e}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| format!("failed to replace existing config: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("failed to finalize config: {e}"))?;
    Ok(())
}

/// LLM 运行时读取完整 `AiConfig`（含 OpenAI 密钥）；**禁止**经 IPC 返回给前端。
pub fn load_ai_config_internal(root: &Path) -> Result<AiConfig, String> {
    let path = config_path(root);
    let v = read_root_value(&path)?;
    load_merged_ai(&v)
}

/// 供 `get_vault_config_for_ui`：合并默认、脱敏 OpenAI 密钥
pub fn load_for_ui(root: &Path) -> Result<VaultConfigForUi, String> {
    let path = config_path(root);
    let v = read_root_value(&path)?;
    let schema_version = effective_schema_version(&v);
    let ai = load_merged_ai(&v)?;
    let cognitive = load_merged_cognitive(&v);
    let semantic = load_merged_semantic(&v);
    Ok(VaultConfigForUi {
        schema_version,
        ai: to_ai_for_ui(ai),
        cognitive: to_cognitive_for_ui(cognitive),
        semantic,
    })
}

/// 合并后的认知配置（供被动高亮等读取）
pub fn load_cognitive_merged(root: &Path) -> Result<CognitiveConfig, String> {
    let path = config_path(root);
    let v = read_root_value(&path)?;
    Ok(load_merged_cognitive(&v))
}

/// 被动高亮「不准确」：按 kind 递增计数并原子写回 config
pub fn bump_passive_highlight_inaccuracy(root: &Path, kind: &str) -> Result<(), String> {
    let path = config_path(root);
    let mut v = read_root_value(&path)?;
    let mut merged = load_merged_cognitive(&v);
    let k = kind.to_ascii_lowercase();
    match k.as_str() {
        "integrate" => {
            merged.passive_highlight_inaccuracy_counts.integrate = merged
                .passive_highlight_inaccuracy_counts
                .integrate
                .saturating_add(1);
        }
        "correct" => {
            merged.passive_highlight_inaccuracy_counts.correct = merged
                .passive_highlight_inaccuracy_counts
                .correct
                .saturating_add(1);
        }
        "cross_domain" => {
            merged.passive_highlight_inaccuracy_counts.cross_domain = merged
                .passive_highlight_inaccuracy_counts
                .cross_domain
                .saturating_add(1);
        }
        _ => return Err(format!("unknown passive highlight kind: {kind}")),
    }
    normalize_cognitive(&mut merged);
    v["cognitive"] =
        serde_json::to_value(&merged).map_err(|e| format!("failed to serialize cognitive: {e}"))?;
    let schema_out = read_schema_version(&v).max(CURRENT_SCHEMA_VERSION);
    v["$schemaVersion"] = json!(schema_out);
    atomic_write_json(&path, &v)
}

/// 合并补丁后原子写回；`ai` 与 `cognitive` 均为 `None` 时不写盘
pub fn save_patch(root: &Path, patch: VaultConfigPatch) -> Result<(), String> {
    if patch.ai.is_none() && patch.cognitive.is_none() && patch.semantic.is_none() {
        return Ok(());
    }

    let dir = knowforge_dir(root);
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create .knowforge: {e}"))?;

    let path = config_path(root);
    let mut v = read_root_value(&path)?;

    if let Some(ai_patch) = patch.ai {
        let mut merged = load_merged_ai(&v)?;
        apply_ai_patch(&mut merged, ai_patch);
        normalize_ai(&mut merged);
        v["ai"] =
            serde_json::to_value(&merged).map_err(|e| format!("failed to serialize ai: {e}"))?;
    }

    if let Some(cog_patch) = patch.cognitive {
        let mut merged = load_merged_cognitive(&v);
        apply_cognitive_patch(&mut merged, cog_patch);
        normalize_cognitive(&mut merged);
        v["cognitive"] = serde_json::to_value(&merged)
            .map_err(|e| format!("failed to serialize cognitive: {e}"))?;
    }

    if let Some(sem_patch) = patch.semantic {
        let mut merged = load_merged_semantic(&v);
        apply_semantic_patch(&mut merged, sem_patch);
        normalize_semantic(&mut merged);
        v["semantic"] =
            serde_json::to_value(&merged).map_err(|e| format!("failed to serialize semantic: {e}"))?;
    }

    let schema_out = read_schema_version(&v).max(CURRENT_SCHEMA_VERSION);
    v["$schemaVersion"] = json!(schema_out);
    atomic_write_json(&path, &v)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_defaults_ollama() {
        let cfg = load_merged_ai(&json!({})).unwrap();
        assert_eq!(cfg.active_provider, ActiveProvider::Ollama);
        assert_eq!(cfg.ollama.base_url, DEFAULT_OLLAMA_BASE);
    }

    #[test]
    fn patch_ollama_preserves_openai_key() {
        let mut v = json!({
            "ai": {
                "openaiCompatible": { "apiKey": "secret", "baseUrl": "https://api.openai.com/v1", "defaultModel": "gpt-4o-mini" }
            }
        });
        // 直接在内存上测合并逻辑
        let mut merged = load_merged_ai(&v).unwrap();
        apply_ai_patch(
            &mut merged,
            AiConfigPatch {
                ollama: Some(OllamaPatch {
                    base_url: Some("http://127.0.0.1:11435".to_string()),
                    default_model: Some("llama".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        normalize_ai(&mut merged);
        assert_eq!(merged.openai_compatible.api_key, "secret");
        assert_eq!(merged.ollama.default_model, "llama");
        v["ai"] = serde_json::to_value(&merged).unwrap();
        let again = load_merged_ai(&v).unwrap();
        assert_eq!(again.openai_compatible.api_key, "secret");
    }

    #[test]
    fn clear_api_key_with_empty_string() {
        let v = json!({
            "ai": {
                "openaiCompatible": { "apiKey": "x", "baseUrl": "https://api.openai.com/v1", "defaultModel": "gpt-4o-mini" }
            }
        });
        let mut merged = load_merged_ai(&v).unwrap();
        apply_ai_patch(
            &mut merged,
            AiConfigPatch {
                openai_compatible: Some(OpenAiCompatiblePatch {
                    api_key: Some(String::new()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        normalize_ai(&mut merged);
        assert!(merged.openai_compatible.api_key.is_empty());
    }

    #[test]
    fn cognitive_defaults_to_auto() {
        let cfg = load_merged_cognitive(&json!({}));
        assert_eq!(cfg.depth_mode, DepthMode::Auto);
        assert_eq!(cfg.invite_stats.consecutive_enough_count, 0);
        assert!(cfg.invite_stats.acceptance_window.is_empty());
        assert!(cfg.snooze_invites_until.is_none());
        assert!(cfg.passive_highlight_enabled);
        assert!((cfg.passive_highlight_confidence_min - 0.55).abs() < f64::EPSILON);
        assert_eq!(cfg.passive_highlight_inaccuracy_counts.integrate, 0);
        assert!(!cfg.independent_review_enabled);
        assert_eq!(cfg.challenge_review_daily_cap_independent, 3);
        assert_eq!(cfg.challenge_review_daily_cap_inline, 2);
        assert!(cfg.challenge_review_inline_dates.by_day.is_empty());
        assert!(cfg.writing_coach_enabled);
        assert!(cfg.writing_coach_cooldown_until.is_none());
        assert_eq!(cfg.writing_coach_idle_seconds, 15);
        assert_eq!(cfg.writing_coach_depth_min_chars, 500);
        assert_eq!(cfg.writing_coach_term_min_chars, 36);
        assert_eq!(cfg.writing_coach_bubble_seconds, 30);
        assert_eq!(cfg.writing_coach_cooldown_minutes, 15);
    }

    #[test]
    fn cognitive_merge_from_disk() {
        let v = json!({
            "cognitive": {
                "depthMode": "deep",
                "inviteStats": {
                    "consecutiveEnoughCount": 3,
                    "acceptanceWindow": [true, false, true]
                },
                "snoozeInvitesUntil": "2026-04-20T00:00:00Z"
            }
        });
        let cfg = load_merged_cognitive(&v);
        assert_eq!(cfg.depth_mode, DepthMode::Deep);
        assert_eq!(cfg.invite_stats.consecutive_enough_count, 3);
        assert_eq!(cfg.invite_stats.acceptance_window, vec![true, false, true]);
        assert_eq!(
            cfg.snooze_invites_until.as_deref(),
            Some("2026-04-20T00:00:00Z")
        );
    }

    #[test]
    fn cognitive_patch_depth_mode() {
        let mut cfg = CognitiveConfig::default();
        assert_eq!(cfg.depth_mode, DepthMode::Auto);
        apply_cognitive_patch(
            &mut cfg,
            CognitiveConfigPatch {
                depth_mode: Some(DepthMode::Shallow),
                ..Default::default()
            },
        );
        assert_eq!(cfg.depth_mode, DepthMode::Shallow);
    }

    #[test]
    fn cognitive_acceptance_window_cap() {
        let mut cfg = CognitiveConfig::default();
        cfg.invite_stats.acceptance_window = vec![true; 40];
        normalize_cognitive(&mut cfg);
        assert_eq!(cfg.invite_stats.acceptance_window.len(), ACCEPTANCE_WINDOW_CAP);
    }

    #[test]
    fn load_for_ui_includes_cognitive() {
        let v = json!({
            "cognitive": { "depthMode": "medium" }
        });
        let cog = load_merged_cognitive(&v);
        let ui = to_cognitive_for_ui(cog);
        assert_eq!(ui.depth_mode, DepthMode::Medium);
    }
}
