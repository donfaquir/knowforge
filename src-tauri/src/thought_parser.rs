//! Markdown `> [!thought]` / `> \[!thought]`（转义 `[`）callout 解析 + `kf-thoughts` frontmatter 元数据读写（迭代 3）。
#![cfg_attr(not(test), allow(dead_code))]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::vault_thoughts_db;

// --- kf-thoughts YAML 元数据类型 ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThoughtMaturity {
    Seedling,
    Growing,
    Mature,
}

impl Default for ThoughtMaturity {
    fn default() -> Self {
        Self::Seedling
    }
}

/// 成熟度序：用于比较是否「升级」。
pub fn thought_maturity_rank(m: ThoughtMaturity) -> u8 {
    match m {
        ThoughtMaturity::Seedling => 0,
        ThoughtMaturity::Growing => 1,
        ThoughtMaturity::Mature => 2,
    }
}

pub fn thought_maturity_as_str(m: ThoughtMaturity) -> &'static str {
    match m {
        ThoughtMaturity::Seedling => "seedling",
        ThoughtMaturity::Growing => "growing",
        ThoughtMaturity::Mature => "mature",
    }
}

/// 供 Tauri 事件 `thought-maturity-changed` 与前端 Toast 对齐（camelCase）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtMaturityChangedPayload {
    pub rel_path: String,
    pub thought_id: String,
    pub from_maturity: String,
    pub to_maturity: String,
    pub start_line: usize,
}

/// 挑战写回路径上不含 `rel_path`，由 `lib.rs` 组装为 [`ThoughtMaturityChangedPayload`]。
#[derive(Debug, Clone)]
pub struct ThoughtMaturityChangedCore {
    pub thought_id: String,
    pub from_maturity: String,
    pub to_maturity: String,
    pub start_line: usize,
}

#[derive(Debug, Clone)]
pub struct ApplyChallengePassToMarkdownOutcome {
    pub markdown: String,
    pub maturity_change: Option<ThoughtMaturityChangedCore>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtHistoryEntry {
    pub date: String,
    /// "created" | "substantial-change"
    #[serde(rename = "type")]
    pub entry_type: String,
    /// "manual" | "ai-save"
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtReference {
    pub date: String,
    pub context: String,
    pub relevance: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KfThoughtMeta {
    pub id: String,
    #[serde(default)]
    pub maturity: ThoughtMaturity,
    pub created: String,
    pub updated: String,
    #[serde(default)]
    pub temporary: bool,
    /// 挑战式回顾通过次数（迭代 4）
    #[serde(default)]
    pub challenge_pass_count: u32,
    /// 上次成功回顾时间（ISO8601，用于遗忘曲线）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reviewed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<ThoughtHistoryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ThoughtReference>,
}

// --- 从 Markdown 正文解析出的理解区块 ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedThoughtBlock {
    pub id: String,
    pub maturity: ThoughtMaturity,
    pub excerpt: String,
    pub start_line: usize,
    pub end_line: usize,
    pub temporary: bool,
}

// --- 解析逻辑 ---

/// Callout 标题行是否标记为临时理解（与语言无关的存储层语法）。
///
/// 说明：UI 文案在 `src/locales/*.json`（如 `thoughtSave.temporary`），**不得**作为解析依据；
/// 磁盘格式使用 Obsidian 式 `|temporary` 或少数文档中的 `[temporary]` 片段。
fn header_line_temporary(header_line: &str) -> bool {
    let lower = header_line.to_ascii_lowercase();
    lower.contains("|temporary") || lower.contains("[temporary]")
}

/// CommonMark 风格围栏：行首空白后以 ≥3 个连续 `` ` `` 开头视为起止；闭合序列长度须 ≥ 起始长度。
/// 用于忽略代码块 / Mermaid 等 fenced 区域内的示例文本，避免把 `` ```markdown `` 里的 ``> [!thought]`` 当成真实 callout。
fn apply_backtick_fence_line(line: &str, in_fence: &mut bool, open_len: &mut usize) {
    let t = line.trim_start();
    let mut n = 0usize;
    for ch in t.chars() {
        if ch == '`' {
            n += 1;
        } else {
            break;
        }
    }
    if n < 3 {
        return;
    }
    if !*in_fence {
        *in_fence = true;
        *open_len = n;
    } else if n >= *open_len {
        *in_fence = false;
        *open_len = 0;
    }
}

/// `out[i]`：处理第 `i` 行**之前**是否处于反引号围栏内（故该行若为 ``> [!thought]`` 且为 true 则跳过识别）。
fn inside_backtick_fence_before_each_line(lines: &[&str]) -> Vec<bool> {
    let mut in_fence = false;
    let mut open_len = 0usize;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        out.push(in_fence);
        apply_backtick_fence_line(line, &mut in_fence, &mut open_len);
    }
    out
}

/// 从 Markdown 正文扫描所有 `> [!thought` / `> \[!thought` callout 区块。
/// 围栏代码块（`` ``` ``）内的行不参与识别，避免设计文档中的示例被误计。
/// 返回按出现顺序排列的结构化列表。行号从 1 开始。
pub fn parse_thought_blocks(markdown: &str) -> Vec<ParsedThoughtBlock> {
    let lines: Vec<&str> = markdown.lines().collect();
    let skip_line = inside_backtick_fence_before_each_line(&lines);
    let mut results = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if skip_line[i] {
            i += 1;
            continue;
        }
        let trimmed = lines[i].trim();
        if is_thought_callout_start(trimmed) {
            let start_line = i + 1; // 1-based
            let maturity = extract_maturity(trimmed);
            let temporary = header_line_temporary(trimmed);

            // 收集 callout 正文行（连续 `> ` 或 `>` 前缀行）
            let mut body_parts: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let line = lines[j];
                if let Some(content) = strip_blockquote_prefix(line) {
                    body_parts.push(content);
                    j += 1;
                } else if line.trim().is_empty() {
                    // 空行终止 callout
                    break;
                } else {
                    break;
                }
            }

            let end_line = if j > i + 1 { j } else { start_line }; // 1-based
            let excerpt = build_excerpt(&body_parts, 200);

            // 尝试从 frontmatter 中匹配 ID，否则生成占位 ID
            let id = format!("thought-parsed-L{start_line}");

            results.push(ParsedThoughtBlock {
                id,
                maturity,
                excerpt,
                start_line,
                end_line,
                temporary,
            });
            i = j;
        } else {
            i += 1;
        }
    }
    results
}

/// `trimmed` 为整行 trim 后；须以 `>` 起头，其后仅允许空白再接 `[!thought` 或 `\[!thought`（thought 段 ASCII 不区分大小写），且 `thought` 后须为 `]`、`|`、空白或行尾，避免 `[!thoughtful`。
fn is_thought_callout_start(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    let after_gt = match lower.strip_prefix('>') {
        Some(rest) => rest.trim_start(),
        None => return false,
    };
    thought_callout_marker_after_gt(after_gt)
}

fn thought_callout_marker_after_gt(after_gt_trimmed: &str) -> bool {
    if let Some(tail) = after_gt_trimmed.strip_prefix("[!thought") {
        return thought_type_suffix_boundary_ok(tail);
    }
    if let Some(tail) = after_gt_trimmed.strip_prefix("\\[!thought") {
        return thought_type_suffix_boundary_ok(tail);
    }
    false
}

fn thought_type_suffix_boundary_ok(tail: &str) -> bool {
    tail.is_empty()
        || tail.starts_with(|c: char| matches!(c, ']' | '|' | ' ' | '\t'))
}

fn extract_maturity(header_line: &str) -> ThoughtMaturity {
    let lower = header_line.to_ascii_lowercase();
    if header_line.contains("🌳") || lower.contains("mature") {
        ThoughtMaturity::Mature
    } else if header_line.contains("🌿") || lower.contains("growing") {
        ThoughtMaturity::Growing
    } else {
        // 默认或 🌱 -> seedling
        ThoughtMaturity::Seedling
    }
}

fn strip_blockquote_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("> ") {
        Some(rest)
    } else if trimmed == ">" {
        Some("")
    } else {
        None
    }
}

fn build_excerpt(parts: &[&str], max_chars: usize) -> String {
    let mut buf = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            buf.push(' ');
        }
        buf.push_str(part.trim());
    }
    if buf.len() > max_chars {
        // 按 char boundary 截断
        let mut end = max_chars;
        while !buf.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        buf.truncate(end);
        buf.push_str("...");
    }
    buf
}

// --- kf-thoughts frontmatter 解析 ---

/// 从 frontmatter YAML（已提取的字符串）中解析 `kf-thoughts` 数组。
/// `warnings` 为非致命问题（供 IPC 返回与终端日志）；解析失败时 `meta` 为空。
pub fn parse_kf_thoughts_from_yaml(yaml_str: &str) -> (Vec<KfThoughtMeta>, Vec<String>) {
    let mut warnings = Vec::new();
    let trimmed = yaml_str.trim();
    if trimmed.is_empty() {
        return (Vec::new(), warnings);
    }
    let value = match serde_yaml::from_str::<serde_yaml::Value>(trimmed) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!(
                "kf-thoughts: failed to parse YAML frontmatter root: {e}"
            ));
            return (Vec::new(), warnings);
        }
    };
    let Some(mapping) = value.as_mapping() else {
        warnings.push(
            "kf-thoughts: frontmatter root is not a YAML mapping; kf-thoughts ignored".to_string(),
        );
        return (Vec::new(), warnings);
    };
    let key = serde_yaml::Value::String("kf-thoughts".to_string());
    let Some(thoughts_val) = mapping.get(&key) else {
        return (Vec::new(), warnings);
    };
    match serde_yaml::from_value::<Vec<KfThoughtMeta>>(thoughts_val.clone()) {
        Ok(v) => (v, warnings),
        Err(e) => {
            warnings.push(format!(
                "kf-thoughts: failed to deserialize kf-thoughts array: {e}"
            ));
            (Vec::new(), warnings)
        }
    }
}

/// 从 frontmatter YAML 读取笔记级稳定 id（与侧车 `note_stable_id` 对应）。
pub fn parse_kf_vault_note_id(yaml_src: &str) -> Option<String> {
    let trimmed = yaml_src.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = serde_yaml::from_str::<serde_yaml::Value>(trimmed).ok()?;
    value
        .get("kfVaultNoteId")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 若无 `kfVaultNoteId` 则写入新 UUID；根须为 YAML mapping。
pub fn ensure_kf_vault_note_id_in_yaml(yaml_src: &str) -> Result<String, String> {
    let mut doc: serde_yaml::Value = if yaml_src.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(yaml_src).map_err(|e| format!("解析 frontmatter YAML 失败: {e}"))?
    };
    let map = doc
        .as_mapping_mut()
        .ok_or_else(|| "frontmatter 根须为 YAML mapping".to_string())?;
    let key = serde_yaml::Value::String("kfVaultNoteId".to_string());
    let need = match map.get(&key) {
        None => true,
        Some(v) => v.as_str().map(|s| s.trim().is_empty()).unwrap_or(true),
    };
    if need {
        map.insert(
            key,
            serde_yaml::Value::String(uuid::Uuid::new_v4().to_string()),
        );
    }
    serde_yaml::to_string(&doc).map_err(|e| format!("序列化 frontmatter 失败: {e}"))
}

/// 侧车 / 存储中的成熟度小写串 → 枚举
pub fn thought_maturity_from_storage(s: &str) -> ThoughtMaturity {
    match s.to_ascii_lowercase().as_str() {
        "growing" => ThoughtMaturity::Growing,
        "mature" => ThoughtMaturity::Mature,
        _ => ThoughtMaturity::Seedling,
    }
}

fn build_excerpt_from_body_text(body: &str, max_chars: usize) -> String {
    let lines: Vec<&str> = body.lines().collect();
    build_excerpt(&lines, max_chars)
}

// --- 生成 thought ID ---

/// 生成唯一的 thought ID：`thought-YYYYMMDD-HHMMSS-NNN`。
/// `seq` 用于同一秒内的区分（通常传入已有 thought 的数量）。
pub fn generate_thought_id(seq: usize) -> String {
    let now = Utc::now();
    format!(
        "thought-{}-{:03}",
        now.format("%Y%m%d-%H%M%S"),
        seq % 1000
    )
}

// --- 生成 callout 文本 ---

/// 挑战通过后更新成熟度：Growing 且 `challenge_pass_count >= 1` 时升为 Mature。
pub fn maturity_after_challenge_pass(
    current: ThoughtMaturity,
    challenge_pass_count: u32,
) -> ThoughtMaturity {
    if challenge_pass_count >= 1 && matches!(current, ThoughtMaturity::Growing) {
        ThoughtMaturity::Mature
    } else {
        current
    }
}

/// 生成插入到 Markdown 中的 callout 文本块。
pub fn build_thought_callout(content: &str, maturity: ThoughtMaturity) -> String {
    let emoji = match maturity {
        ThoughtMaturity::Seedling => "🌱",
        ThoughtMaturity::Growing => "🌿",
        ThoughtMaturity::Mature => "🌳",
    };
    let header = format!("> [!thought] 随手想法 {emoji}");
    if content.trim().is_empty() {
        format!("{header}\n> ")
    } else {
        let body_lines: Vec<String> = content.lines().map(|l| format!("> {l}")).collect();
        format!("{header}\n{}", body_lines.join("\n"))
    }
}

// --- 生成新 KfThoughtMeta ---

pub fn new_thought_meta(id: &str, temporary: bool, source: &str) -> KfThoughtMeta {
    let now = Utc::now().to_rfc3339();
    KfThoughtMeta {
        id: id.to_string(),
        maturity: ThoughtMaturity::Seedling,
        created: now.clone(),
        updated: now.clone(),
        temporary,
        challenge_pass_count: 0,
        last_reviewed_at: None,
        history: vec![ThoughtHistoryEntry {
            date: now,
            entry_type: "created".to_string(),
            source: source.to_string(),
            diff_summary: None,
        }],
        references: Vec::new(),
    }
}

// --- Frontmatter 操作（split / rebuild / upsert） ---

/// 是否为 YAML frontmatter 的 `---` 围栏行（与正文中的水平线 `---` 区分：仅当整行 trim 后为 `---`）。
fn line_is_frontmatter_fence_delimiter(line: &str) -> bool {
    line.trim()
        .trim_start_matches('\u{feff}')
        .trim_end_matches('\r')
        .trim_end()
        == "---"
}

/// Obsidian 约定：frontmatter 起始 `---` 须紧邻文件开头（仅允许 BOM 与空白行）。否则不把首对 `---` 当 YAML，避免误扫设计文档中的水平线。
fn leading_prefix_allows_yaml_frontmatter(prefix: &str) -> bool {
    for ch in prefix.chars() {
        if ch == '\u{feff}' || ch.is_whitespace() {
            continue;
        }
        return false;
    }
    true
}

/// `split_frontmatter` 的三种结果：无起始围栏、合法闭合、有起始无闭合（禁止插入管线静默改写）。
#[derive(Debug, Clone, PartialEq)]
pub enum FrontmatterSplit {
    /// 全文无起始 `---` 围栏；整篇视为正文，插入时可新建 frontmatter。
    NoFence(String),
    /// 合法闭合围栏
    Closed {
        yaml: String,
        body: String,
        leading_prefix: String,
    },
    /// 有起始围栏但无闭合围栏；原文须完整保留，不得包一层新 `---`。
    Unclosed(String),
}

/// 拆分 frontmatter：不 `trim_start` 全文；未闭合时返回 [`FrontmatterSplit::Unclosed`]，避免与「无围栏」混淆。
pub fn split_frontmatter(markdown: &str) -> FrontmatterSplit {
    let bytes = markdown.as_bytes();
    let mut off = 0usize;
    let mut open_fence: Option<(usize, usize)> = None;

    while off < markdown.len() {
        let line_start = off;
        let mut line_end = off;
        while line_end < bytes.len() && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let line = &markdown[line_start..line_end];
        if line_is_frontmatter_fence_delimiter(line) {
            open_fence = Some((line_start, line_end));
            break;
        }
        if line_end >= bytes.len() {
            break;
        }
        off = line_end + 1;
    }

    let Some((open_start, open_end)) = open_fence else {
        return FrontmatterSplit::NoFence(markdown.to_string());
    };

    let leading_prefix = markdown[0..open_start].to_string();

    let mut yaml_lines: Vec<&str> = Vec::new();
    let mut body_lines: Vec<&str> = Vec::new();
    let mut in_yaml = true;

    // 跳过 opening 行尾换行（`\n` 或 `\r\n` 中最后的 `\n` 已由 line 切片排除）
    off = if open_end < bytes.len() {
        open_end + 1
    } else {
        open_end
    };

    while off < markdown.len() {
        let line_start = off;
        let mut line_end = off;
        while line_end < bytes.len() && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let line = &markdown[line_start..line_end];
        if in_yaml {
            if line_is_frontmatter_fence_delimiter(line) {
                in_yaml = false;
            } else {
                yaml_lines.push(line);
            }
        } else {
            body_lines.push(line);
        }
        if line_end >= bytes.len() {
            break;
        }
        off = line_end + 1;
    }

    if in_yaml {
        if !leading_prefix_allows_yaml_frontmatter(&leading_prefix) {
            return FrontmatterSplit::NoFence(markdown.to_string());
        }
        return FrontmatterSplit::Unclosed(markdown.to_string());
    }

    if !leading_prefix_allows_yaml_frontmatter(&leading_prefix) {
        return FrontmatterSplit::NoFence(markdown.to_string());
    }

    FrontmatterSplit::Closed {
        yaml: yaml_lines.join("\n"),
        body: body_lines.join("\n"),
        leading_prefix,
    }
}

/// 从 frontmatter YAML + body 重建完整 Markdown。
pub fn rebuild_with_frontmatter(yaml: &str, body: &str) -> String {
    let trimmed_yaml = yaml.trim_end();
    if body.is_empty() {
        format!("---\n{trimmed_yaml}\n---\n")
    } else {
        format!("---\n{trimmed_yaml}\n---\n{body}")
    }
}

/// 在 frontmatter YAML 中 upsert 一条 `kf-thoughts` 条目（按 id 去重）。
/// 返回更新后的 YAML 字符串。
pub fn upsert_kf_thought_in_yaml(yaml: &str, meta: &KfThoughtMeta) -> String {
    let mut doc: serde_yaml::Value = if yaml.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(yaml).unwrap_or_else(|e| {
            eprintln!(
                "[thought_parser] upsert_kf_thought_in_yaml: failed to parse existing YAML, replacing with empty mapping (data loss risk): {e}"
            );
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        })
    };

    let mapping = match doc.as_mapping_mut() {
        Some(m) => m,
        None => {
            // 根非 mapping，重建为新 mapping
            doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            doc.as_mapping_mut().expect("刚赋值为 Mapping")
        }
    };

    let key = serde_yaml::Value::String("kf-thoughts".to_string());
    let meta_val = serde_yaml::to_value(meta).unwrap_or(serde_yaml::Value::Null);

    match mapping.get_mut(&key) {
        Some(existing) => {
            if let Some(arr) = existing.as_sequence_mut() {
                let existing_idx = arr.iter().position(|v| {
                    v.get("id").and_then(|id| id.as_str()) == Some(&meta.id)
                });
                if let Some(idx) = existing_idx {
                    arr[idx] = meta_val;
                } else {
                    arr.push(meta_val);
                }
            } else {
                *existing = serde_yaml::Value::Sequence(vec![meta_val]);
            }
        }
        None => {
            mapping.insert(key, serde_yaml::Value::Sequence(vec![meta_val]));
        }
    }

    serde_yaml::to_string(&doc).unwrap_or_else(|e| {
        eprintln!(
            "[thought_parser] upsert_kf_thought_in_yaml: failed to serialize YAML after upsert: {e}"
        );
        String::new()
    })
}

/// 将 `kf-thoughts` 整表替换为 `metas`（保持其余 YAML 键）
fn replace_kf_thoughts_sequence_in_yaml(yaml: &str, metas: &[KfThoughtMeta]) -> Result<String, String> {
    let mut doc: serde_yaml::Value = if yaml.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(yaml).map_err(|e| format!("解析 YAML 失败: {e}"))?
    };
    let mapping = doc
        .as_mapping_mut()
        .ok_or_else(|| "YAML 根须为 mapping".to_string())?;
    let key = serde_yaml::Value::String("kf-thoughts".to_string());
    let seq: Vec<serde_yaml::Value> = metas
        .iter()
        .map(|m| serde_yaml::to_value(m).map_err(|e| format!("序列化 meta 失败: {e}")))
        .collect::<Result<_, _>>()?;
    mapping.insert(key, serde_yaml::Value::Sequence(seq));
    serde_yaml::to_string(&doc).map_err(|e| format!("写回 YAML 失败: {e}"))
}

/// 将某条 thought 的 `updated` 写为当前时间（RFC3339）
pub fn bump_kf_thought_updated_in_markdown(markdown: &str, thought_id: &str) -> Result<String, String> {
    if thought_id.is_empty() {
        return Err("thought_id is empty".to_string());
    }
    const UNCLOSED_MSG: &str =
        "Unclosed YAML frontmatter: add a closing --- line before saving a thought.";
    const NO_FM_MSG: &str = "笔记缺少闭合的 YAML frontmatter，无法更新想法元数据。";

    let (yaml_src, body, leading_prefix) = match split_frontmatter(markdown) {
        FrontmatterSplit::Unclosed(_) => return Err(UNCLOSED_MSG.to_string()),
        FrontmatterSplit::NoFence(_) => return Err(NO_FM_MSG.to_string()),
        FrontmatterSplit::Closed {
            yaml,
            body,
            leading_prefix,
        } => (yaml, body, leading_prefix),
    };

    let (mut meta_vec, _) = parse_kf_thoughts_from_yaml(&yaml_src);
    let idx = meta_vec
        .iter()
        .position(|m| m.id == thought_id)
        .ok_or_else(|| format!("找不到 id 为 {thought_id} 的 thought 元数据"))?;
    let now_rfc = Utc::now().to_rfc3339();
    meta_vec[idx].updated = now_rfc;
    let new_yaml = replace_kf_thoughts_sequence_in_yaml(&yaml_src, &meta_vec)?;
    Ok(format!(
        "{}{}",
        leading_prefix,
        rebuild_with_frontmatter(&new_yaml, &body)
    ))
}

/// 从正文按 1-based 行号切除一段（含起止行）
fn remove_line_range_inclusive(body: &str, start_line: usize, end_line: usize) -> String {
    if start_line == 0 || end_line < start_line {
        return body.to_string();
    }
    let lines: Vec<&str> = body.lines().collect();
    let start0 = start_line.saturating_sub(1).min(lines.len());
    let end0 = end_line.min(lines.len());
    let mut out: Vec<&str> = Vec::new();
    out.extend_from_slice(&lines[..start0]);
    out.extend_from_slice(&lines[end0..]);
    let mut s = out.join("\n");
    if body.ends_with('\n') && !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    } else if body.ends_with('\n') && s.is_empty() {
        s.push('\n');
    }
    s
}

/// 从 Markdown 中删除指定 thought：`kf-thoughts` 必删；正文 callout 仅当与 meta 条数对齐时按同一下标删除
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveThoughtOutcome {
    pub markdown: String,
    /// 是否从 YAML 中移除了该 id
    pub removed: bool,
    /// 是否同时删除了正文中的 callout 块
    pub callout_removed: bool,
    /// meta 与 callout 数量不一致时未删正文 callout，用户可手动清理
    pub orphan_callout_may_remain: bool,
}

pub fn remove_thought_from_markdown(markdown: &str, thought_id: &str) -> Result<RemoveThoughtOutcome, String> {
    if thought_id.is_empty() {
        return Err("thought_id is empty".to_string());
    }
    const UNCLOSED_MSG: &str =
        "Unclosed YAML frontmatter: add a closing --- line before saving a thought.";
    const NO_FM_MSG: &str = "笔记缺少闭合的 YAML frontmatter，无法删除想法。";

    let (yaml_src, body, leading_prefix) = match split_frontmatter(markdown) {
        FrontmatterSplit::Unclosed(_) => return Err(UNCLOSED_MSG.to_string()),
        FrontmatterSplit::NoFence(_) => return Err(NO_FM_MSG.to_string()),
        FrontmatterSplit::Closed {
            yaml,
            body,
            leading_prefix,
        } => (yaml, body, leading_prefix),
    };

    let (meta_vec_before, _) = parse_kf_thoughts_from_yaml(&yaml_src);
    let idx = meta_vec_before
        .iter()
        .position(|m| m.id == thought_id)
        .ok_or_else(|| format!("YAML 中不存在 thought_id {thought_id}"))?;

    let blocks = parse_thought_blocks(&body);
    let aligned = meta_vec_before.len() == blocks.len();
    let mut callout_removed = false;
    let new_body = if aligned && idx < blocks.len() {
        let b = &blocks[idx];
        callout_removed = true;
        remove_line_range_inclusive(&body, b.start_line, b.end_line)
    } else {
        body.to_string()
    };
    let orphan_callout_may_remain = !callout_removed && !blocks.is_empty();

    let mut meta_vec = meta_vec_before;
    meta_vec.remove(idx);
    let new_yaml = replace_kf_thoughts_sequence_in_yaml(&yaml_src, &meta_vec)?;
    let markdown_out = format!(
        "{}{}",
        leading_prefix,
        rebuild_with_frontmatter(&new_yaml, &new_body)
    );
    Ok(RemoveThoughtOutcome {
        markdown: markdown_out,
        removed: true,
        callout_removed,
        orphan_callout_may_remain,
    })
}

/// 在 Markdown body 的指定位置插入 thought callout。
/// `after_line`：0-based 行号（在该行之后插入），`None` 表示追加到末尾。
/// 返回 `(新 body, 插入起始行号 1-based)`。
pub fn insert_callout_into_body(
    body: &str,
    content: &str,
    maturity: ThoughtMaturity,
    after_line: Option<usize>,
) -> (String, usize) {
    let callout = build_thought_callout(content, maturity);
    let mut lines: Vec<&str> = body.lines().collect();
    // 保留尾部空行特性
    let trailing_newline = body.ends_with('\n');

    match after_line {
        Some(line_0based) => {
            let insert_pos = (line_0based + 1).min(lines.len());
            // 插入空行 + callout + 空行
            let callout_lines: Vec<&str> = callout.lines().collect();
            let mut new_lines = Vec::with_capacity(lines.len() + callout_lines.len() + 2);
            new_lines.extend_from_slice(&lines[..insert_pos]);
            new_lines.push("");
            for cl in &callout_lines {
                new_lines.push(cl);
            }
            new_lines.push("");
            new_lines.extend_from_slice(&lines[insert_pos..]);
            let inserted_at = insert_pos + 2; // 1-based, after the blank line
            let mut result = new_lines.join("\n");
            if trailing_newline && !result.ends_with('\n') {
                result.push('\n');
            }
            (result, inserted_at)
        }
        None => {
            // 追加到末尾
            let insert_at = lines.len() + 2; // 1-based, after blank line
            if !lines.is_empty() && !lines.last().map_or(true, |l| l.is_empty()) {
                lines.push("");
            }
            let callout_lines: Vec<&str> = callout.lines().collect();
            for cl in &callout_lines {
                lines.push(cl);
            }
            lines.push("");
            let mut result = lines.join("\n");
            if !result.ends_with('\n') {
                result.push('\n');
            }
            (result, insert_at)
        }
    }
}

// --- IPC 辅助类型 ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseNoteThoughtsResponse {
    pub blocks: Vec<ParsedThoughtBlock>,
    pub meta: Vec<KfThoughtMeta>,
    /// 解析 kf-thoughts 时的非致命告警（如 YAML 结构错误）；空则省略序列化。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub yaml_warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertThoughtArgs {
    pub rel_path: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub temporary: bool,
    /// 0-based body line number; None = append
    #[serde(default)]
    pub after_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertThoughtResponse {
    pub thought_id: String,
    pub inserted_at_line: usize,
}

/// 从完整 Markdown 解析 `kf-thoughts`；**无 Vault 根目录时正文摘录为空**（仅元数据，供轻量测试）。
pub fn parse_note_thoughts_from_markdown(markdown: &str) -> ParseNoteThoughtsResponse {
    parse_note_thoughts_inner(markdown, None, None)
}

/// 生产路径：正文从侧车 SQLite 按 `thought_id` 加载，并在有 `kfVaultNoteId` 时刷新 `note_rel_path` 缓存。
pub fn parse_note_thoughts_for_workspace(
    vault_root: &Path,
    rel_path: &str,
    markdown: &str,
) -> ParseNoteThoughtsResponse {
    parse_note_thoughts_inner(markdown, Some(vault_root), Some(rel_path))
}

fn parse_note_thoughts_inner(
    markdown: &str,
    vault_root: Option<&Path>,
    rel_path: Option<&str>,
) -> ParseNoteThoughtsResponse {
    let mut extra_warnings = Vec::new();
    let yaml_src = match split_frontmatter(markdown) {
        FrontmatterSplit::NoFence(_) => None,
        FrontmatterSplit::Unclosed(_) => {
            extra_warnings.push(
                "Unclosed YAML frontmatter: closing --- missing; kf-thoughts not loaded.".to_string(),
            );
            None
        }
        FrontmatterSplit::Closed { yaml, .. } => Some(yaml),
    };

    let (meta, mut yaml_warnings) = match yaml_src.as_deref() {
        Some(y) => parse_kf_thoughts_from_yaml(y),
        None => (Vec::new(), Vec::new()),
    };
    yaml_warnings.append(&mut extra_warnings);

    let blocks: Vec<ParsedThoughtBlock> = match (vault_root, rel_path) {
        (Some(root), Some(rel)) => match vault_thoughts_db::open_thoughts_db(root) {
            Ok(conn) => {
                if let Some(ref y) = yaml_src {
                    if let Some(sid) = parse_kf_vault_note_id(y) {
                        let _ = vault_thoughts_db::refresh_rel_path_for_stable_id(&conn, &sid, rel);
                    }
                }
                meta
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let raw = vault_thoughts_db::get_body(&conn, &m.id)
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let excerpt = build_excerpt_from_body_text(&raw, 200);
                        ParsedThoughtBlock {
                            id: m.id.clone(),
                            maturity: m.maturity,
                            excerpt,
                            start_line: i.saturating_add(1),
                            end_line: i.saturating_add(1),
                            temporary: m.temporary,
                        }
                    })
                    .collect()
            }
            Err(e) => {
                yaml_warnings.push(format!("想法侧车: {e}"));
                synthetic_blocks_from_meta(&meta)
            }
        },
        _ => synthetic_blocks_from_meta(&meta),
    };

    ParseNoteThoughtsResponse {
        blocks,
        meta,
        yaml_warnings,
    }
}

fn synthetic_blocks_from_meta(meta: &[KfThoughtMeta]) -> Vec<ParsedThoughtBlock> {
    meta
        .iter()
        .enumerate()
        .map(|(i, m)| ParsedThoughtBlock {
            id: m.id.clone(),
            maturity: m.maturity,
            excerpt: String::new(),
            start_line: i.saturating_add(1),
            end_line: i.saturating_add(1),
            temporary: m.temporary,
        })
        .collect()
}

/// 在 Markdown 中插入新想法：**仅更新 YAML + 侧车 SQLite**，不改写正文 callout。
/// `after_line` 保留参数以兼容 IPC，侧车方案下忽略。
pub fn insert_thought_into_markdown(
    vault_root: &Path,
    rel_path: &str,
    markdown: &str,
    content: &str,
    temporary: bool,
    _after_line: Option<usize>,
    existing_count: usize,
) -> Result<(String, InsertThoughtResponse), String> {
    vault_thoughts_db::validate_body_text(content)?;

    const UNCLOSED_MSG: &str =
        "Unclosed YAML frontmatter: add a closing --- line before saving a thought.";

    let (fm_yaml, body, leading_prefix) = match split_frontmatter(markdown) {
        FrontmatterSplit::Unclosed(_) => return Err(UNCLOSED_MSG.to_string()),
        FrontmatterSplit::NoFence(body) => (None, body, String::new()),
        FrontmatterSplit::Closed {
            yaml,
            body,
            leading_prefix,
        } => (Some(yaml), body, leading_prefix),
    };

    let thought_id = generate_thought_id(existing_count);
    let meta = new_thought_meta(&thought_id, temporary, "manual");

    let yaml_base = fm_yaml.as_deref().unwrap_or("");
    let yaml_with_id = ensure_kf_vault_note_id_in_yaml(yaml_base)?;
    let new_yaml = upsert_kf_thought_in_yaml(&yaml_with_id, &meta);

    let note_sid = parse_kf_vault_note_id(&new_yaml)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "kfVaultNoteId 缺失：无法写入侧车".to_string())?;

    let conn = vault_thoughts_db::open_thoughts_db(vault_root)?;
    vault_thoughts_db::upsert_thought_body(
        &conn,
        &thought_id,
        &note_sid,
        rel_path,
        content,
        None,
        thought_maturity_as_str(ThoughtMaturity::Seedling),
        temporary,
        false,
        &meta.created,
        &meta.updated,
        meta.challenge_pass_count,
        meta.last_reviewed_at.as_deref(),
    )?;

    let new_markdown = format!(
        "{}{}",
        leading_prefix,
        rebuild_with_frontmatter(&new_yaml, &body)
    );

    let resp = InsertThoughtResponse {
        thought_id,
        inserted_at_line: 1,
    };
    Ok((new_markdown, resp))
}

/// 对比保存前后 Markdown，找出 kf-thoughts 中成熟度**严格升高**的条目（用于 `write_markdown_file` 派发事件）。
pub fn detect_thought_maturity_promotions(
    rel_path: &str,
    old_markdown: &str,
    new_markdown: &str,
) -> Vec<ThoughtMaturityChangedPayload> {
    fn snapshot_by_id(md: &str) -> HashMap<String, ThoughtMaturity> {
        let r = parse_note_thoughts_from_markdown(md);
        r.meta
            .iter()
            .filter(|m| !m.id.is_empty())
            .map(|m| (m.id.clone(), m.maturity))
            .collect()
    }

    let old_map = snapshot_by_id(old_markdown);
    let new_map = snapshot_by_id(new_markdown);
    let mut out = Vec::new();
    for (id, new_mat) in new_map {
        let Some(old_mat) = old_map.get(&id) else {
            continue;
        };
        if thought_maturity_rank(new_mat) > thought_maturity_rank(*old_mat) {
            out.push(ThoughtMaturityChangedPayload {
                rel_path: rel_path.to_string(),
                thought_id: id,
                from_maturity: thought_maturity_as_str(*old_mat).to_string(),
                to_maturity: thought_maturity_as_str(new_mat).to_string(),
                start_line: 1,
            });
        }
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendAiThoughtReferenceArgs {
    pub rel_path: String,
    pub thought_id: String,
    pub context: String,
    pub relevance: String,
}

/// 向指定 thought 的 `references[]` 追加一条 AI 引用记录（不写 callout 正文）。
pub fn append_ai_thought_reference_to_markdown(
    markdown: &str,
    thought_id: &str,
    context: &str,
    relevance: &str,
) -> Result<String, String> {
    if thought_id.is_empty() {
        return Err("thought_id is empty".to_string());
    }

    const UNCLOSED_MSG: &str =
        "Unclosed YAML frontmatter: add a closing --- line before saving a thought.";
    const NO_FM_MSG: &str =
        "笔记缺少闭合的 YAML frontmatter，无法追加 thought 引用记录。";

    let (yaml_src, body, leading_prefix) = match split_frontmatter(markdown) {
        FrontmatterSplit::Unclosed(_) => return Err(UNCLOSED_MSG.to_string()),
        FrontmatterSplit::NoFence(_) => return Err(NO_FM_MSG.to_string()),
        FrontmatterSplit::Closed {
            yaml,
            body,
            leading_prefix,
        } => (yaml, body, leading_prefix),
    };

    let (mut meta_vec, _warnings) = parse_kf_thoughts_from_yaml(&yaml_src);
    let idx = meta_vec
        .iter()
        .position(|m| m.id == thought_id)
        .ok_or_else(|| format!("找不到 id 为 {thought_id} 的 thought 元数据"))?;

    let now_rfc = Utc::now().to_rfc3339();
    let m = &mut meta_vec[idx];
    m.references.push(ThoughtReference {
        date: now_rfc.clone(),
        context: context.chars().take(4000).collect(),
        relevance: relevance.chars().take(500).collect(),
    });
    m.updated = now_rfc;
    let new_yaml = upsert_kf_thought_in_yaml(&yaml_src, m);
    Ok(format!(
        "{}{}",
        leading_prefix,
        rebuild_with_frontmatter(&new_yaml, &body)
    ))
}

/// 挑战回顾「通过」时写回：递增 YAML 元数据 + 更新侧车 SQLite；**不改写正文 callout**。
///
/// `passed == false` 时原文不变（跳过或敷衍时不写 `last_reviewed_at`）。
pub fn apply_challenge_pass_to_markdown_vault(
    vault_root: &Path,
    _rel_path: &str,
    markdown: &str,
    thought_id: &str,
    passed: bool,
) -> Result<ApplyChallengePassToMarkdownOutcome, String> {
    if !passed {
        return Ok(ApplyChallengePassToMarkdownOutcome {
            markdown: markdown.to_string(),
            maturity_change: None,
        });
    }
    if thought_id.is_empty() {
        return Err("thought_id is empty".to_string());
    }

    const UNCLOSED_MSG: &str =
        "Unclosed YAML frontmatter: add a closing --- line before saving a thought.";
    const NO_FM_MSG: &str =
        "笔记缺少闭合的 YAML frontmatter，无法写回挑战回顾状态。";

    let (yaml_src, body, leading_prefix) = match split_frontmatter(markdown) {
        FrontmatterSplit::Unclosed(_) => return Err(UNCLOSED_MSG.to_string()),
        FrontmatterSplit::NoFence(_) => return Err(NO_FM_MSG.to_string()),
        FrontmatterSplit::Closed {
            yaml,
            body,
            leading_prefix,
        } => (yaml, body, leading_prefix),
    };

    let (mut meta_vec, _warnings) = parse_kf_thoughts_from_yaml(&yaml_src);
    if meta_vec.is_empty() {
        return Err("frontmatter 中无 kf-thoughts，无法定位 thought。".to_string());
    }

    let idx = meta_vec
        .iter()
        .position(|m| m.id == thought_id)
        .ok_or_else(|| format!("找不到 id 为 {thought_id} 的 thought 元数据"))?;

    let m = &mut meta_vec[idx];
    let prev_maturity = m.maturity;
    m.challenge_pass_count = m.challenge_pass_count.saturating_add(1);
    let pass_count = m.challenge_pass_count;
    m.last_reviewed_at = Some(Utc::now().format("%Y-%m-%d").to_string());
    m.maturity = maturity_after_challenge_pass(prev_maturity, pass_count);
    let maturity_change = if prev_maturity != m.maturity {
        Some(ThoughtMaturityChangedCore {
            thought_id: thought_id.to_string(),
            from_maturity: thought_maturity_as_str(prev_maturity).to_string(),
            to_maturity: thought_maturity_as_str(m.maturity).to_string(),
            start_line: idx.saturating_add(1),
        })
    } else {
        None
    };
    let now_rfc = Utc::now().to_rfc3339();
    m.updated = now_rfc.clone();
    m.history.push(ThoughtHistoryEntry {
        date: now_rfc,
        entry_type: "challenge-review-pass".to_string(),
        source: "challenge-review".to_string(),
        diff_summary: None,
    });

    let new_yaml = upsert_kf_thought_in_yaml(&yaml_src, m);

    let conn = vault_thoughts_db::open_thoughts_db(vault_root)?;
    vault_thoughts_db::update_thought_after_challenge(
        &conn,
        thought_id,
        thought_maturity_as_str(m.maturity),
        &m.updated,
        m.challenge_pass_count,
        m.last_reviewed_at.as_deref(),
    )?;

    Ok(ApplyChallengePassToMarkdownOutcome {
        markdown: format!(
            "{}{}",
            leading_prefix,
            rebuild_with_frontmatter(&new_yaml, &body)
        ),
        maturity_change,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_thoughts_db;
    use tempfile::tempdir;

    #[test]
    fn parse_single_thought_block() {
        let md = r#"# My Note

> [!thought] 随手想法 🌱
> This is my first understanding.
> It spans two lines.

Some other text.
"#;
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].maturity, ThoughtMaturity::Seedling);
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 5);
        assert!(blocks[0].excerpt.contains("This is my first understanding."));
        assert!(blocks[0].excerpt.contains("It spans two lines."));
    }

    #[test]
    fn parse_growing_maturity() {
        let md = "> [!thought] 成长想法 🌿\n> Growing thought here.\n";
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].maturity, ThoughtMaturity::Growing);
    }

    #[test]
    fn parse_mature_maturity() {
        let md = "> [!thought] 成熟想法 🌳\n> Mature line.\n";
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].maturity, ThoughtMaturity::Mature);
    }

    #[test]
    fn parse_kf_thoughts_challenge_fields_defaults() {
        let yaml = r#"
kf-thoughts:
  - id: t1
    maturity: growing
    created: "2026-04-10T10:00:00Z"
    updated: "2026-04-10T10:00:00Z"
    temporary: false
    history: []
"#;
        let (thoughts, w) = parse_kf_thoughts_from_yaml(yaml);
        assert!(w.is_empty());
        assert_eq!(thoughts[0].challenge_pass_count, 0);
        assert!(thoughts[0].last_reviewed_at.is_none());
    }

    #[test]
    fn maturity_after_challenge_pass_growing_to_mature() {
        assert_eq!(
            maturity_after_challenge_pass(ThoughtMaturity::Growing, 1),
            ThoughtMaturity::Mature
        );
        assert_eq!(
            maturity_after_challenge_pass(ThoughtMaturity::Growing, 0),
            ThoughtMaturity::Growing
        );
        assert_eq!(
            maturity_after_challenge_pass(ThoughtMaturity::Seedling, 1),
            ThoughtMaturity::Seedling
        );
    }

    #[test]
    fn parse_multiple_thought_blocks() {
        let md = r#"> [!thought] A 🌱
> First

Normal paragraph

> [!thought] B 🌿
> Second
"#;
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].maturity, ThoughtMaturity::Seedling);
        assert_eq!(blocks[1].maturity, ThoughtMaturity::Growing);
    }

    #[test]
    fn parse_no_thought_blocks() {
        let md = "# Hello\n\n> This is a normal blockquote.\n";
        let blocks = parse_thought_blocks(md);
        assert!(blocks.is_empty());
    }

    /// 围栏内 ``> [!thought]`` 仅为文档示例，不得计入想法块（产品说明类 Markdown 常见）
    #[test]
    fn ignore_thought_like_lines_inside_fenced_code_block() {
        let md = r#"# Doc

```markdown
> [!thought] 随手想法 🌱
> example body line

> [!thought|growing] 标题 🌿
> more
```

After fence, real thought:

> [!thought] 真想法 🌱
> only this counts
"#;
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].excerpt.contains("only this counts"));
        assert!(!blocks[0].excerpt.contains("example body"));
    }

    /// CommonMark 转义：`[` 写作 `\[`，磁盘上常见 `> \[!thought]`
    #[test]
    fn parse_escaped_bracket_thought_callout() {
        let md = "> \\[!thought] 随手想法 🌱\n> body line\n";
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].maturity, ThoughtMaturity::Seedling);
        assert!(blocks[0].excerpt.contains("body line"));
    }

    #[test]
    fn parse_callout_pipe_temporary_flag() {
        let md = "> [!thought|temporary] title\n> body line\n";
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].temporary);
    }

    #[test]
    fn parse_escaped_bracket_temporary_pipe() {
        let md = "> \\[!thought|temporary] t\n> x\n";
        let blocks = parse_thought_blocks(md);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].temporary);
    }

    #[test]
    fn reject_thoughtful_not_thought_type() {
        assert!(parse_thought_blocks("> [!thoughtful] x\n> body\n").is_empty());
        assert!(parse_thought_blocks("> \\[!thoughtful] x\n> body\n").is_empty());
    }

    #[test]
    fn parse_note_aligns_temporary_from_yaml() {
        let md = "---\nkf-thoughts:\n- id: t1\n  maturity: seedling\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: true\n---\n# Note\n\n> [!thought] 🌱\n> Block\n";
        let resp = parse_note_thoughts_from_markdown(md);
        assert_eq!(resp.blocks.len(), 1);
        assert!(resp.blocks[0].temporary);
        assert!(resp.meta[0].temporary);
    }

    #[test]
    fn excerpt_truncation() {
        let long_content = "a".repeat(300);
        let excerpt = build_excerpt(&[long_content.as_str()], 200);
        assert!(excerpt.len() <= 203 + "...".len()); // 200 chars + "..."
        assert!(excerpt.ends_with("..."));
    }

    #[test]
    fn parse_kf_thoughts_yaml() {
        let yaml = r#"
kf-private: false
kf-thoughts:
  - id: "thought-20260410-001"
    maturity: seedling
    created: "2026-04-10T10:00:00Z"
    updated: "2026-04-10T10:00:00Z"
    temporary: false
    history:
      - date: "2026-04-10T10:00:00Z"
        type: "created"
        source: "manual"
"#;
        let (thoughts, warnings) = parse_kf_thoughts_from_yaml(yaml);
        assert!(warnings.is_empty());
        assert_eq!(thoughts.len(), 1);
        assert_eq!(thoughts[0].id, "thought-20260410-001");
        assert_eq!(thoughts[0].maturity, ThoughtMaturity::Seedling);
    }

    #[test]
    fn parse_kf_thoughts_empty() {
        let (thoughts, warnings) = parse_kf_thoughts_from_yaml("");
        assert!(warnings.is_empty());
        assert!(thoughts.is_empty());
        let (thoughts2, w2) = parse_kf_thoughts_from_yaml("title: hello");
        assert!(w2.is_empty());
        assert!(thoughts2.is_empty());
    }

    #[test]
    fn parse_kf_thoughts_invalid_array_yields_warning() {
        let yaml = "kf-thoughts: not-an-array\n";
        let (thoughts, warnings) = parse_kf_thoughts_from_yaml(yaml);
        assert!(thoughts.is_empty());
        assert!(!warnings.is_empty());
    }

    #[test]
    fn build_thought_callout_with_content() {
        let callout = build_thought_callout("My thought", ThoughtMaturity::Seedling);
        assert!(callout.starts_with("> [!thought] 随手想法 🌱"));
        assert!(callout.contains("> My thought"));
    }

    #[test]
    fn build_thought_callout_empty() {
        let callout = build_thought_callout("", ThoughtMaturity::Growing);
        assert!(callout.contains("🌿"));
        assert!(callout.ends_with("> "));
    }

    #[test]
    fn build_thought_callout_mature_emoji() {
        let callout = build_thought_callout("x", ThoughtMaturity::Mature);
        assert!(callout.contains("🌳"));
    }

    #[test]
    fn generate_thought_id_format() {
        let id = generate_thought_id(5);
        assert!(id.starts_with("thought-"));
        assert!(id.ends_with("-005"));
    }

    // --- split_frontmatter tests ---

    #[test]
    fn split_frontmatter_present() {
        let md = "---\ntitle: Hello\nkf-private: false\n---\n# Body\nParagraph.\n";
        match split_frontmatter(md) {
            FrontmatterSplit::Closed {
                yaml,
                body,
                leading_prefix,
            } => {
                assert!(leading_prefix.is_empty());
                assert!(yaml.contains("title: Hello"));
                assert!(body.contains("# Body"));
                assert!(body.contains("Paragraph."));
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn split_frontmatter_absent() {
        let md = "# No frontmatter\nJust text.\n";
        match split_frontmatter(md) {
            FrontmatterSplit::NoFence(s) => assert_eq!(s, md),
            _ => panic!("expected NoFence"),
        }
    }

    #[test]
    fn split_frontmatter_unclosed() {
        let md = "---\ntitle: Bad\nNo closing fence\n";
        match split_frontmatter(md) {
            FrontmatterSplit::Unclosed(s) => assert_eq!(s, md),
            _ => panic!("expected Unclosed"),
        }
    }

    #[test]
    fn split_frontmatter_preserves_leading_prefix() {
        let md = "  \n---\ntitle: X\n---\n# Body\n";
        match split_frontmatter(md) {
            FrontmatterSplit::Closed {
                yaml,
                body,
                leading_prefix,
            } => {
                assert_eq!(leading_prefix, "  \n");
                assert!(yaml.contains("title: X"));
                assert!(body.contains("# Body"));
            }
            _ => panic!("expected Closed"),
        }
    }

    /// 正文中的水平线 `---` 不得当作 YAML 围栏（避免把后续大段当 YAML 解析并报错）
    #[test]
    fn split_frontmatter_mid_document_horizontal_rule_is_not_frontmatter() {
        let md = "# Design doc\n\n---\n\n## Section\nnot: yaml\n---\nReal body\n";
        match split_frontmatter(md) {
            FrontmatterSplit::NoFence(s) => assert_eq!(s, md),
            other => panic!("expected NoFence, got {other:?}"),
        }
    }

    /// UTF-8 BOM 后紧跟合法 frontmatter 仍应识别
    #[test]
    fn split_frontmatter_accepts_bom_only_before_opening_fence() {
        let md = format!("\u{feff}---\ntitle: Z\n---\n# Hi\n");
        match split_frontmatter(&md) {
            FrontmatterSplit::Closed { yaml, body, .. } => {
                assert!(yaml.contains("title: Z"));
                assert!(body.contains("# Hi"));
            }
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    // --- upsert_kf_thought_in_yaml tests ---

    #[test]
    fn upsert_adds_to_empty_yaml() {
        let meta = new_thought_meta("thought-test-001", false, "manual");
        let result = upsert_kf_thought_in_yaml("", &meta);
        assert!(result.contains("kf-thoughts"));
        assert!(result.contains("thought-test-001"));
    }

    #[test]
    fn upsert_adds_to_existing_yaml() {
        let yaml = "kf-private: false\n";
        let meta = new_thought_meta("thought-test-002", true, "ai-save");
        let result = upsert_kf_thought_in_yaml(yaml, &meta);
        assert!(result.contains("kf-private"));
        assert!(result.contains("kf-thoughts"));
        assert!(result.contains("thought-test-002"));
    }

    #[test]
    fn upsert_deduplicates_by_id() {
        let yaml = "kf-thoughts:\n- id: thought-dup\n  maturity: seedling\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n";
        let meta = new_thought_meta("thought-dup", false, "manual");
        let result = upsert_kf_thought_in_yaml(yaml, &meta);
        // Should contain only one entry with id thought-dup
        let count = result.matches("thought-dup").count();
        // id appears once in the entry
        assert!(count >= 1, "should have the id present");
    }

    // --- rebuild_with_frontmatter tests ---

    #[test]
    fn rebuild_roundtrip() {
        let yaml = "title: Hello";
        let body = "# Body\nText\n";
        let rebuilt = rebuild_with_frontmatter(yaml, body);
        assert!(rebuilt.starts_with("---\n"));
        assert!(rebuilt.contains("title: Hello"));
        assert!(rebuilt.contains("---\n# Body"));
    }

    // --- insert_callout_into_body tests ---

    #[test]
    fn insert_callout_at_end() {
        let body = "# Note\n\nSome text.\n";
        let (new_body, _line) = insert_callout_into_body(
            body, "My thought", ThoughtMaturity::Seedling, None,
        );
        assert!(new_body.contains("> [!thought]"));
        assert!(new_body.contains("> My thought"));
    }

    #[test]
    fn insert_callout_after_line() {
        let body = "Line 0\nLine 1\nLine 2\n";
        let (new_body, _line) = insert_callout_into_body(
            body, "Inserted", ThoughtMaturity::Growing, Some(1),
        );
        let lines: Vec<&str> = new_body.lines().collect();
        // Line 0, Line 1, blank, callout..., blank, Line 2
        assert_eq!(lines[0], "Line 0");
        assert_eq!(lines[1], "Line 1");
        assert_eq!(lines[2], ""); // blank separator
        assert!(lines[3].contains("[!thought]"));
    }

    // --- parse_note_thoughts（侧车正文） tests ---

    #[test]
    fn parse_note_thoughts_with_fm_and_sidecar_body() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "---\nkfVaultNoteId: nid-test-1\nkf-thoughts:\n- id: t1\n  maturity: seedling\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n---\n# Note\n\nSalad.\n";
        let conn = vault_thoughts_db::open_thoughts_db(root).unwrap();
        vault_thoughts_db::upsert_thought_body(
            &conn,
            "t1",
            "nid-test-1",
            "n.md",
            "Block content\nMore",
            None,
            "seedling",
            false,
            false,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            0,
            None,
        )
        .unwrap();
        let resp = parse_note_thoughts_for_workspace(root, "n.md", md);
        assert_eq!(resp.meta.len(), 1);
        assert_eq!(resp.meta[0].id, "t1");
        assert_eq!(resp.blocks.len(), 1);
        assert_eq!(resp.blocks[0].id, "t1");
        assert!(resp.blocks[0].excerpt.contains("Block content"));
    }

    // --- insert_thought_into_markdown tests ---

    #[test]
    fn insert_thought_writes_yaml_and_sidecar_not_callout() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "# Simple note\n\nParagraph.\n";
        let (new_md, resp) =
            insert_thought_into_markdown(root, "x.md", md, "My idea", false, None, 0).unwrap();
        assert!(new_md.contains("---\n"));
        assert!(new_md.contains("kf-thoughts"));
        assert!(new_md.contains("kfVaultNoteId"));
        assert!(resp.thought_id.starts_with("thought-"));
        assert!(!new_md.contains("> [!thought]"));
        let p = parse_note_thoughts_for_workspace(root, "x.md", &new_md);
        assert!(p.blocks[0].excerpt.contains("My idea"));
    }

    #[test]
    fn insert_thought_temporary_roundtrip_parse() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "# N\n\n";
        let (new_md, _) =
            insert_thought_into_markdown(root, "n.md", md, "draft", true, None, 0).unwrap();
        let resp = parse_note_thoughts_for_workspace(root, "n.md", &new_md);
        assert_eq!(resp.blocks.len(), 1);
        assert!(resp.blocks[0].temporary);
        assert!(resp.meta[0].temporary);
    }

    #[test]
    fn insert_thought_preserves_leading_prefix() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = " \t\n---\nk: 1\n---\n\nIntro\n";
        let (new_md, _) =
            insert_thought_into_markdown(root, "p.md", md, "idea", false, None, 0).unwrap();
        assert!(new_md.starts_with(" \t\n"));
        assert!(new_md.contains("Intro"));
    }

    #[test]
    fn insert_thought_rejects_unclosed_frontmatter() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "---\ntitle: x\nno closing\n";
        let err = insert_thought_into_markdown(root, "u.md", md, "a", false, None, 0).unwrap_err();
        assert!(err.contains("Unclosed"));
    }

    #[test]
    fn parse_note_unclosed_emits_warning() {
        let md = "---\na: 1\nno close\n";
        let resp = parse_note_thoughts_from_markdown(md);
        assert!(resp.yaml_warnings.iter().any(|w| w.contains("Unclosed")));
        assert!(resp.meta.is_empty());
    }

    #[test]
    fn apply_challenge_pass_skipped_when_not_passed() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "---\nkfVaultNoteId: nx\nkf-thoughts:\n- id: t1\n  maturity: growing\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n---\nBody\n";
        let out = apply_challenge_pass_to_markdown_vault(root, "a.md", md, "t1", false).unwrap();
        assert_eq!(out.markdown, md);
        assert!(out.maturity_change.is_none());
    }

    #[test]
    fn apply_challenge_pass_updates_growing_to_mature() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let md = "---\nkfVaultNoteId: n2\nkf-thoughts:\n- id: thought-x\n  maturity: growing\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n---\nBody line\n";
        let conn = vault_thoughts_db::open_thoughts_db(root).unwrap();
        vault_thoughts_db::upsert_thought_body(
            &conn,
            "thought-x",
            "n2",
            "note.md",
            "hello",
            None,
            "growing",
            false,
            false,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            0,
            None,
        )
        .unwrap();
        let out =
            apply_challenge_pass_to_markdown_vault(root, "note.md", md, "thought-x", true).unwrap();
        assert!(out.markdown.contains("mature"), "{}", out.markdown);
        assert!(out.markdown.contains("challengePassCount: 1"), "{}", out.markdown);
        assert!(out.markdown.contains("lastReviewedAt"), "{}", out.markdown);
        let ch = out.maturity_change.expect("growing→mature should emit change");
        assert_eq!(ch.thought_id, "thought-x");
        assert_eq!(ch.from_maturity, "growing");
        assert_eq!(ch.to_maturity, "mature");
    }

    #[test]
    fn remove_thought_aligned_removes_yaml_and_callout() {
        let md = r#"---
kf-thoughts:
  - id: t-del-1
    maturity: seedling
    created: "2026-04-01T00:00:00Z"
    updated: "2026-04-01T00:00:00Z"
    temporary: false
---
# Note

> [!thought] 随手想法 🌱
> Only line.

Tail.
"#;
        let out = remove_thought_from_markdown(md, "t-del-1").unwrap();
        assert!(out.callout_removed);
        assert!(!out.orphan_callout_may_remain);
        assert!(!out.markdown.contains("t-del-1"));
        assert!(!out.markdown.contains("[!thought]"));
        assert!(out.markdown.contains("Tail."));
    }

    #[test]
    fn remove_thought_mismatch_keeps_callout_and_flags_orphan() {
        let md = r#"---
kf-thoughts:
  - id: t-a
    maturity: seedling
    created: "2026-04-01T00:00:00Z"
    updated: "2026-04-01T00:00:00Z"
    temporary: false
  - id: t-b
    maturity: seedling
    created: "2026-04-01T00:00:00Z"
    updated: "2026-04-01T00:00:00Z"
    temporary: false
---
> [!thought] X 🌱
> one block
"#;
        let out = remove_thought_from_markdown(md, "t-a").unwrap();
        assert!(!out.callout_removed);
        assert!(out.orphan_callout_may_remain);
        assert!(!out.markdown.contains("t-a"));
        assert!(out.markdown.contains("t-b"));
        assert!(out.markdown.contains("[!thought]"));
    }

    #[test]
    fn bump_kf_thought_updated_changes_timestamp() {
        let md = "---\nkf-thoughts:\n- id: tbump\n  maturity: seedling\n  created: '2026-01-01T00:00:00Z'\n  updated: '2026-01-01T00:00:00Z'\n  temporary: false\n---\nBody\n";
        let out = bump_kf_thought_updated_in_markdown(md, "tbump").unwrap();
        assert!(out.contains("tbump"));
        assert_ne!(out, md);
        assert!(out.contains("Body"));
    }
}
