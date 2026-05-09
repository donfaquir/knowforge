//! 文档级语义相似度 → 双向链接推荐候选（迭代 6.3 步骤 15）。
//! 无语义索引时不提供推荐（无关键词兜底）。

use crate::llm::ollama;
use crate::llm::LlmChatMessage;
use crate::note_privacy;
use crate::semantic_index::{self, DocChunkRow};
use crate::thought_parser::{split_frontmatter, FrontmatterSplit};
use crate::understanding_graph::{extract_wikilink_inners, normalize_markdown_rel_path, resolve_wikilink_inner_to_rel_path};
use crate::vault_config::{ActiveProvider, AiConfig};
use aho_corasick::AhoCorasick;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

/// 蓝图中的「已解析」AI 配置，与 `load_ai_config_internal` 返回的 `AiConfig` 同型
pub type ResolvedAiConfig = AiConfig;

/// 单条链接推荐（序列化字段名与前端 camelCase 对齐）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRecommendation {
    pub target_rel_path: String,
    pub score: f64,
    pub shared_topics: Vec<String>,
    pub existing_link: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 对同一文档下各 chunk 的向量取元素均值；维数不一致或空输入返回 None
pub fn compute_document_embedding(chunks: &[DocChunkRow]) -> Option<Vec<f32>> {
    if chunks.is_empty() {
        return None;
    }
    let dim = chunks.first()?.embedding.len();
    if dim == 0 {
        return None;
    }
    if !chunks.iter().all(|c| c.embedding.len() == dim) {
        return None;
    }
    let mut acc = vec![0f32; dim];
    for c in chunks {
        for (i, v) in c.embedding.iter().enumerate() {
            acc[i] += v;
        }
    }
    let n = chunks.len() as f32;
    for x in &mut acc {
        *x /= n;
    }
    Some(acc)
}

/// 索引里 `rel_path` 可能与编辑器传入的相对路径在 `.md` 省略、ASCII 大小写等处不一致；与 `current_norm` 同规则再比较/聚合
fn embedding_rel_key(rel: &str) -> String {
    normalize_markdown_rel_path(rel.trim())
}

fn mean_pooled_by_rel_path(rows: &[DocChunkRow]) -> HashMap<String, Vec<f32>> {
    let mut groups: HashMap<String, Vec<DocChunkRow>> = HashMap::new();
    for r in rows {
        let key = embedding_rel_key(&r.rel_path);
        groups.entry(key).or_default().push(r.clone());
    }
    let mut out = HashMap::new();
    for (rel, group) in groups {
        if let Some(v) = compute_document_embedding(&group) {
            out.insert(rel, v);
        }
    }
    out
}

fn wikilink_targets_for_source(source_rel: &str, markdown: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for inner in extract_wikilink_inners(markdown) {
        if let Some(to) = resolve_wikilink_inner_to_rel_path(source_rel, &inner) {
            set.insert(embedding_rel_key(&to));
        }
    }
    set
}

/// 与当前笔记展示「为何相关」的关联词上限（中英混合；中文仅 3–4 字且过滤切断碎片）
const LINK_OVERLAP_KEYWORD_MAX: usize = 3;
/// 文档级余弦相似度须**严格大于**该阈值才进入推荐（关联相似门槛）
const LINK_RECOMMENDATION_MIN_SIMILARITY: f32 = 0.7;
/// 单段连续汉字参与 n-gram 扫描长度上限，避免极长无标点段落拖慢
const LINK_CJK_RUN_SCAN_CAP: usize = 400;

/// 合并主表与 `vendor_*` 开源停用词子串后构建 Aho–Corasick（编译期嵌入）
fn ingest_cn_noise_lines(raw: &str, seen: &mut HashSet<String>, patterns: &mut Vec<String>) {
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.chars().count() < 2 {
            continue;
        }
        if !t.chars().all(|c| matches!(c as u32, 0x4e00..=0x9fff)) {
            continue;
        }
        if seen.insert(t.to_string()) {
            patterns.push(t.to_string());
        }
    }
}

/// 中文空泛子串：`link_rec_lexical_noise_cn.txt` + `vendor_cn_stopwords_for_link_rec.txt`
static CN_LEXICAL_NOISE: LazyLock<AhoCorasick> = LazyLock::new(|| {
    let mut seen = HashSet::new();
    let mut patterns: Vec<String> = Vec::new();
    const PRIMARY: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/link_rec_lexical_noise_cn.txt"));
    const VENDOR: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/vendor_cn_stopwords_for_link_rec.txt"
    ));
    ingest_cn_noise_lines(PRIMARY, &mut seen, &mut patterns);
    ingest_cn_noise_lines(VENDOR, &mut seen, &mut patterns);
    if patterns.is_empty() {
        patterns.extend(["核心", "大量"].iter().map(|s| (*s).to_string()));
    }
    AhoCorasick::new(patterns.iter().map(|p| p.as_str())).expect("cn lexical noise automaton")
});

fn ingest_en_noise_lines(raw: &str, set: &mut HashSet<String>) {
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if lower.len() < 3 || !lower.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        set.insert(lower);
    }
}

/// 英文空泛词：`link_rec_lexical_noise_en.txt` + `vendor_en_stopwords_for_link_rec.txt`（整词、小写）
static EN_LEXICAL_NOISE: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set = HashSet::new();
    const PRIMARY: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/link_rec_lexical_noise_en.txt"));
    const VENDOR: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/vendor_en_stopwords_for_link_rec.txt"
    ));
    ingest_en_noise_lines(PRIMARY, &mut set);
    ingest_en_noise_lines(VENDOR, &mut set);
    if set.is_empty() {
        set.insert("core".into());
        set.insert("many".into());
    }
    set
});

fn is_lexical_noise_token(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.chars().all(|c| c.is_ascii_alphabetic()) {
        return EN_LEXICAL_NOISE.contains(&s.to_ascii_lowercase());
    }
    if s.chars().all(is_cjk_unified) {
        let n = s.chars().count();
        if n <= 4 {
            return CN_LEXICAL_NOISE.is_match(s);
        }
    }
    false
}

fn markdown_body_for_overlap(md: &str) -> String {
    match split_frontmatter(md) {
        FrontmatterSplit::NoFence(s) | FrontmatterSplit::Unclosed(s) => s,
        FrontmatterSplit::Closed { body, .. } => body,
    }
}

fn strip_fenced_code_blocks(body: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            skip = !skip;
            continue;
        }
        if skip {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

fn remove_wikilink_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        if ch == '[' && it.peek() == Some(&'[') {
            it.next();
            let mut depth = 1usize;
            while let Some(c) = it.next() {
                if c == '[' && it.peek() == Some(&'[') {
                    it.next();
                    depth += 1;
                    continue;
                }
                if c == ']' && it.peek() == Some(&']') {
                    it.next();
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
            }
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn prepare_plain_for_overlap(md: &str) -> String {
    let body = markdown_body_for_overlap(md);
    let no_code = strip_fenced_code_blocks(&body);
    remove_wikilink_spans(&no_code)
}

fn is_cjk_unified(ch: char) -> bool {
    matches!(ch as u32, 0x4e00..=0x9fff)
}

/// 中文关联词仅保留 3～4 字，且排除「记忆的」「忆系统有」类切断：首尾/中间虚词、二字 n-gram 一律不要
fn is_clean_cjk_overlap_term(s: &str) -> bool {
    if !s.chars().all(is_cjk_unified) {
        return true;
    }
    let n = s.chars().count();
    if n < 3 || n > 4 {
        return false;
    }
    let chars: Vec<char> = s.chars().collect();
    const LEAD_BAD: &[char] = &['的', '地', '得', '了', '之', '所', '与', '而', '以', '于','有'];
    const TRAIL_BAD: &[char] = &[
        '的', '地', '得', '了', '着', '过', '与', '之', '中', '等', '吗', '呢', '吧', '啊', '呀', '所', '为', '以', '于','有'
    ];
    if LEAD_BAD.contains(&chars[0]) {
        return false;
    }
    if TRAIL_BAD.contains(chars.last().unwrap()) {
        return false;
    }
    if n == 3 && matches!(chars[1], '的' | '地' | '得') {
        return false;
    }
    if n == 4 && (chars[1] == '的' || chars[2] == '的') {
        return false;
    }
    true
}

fn is_en_stopword(w: &str) -> bool {
    const STOPS: &[&str] = &[
        "about", "after", "all", "also", "and", "any", "are", "been", "before", "being", "between", "both", "but",
        "can", "could", "day", "did", "each", "even", "for", "from", "get", "has", "have", "here", "him", "his",
        "how", "into", "its", "just", "let", "may", "more", "most", "new", "not", "now", "old", "one", "only",
        "our", "out", "over", "put", "same", "say", "see", "she", "should", "some", "such", "than", "that",
        "the", "their", "them", "then", "there", "these", "they", "this", "those", "too", "two", "under", "use",
        "very", "was", "way", "what", "when", "where", "which", "while", "who", "whom", "whose", "will", "with",
        "would", "you", "your",
    ];
    STOPS.binary_search(&w).is_ok()
}

fn collect_ascii_terms(s: &str, m: &mut HashMap<String, u32>) {
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphabetic() {
            cur.push(ch.to_ascii_lowercase());
        } else if (ch.is_ascii_digit() || ch == '_') && !cur.is_empty() {
            cur.push(ch);
        } else {
            flush_ascii_word(&mut cur, m);
        }
    }
    flush_ascii_word(&mut cur, m);
}

fn flush_ascii_word(cur: &mut String, m: &mut HashMap<String, u32>) {
    if cur.len() >= 3 && !is_en_stopword(cur.as_str()) && !is_lexical_noise_token(cur.as_str()) {
        *m.entry(cur.clone()).or_insert(0) += 1;
    }
    cur.clear();
}

fn flush_cjk_run(buf: &mut String, m: &mut HashMap<String, u32>) {
    let n_chars = buf.chars().count();
    if n_chars < 3 {
        buf.clear();
        return;
    }
    let run: String = buf.chars().take(LINK_CJK_RUN_SCAN_CAP).collect();
    buf.clear();
    let chars: Vec<char> = run.chars().collect();
    let len = chars.len();
    let max_n = 4usize.min(len);
    for ng in 3..=max_n {
        for w in chars.windows(ng) {
            let t: String = w.iter().collect();
            if !is_lexical_noise_token(&t) && is_clean_cjk_overlap_term(&t) {
                *m.entry(t).or_insert(0) += 1;
            }
        }
    }
}

fn collect_cjk_terms(s: &str, m: &mut HashMap<String, u32>) {
    let mut buf = String::new();
    for ch in s.chars() {
        if is_cjk_unified(ch) {
            buf.push(ch);
            if buf.chars().count() >= LINK_CJK_RUN_SCAN_CAP {
                flush_cjk_run(&mut buf, m);
            }
        } else {
            flush_cjk_run(&mut buf, m);
        }
    }
    flush_cjk_run(&mut buf, m);
}

fn term_frequencies(plain: &str) -> HashMap<String, u32> {
    let mut m = HashMap::new();
    collect_ascii_terms(plain, &mut m);
    collect_cjk_terms(plain, &mut m);
    m
}

/// 两文术语表交集：关联性（共现）+ 语义倾向（两文总频次越低略优先，弱化「系统有」类碎块）
fn top_overlap_terms(a: &HashMap<String, u32>, b: &HashMap<String, u32>, max_out: usize) -> Vec<String> {
    let mut scored: Vec<(u32, usize, String)> = Vec::new();
    for (k, &va) in a {
        if let Some(&vb) = b.get(k) {
            let n = k.chars().count();
            if k.chars().all(is_cjk_unified) {
                if n < 3 || n > 4 {
                    continue;
                }
                if !is_clean_cjk_overlap_term(k) {
                    continue;
                }
            } else if k.chars().all(|c| c.is_ascii_alphabetic()) && n < 3 {
                continue;
            }
            if is_lexical_noise_token(k) {
                continue;
            }
            let base = va.min(vb).saturating_mul((n * n) as u32);
            let tf_sum = va.saturating_add(vb).max(1);
            let denom = tf_sum.min(48).max(1);
            let spec = 24u32.saturating_div(denom).saturating_add(1).min(12);
            let score = base.saturating_mul(spec);
            scored.push((score, n, k.clone()));
        }
    }
    scored.sort_by(|x, y| {
        y.0.cmp(&x.0)
            .then_with(|| y.1.cmp(&x.1))
            .then_with(|| x.2.cmp(&y.2))
    });
    let mut out: Vec<String> = Vec::new();
    for (_, _, term) in scored {
        if out.len() >= max_out {
            break;
        }
        if is_lexical_noise_token(&term) {
            continue;
        }
        if term.chars().all(is_cjk_unified) && !is_clean_cjk_overlap_term(&term) {
            continue;
        }
        if out.iter().any(|e: &String| e != &term && e.contains(term.as_str())) {
            continue;
        }
        out.retain(|e: &String| !(e != &term && term.contains(e.as_str())));
        out.push(term);
    }
    out
}

fn read_note_markdown_for_overlap(vault_root: &Path, rel_key: &str, chunks: &[DocChunkRow]) -> String {
    let norm = embedding_rel_key(rel_key);
    if let Ok(abs) = crate::join_under_root(vault_root, &norm) {
        if abs.is_file() {
            if let Ok(s) = fs::read_to_string(&abs) {
                if !note_privacy::markdown_treat_as_kf_private(&s) {
                    return s;
                }
            }
        }
    }
    chunks
        .iter()
        .filter(|c| {
            let k = embedding_rel_key(&c.rel_path);
            k == norm || k.eq_ignore_ascii_case(&norm)
        })
        .map(|c| c.chunk_text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 基于语义索引为当前文档推荐可链接的其它笔记（已排除已有 WikiLink 目标）。
///
/// 仅保留文档级余弦相似度 **严格大于** `LINK_RECOMMENDATION_MIN_SIMILARITY` 的候选，再取前 `max_results` 条。
///
/// `shared_topics`：由当前文与候选文正文重叠词填充（非 LLM）。
///
/// `editor_markdown_override`：未保存缓冲区正文；有值时用于解析已有 WikiLink（避免仅读盘时与编辑器不一致）。
pub fn suggest_related_notes(
    vault_root: &Path,
    current_rel_path: &str,
    embedding_db: &Connection,
    _thoughts_db: &Connection,
    max_results: usize,
    editor_markdown_override: Option<&str>,
) -> Result<Vec<LinkRecommendation>, String> {
    if max_results == 0 {
        return Ok(Vec::new());
    }

    let current_rel_path = current_rel_path.trim().replace('\\', "/");
    note_privacy::validate_workspace_rel_path(&current_rel_path)?;

    let current_norm = embedding_rel_key(&current_rel_path);
    let abs = crate::join_under_root(vault_root, &current_norm)?;
    if !abs.is_file() {
        return Err(format!("semantic_index_not_ready: note file not found: {current_norm}"));
    }
    let markdown = match editor_markdown_override {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => fs::read_to_string(&abs).map_err(|e| format!("read note: {e}"))?,
    };
    if note_privacy::markdown_treat_as_kf_private(&markdown) {
        return Err("semantic_index_not_ready: kf-private notes are excluded from link recommendations".to_string());
    }

    let all_chunks = semantic_index::load_all_doc_embeddings(embedding_db)?;
    if all_chunks.is_empty() {
        return Err(
            "semantic_index_not_ready: no document chunks in embedding index; rebuild embeddings first"
                .to_string(),
        );
    }

    let current_chunks: Vec<DocChunkRow> = all_chunks
        .iter()
        .filter(|c| {
            let k = embedding_rel_key(&c.rel_path);
            k == current_norm || k.eq_ignore_ascii_case(&current_norm)
        })
        .cloned()
        .collect();

    let Some(current_vec) = compute_document_embedding(&current_chunks) else {
        return Err(format!(
            "semantic_index_not_ready: no embedding chunks indexed for `{current_norm}`"
        ));
    };

    let linked = wikilink_targets_for_source(&current_norm, &markdown);
    let doc_vecs = mean_pooled_by_rel_path(&all_chunks);

    let mut scored: Vec<(String, f32)> = doc_vecs
        .into_iter()
        .filter(|(rel, _)| rel != &current_norm)
        .filter(|(rel, _)| !linked.contains(rel))
        .map(|(rel, vec)| {
            let s = semantic_index::cosine_similarity(&current_vec, &vec);
            (rel, s)
        })
        .filter(|(_, s)| *s > LINK_RECOMMENDATION_MIN_SIMILARITY)
        .collect();

    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let source_plain = prepare_plain_for_overlap(&markdown);
    let source_tf = term_frequencies(&source_plain);

    let mut out: Vec<LinkRecommendation> = Vec::new();
    for (target_rel_path, sim) in scored.into_iter().take(max_results) {
        let target_md = read_note_markdown_for_overlap(vault_root, &target_rel_path, &all_chunks);
        let target_plain = prepare_plain_for_overlap(&target_md);
        let target_tf = term_frequencies(&target_plain);
        let shared_topics = top_overlap_terms(&source_tf, &target_tf, LINK_OVERLAP_KEYWORD_MAX);
        out.push(LinkRecommendation {
            target_rel_path,
            score: sim as f64,
            shared_topics,
            existing_link: false,
            reason: None,
        });
    }

    Ok(out)
}

/// 读取当前笔记正文（截断）供 `enrich_recommendations_with_reasons`；私密或读失败时返回空串（不报错中断命令）。
pub fn load_note_excerpt_for_reasons(vault_root: &Path, rel_path: &str) -> Result<String, String> {
    let rel = rel_path.replace('\\', "/");
    note_privacy::validate_workspace_rel_path(&rel)?;
    let norm = normalize_markdown_rel_path(&rel);
    let abs = crate::join_under_root(vault_root, &norm)?;
    if !abs.is_file() {
        return Ok(String::new());
    }
    let md = fs::read_to_string(&abs).map_err(|e| format!("read note: {e}"))?;
    if note_privacy::markdown_treat_as_kf_private(&md) {
        return Ok(String::new());
    }
    Ok(truncate_chars(md.trim(), LINK_REASON_EXCERPT_MAX_CHARS))
}

// --- LLM 推荐理由（步骤 16）：失败或超时仅跳过填充，不整批报错 ---

const LINK_REASON_EXCERPT_MAX_CHARS: usize = 2500;
const LINK_REASON_LLM_TIMEOUT_CAP_MS: u64 = 12_000;
const LINK_REASON_MAX_LEN: usize = 400;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkReasonItemJson {
    rel_path: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkReasonBatchJson {
    #[serde(default)]
    items: Vec<LinkReasonItemJson>,
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

/// 从模型输出中截取最外层 `{ ... }`（与写作教练一致思路）
fn extract_json_object(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let start = s
        .find('{')
        .ok_or_else(|| "model output did not contain JSON object".to_string())?;
    let end = s
        .rfind('}')
        .ok_or_else(|| "model output did not contain JSON object".to_string())?;
    if end < start {
        return Err("invalid JSON slice".to_string());
    }
    Ok(s[start..=end].to_string())
}

/// 将已解析的 JSON 对象正文合并进候选（仅白名单 `candidates` 中的 relPath）
fn merge_llm_reason_items(candidates: &mut [LinkRecommendation], json_object: &str) {
    let Ok(batch) = serde_json::from_str::<LinkReasonBatchJson>(json_object) else {
        return;
    };
    let allowed: HashSet<String> = candidates
        .iter()
        .map(|c| normalize_markdown_rel_path(&c.target_rel_path))
        .collect();
    let mut by_rel: HashMap<String, String> = HashMap::new();
    for it in batch.items {
        let rp = normalize_markdown_rel_path(it.rel_path.trim());
        if rp.is_empty() || !allowed.contains(&rp) {
            continue;
        }
        let r = it.reason.trim();
        if r.is_empty() {
            continue;
        }
        let r = truncate_chars(r, LINK_REASON_MAX_LEN);
        by_rel.insert(rp, r);
    }
    for c in candidates.iter_mut() {
        let key = normalize_markdown_rel_path(&c.target_rel_path);
        if let Some(r) = by_rel.get(&key) {
            c.reason = Some(r.clone());
        }
    }
}

const SYSTEM_LINK_REASONS: &str = r#"You suggest why two notes in a personal knowledge vault might be linked.
Output MUST be one JSON object only (no markdown fences, no prose outside JSON). Use exactly:
{ "items": [ { "relPath": string, "reason": string } ] }
Rules:
- "relPath" MUST match one of the candidate paths exactly (character-for-character as given).
- "reason": one short sentence (max ~40 words), informational only, no commands to the user.
- Only include notes you are reasonably confident about; omit uncertain pairs.
- Match the excerpt language when possible (Chinese excerpt → Chinese reasons)."#;

fn resolve_ollama_model_name(ai: &ResolvedAiConfig) -> Option<String> {
    ai.ollama
        .last_used_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let d = ai.ollama.default_model.trim();
            if d.is_empty() {
                None
            } else {
                Some(ai.ollama.default_model.clone())
            }
        })
}

/// 按当前激活提供商拉取一次非流式补全正文；失败或未实现时返回 None（调用方静默降级）。
async fn link_reason_completion_body(
    ai: &ResolvedAiConfig,
    messages: &[LlmChatMessage],
    timeout_ms: u64,
) -> Option<String> {
    match ai.active_provider {
        ActiveProvider::Ollama => {
            let model = resolve_ollama_model_name(ai)?;
            ollama::run_chat_completion(
                &ai.ollama.base_url,
                &model,
                messages,
                ai.parameters.temperature,
                ai.parameters.top_p,
                timeout_ms,
            )
            .await
            .ok()
        }
        ActiveProvider::Openai => {
            // 后续多模型：在此调用 OpenAI 兼容 `/v1/chat/completions`（与主对话配置对齐）
            None
        }
    }
}

/// 对已有候选调用 LLM 填充 `reason`；补全不可用、超时、解析失败时保持 `reason == None`（返回 `Ok(())`）。
///
/// `config` 与 `vault_config::load_ai_config_internal` 返回值同型（蓝图称 ResolvedConfig）。
pub async fn enrich_recommendations_with_reasons(
    candidates: &mut [LinkRecommendation],
    current_doc_excerpt: &str,
    config: &ResolvedAiConfig,
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }

    let excerpt = truncate_chars(current_doc_excerpt.trim(), LINK_REASON_EXCERPT_MAX_CHARS);
    let list_lines: String = candidates
        .iter()
        .map(|c| {
            format!(
                "- relPath: `{}` (similarity score {:.4})",
                c.target_rel_path.replace('\\', "/"),
                c.score
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let user_body = format!(
        "Current note excerpt:\n---\n{excerpt}\n---\n\nCandidate notes (use relPath exactly as listed):\n{list_lines}"
    );

    let timeout_ms = config
        .request
        .timeout_ms
        .min(LINK_REASON_LLM_TIMEOUT_CAP_MS)
        .max(3000);

    let msgs = vec![
        LlmChatMessage {
            role: "system".into(),
            content: SYSTEM_LINK_REASONS.to_string(),
        },
        LlmChatMessage {
            role: "user".into(),
            content: user_body,
        },
    ];

    let raw = match link_reason_completion_body(config, &msgs, timeout_ms).await {
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(());
            }
            t.to_string()
        }
        None => return Ok(()),
    };

    let slice = match extract_json_object(&raw) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    merge_llm_reason_items(candidates, &slice);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_config;

    fn row(rel: &str, idx: i32, emb: Vec<f32>) -> DocChunkRow {
        let dim = emb.len() as i32;
        DocChunkRow {
            chunk_id: format!("{rel}#{idx}"),
            rel_path: rel.to_string(),
            chunk_index: idx,
            chunk_text: String::new(),
            embedding: emb,
            dim,
            model_id: "test".into(),
        }
    }

    #[test]
    fn compute_document_embedding_empty() {
        assert!(compute_document_embedding(&[]).is_none());
    }

    #[test]
    fn compute_document_embedding_mean() {
        let a = row("a.md", 0, vec![1.0, 0.0, 0.0]);
        let b = row("a.md", 1, vec![0.0, 2.0, 0.0]);
        let m = compute_document_embedding(&[a, b]).unwrap();
        assert!((m[0] - 0.5).abs() < 1e-5);
        assert!((m[1] - 1.0).abs() < 1e-5);
        assert!((m[2] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn overlap_keywords_shared_english_terms() {
        let a = super::term_frequencies("Rust async programming guide");
        let b = super::term_frequencies("Async Rust design patterns");
        let v = super::top_overlap_terms(&a, &b, 8);
        assert!(v.contains(&"rust".to_string()));
        assert!(v.contains(&"async".to_string()));
    }

    #[test]
    fn overlap_keywords_shared_cjk_ngrams() {
        let a = super::term_frequencies("记忆管理与检索方案");
        let b = super::term_frequencies("笔记里对记忆管理的备忘");
        let v = super::top_overlap_terms(&a, &b, 10);
        assert!(
            v.iter().any(|s| s.as_str() == "记忆管理" || s.contains("记忆管理")),
            "{v:?}"
        );
    }

    #[test]
    fn clean_cjk_rejects_possessive_and_keeps_phrases() {
        assert!(!super::is_clean_cjk_overlap_term("记忆的"));
        assert!(!super::is_clean_cjk_overlap_term("忆的存"));
        assert!(super::is_clean_cjk_overlap_term("记忆系统"));
        assert!(super::is_clean_cjk_overlap_term("向量数据"));
    }

    /// 无句内「的」切断时仍能抽出稳定四字共现
    #[test]
    fn overlap_keywords_prefers_stable_fourgrams() {
        let a = super::term_frequencies("向量数据库检索与记忆系统调优");
        let b = super::term_frequencies("记忆系统与向量数据库部署备忘");
        let v = super::top_overlap_terms(&a, &b, 8);
        assert!(v.iter().any(|s| s.contains("向量数据") || s.contains("记忆系统")), "{v:?}");
    }

    #[test]
    fn overlap_keywords_filters_lexical_noise_cn() {
        let a = super::term_frequencies("核心能力大量涌现与业务增长");
        let b = super::term_frequencies("大量核心指标与业务增长");
        let v = super::top_overlap_terms(&a, &b, 12);
        assert!(!v.iter().any(|s| s == "核心" || s == "大量"));
        assert!(v.iter().any(|s| s.contains("业务")), "{v:?}");
    }

    #[test]
    fn overlap_keywords_filters_lexical_noise_en() {
        let a = super::term_frequencies("many core features in rust");
        let b = super::term_frequencies("lots of core ideas for rust");
        let v = super::top_overlap_terms(&a, &b, 12);
        assert!(!v.contains(&"core".to_string()));
        assert!(!v.contains(&"many".to_string()));
        assert!(v.contains(&"rust".to_string()), "{v:?}");
    }

    #[test]
    fn suggest_related_notes_orders_by_similarity_and_excludes_linked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("cur.md"), "x [[linked]]\n").unwrap();
        fs::write(root.join("linked.md"), "y").unwrap();
        fs::write(root.join("near.md"), "n").unwrap();
        // 与 cur 正交，余弦为 0，低于 LINK_RECOMMENDATION_MIN_SIMILARITY，不应出现
        fs::write(root.join("far.md"), "z").unwrap();
        fs::write(root.join("mid.md"), "m").unwrap();

        let conn = semantic_index::open_embedding_db(root).expect("open embedding db");

        let hi = vec![1.0_f32, 0.0, 0.0];
        let near = vec![0.9_f32, 0.1, 0.0];
        // L2≈1，与 [1,0,0] 余弦约 0.85，高于 0.7
        let mid = vec![0.85_f32, 0.526783f32, 0.0];
        let far = vec![0.0_f32, 1.0, 0.0];
        semantic_index::upsert_doc_chunk(&conn, "cur.md#0", "cur.md", 0, "t", &hi, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "near.md#0", "near.md", 0, "t", &near, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "mid.md#0", "mid.md", 0, "t", &mid, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "linked.md#0", "linked.md", 0, "t", &near, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "far.md#0", "far.md", 0, "t", &far, "m").unwrap();

        let tconn = Connection::open_in_memory().unwrap();
        let rec = suggest_related_notes(root, "cur.md", &conn, &tconn, 5, None).unwrap();
        assert_eq!(rec.len(), 2);
        assert_eq!(rec[0].target_rel_path, "near.md");
        assert_eq!(rec[1].target_rel_path, "mid.md");
        assert!(rec[0].score > rec[1].score);
    }

    /// 索引 `rel_path` 若省略 `.md`，须与 `current_rel_path` 的规范化键对齐，否则会误报「无 chunk」
    #[test]
    fn suggest_related_notes_matches_index_rel_path_without_md_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("cur.md"), "x [[linked]]\n").unwrap();
        fs::write(root.join("linked.md"), "y").unwrap();
        fs::write(root.join("near.md"), "n").unwrap();

        let conn = semantic_index::open_embedding_db(root).expect("open embedding db");
        let hi = vec![1.0_f32, 0.0, 0.0];
        let near = vec![0.9_f32, 0.1, 0.0];
        semantic_index::upsert_doc_chunk(&conn, "cur#0", "cur", 0, "t", &hi, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "near#0", "near", 0, "t", &near, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "linked#0", "linked", 0, "t", &near, "m").unwrap();

        let tconn = Connection::open_in_memory().unwrap();
        let rec = suggest_related_notes(root, "cur.md", &conn, &tconn, 5, None).expect("chunks align");
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].target_rel_path, "near.md");
    }

    /// 磁盘尚未写入 `[[linked]]` 时，须用缓冲区覆盖解析已链接目标，否则会把 linked 当候选
    #[test]
    fn suggest_related_notes_editor_override_excludes_unsaved_wikilinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("cur.md"), "plain on disk\n").unwrap();
        fs::write(root.join("linked.md"), "y").unwrap();
        fs::write(root.join("near.md"), "n").unwrap();
        fs::write(root.join("far.md"), "z").unwrap();

        let conn = semantic_index::open_embedding_db(root).expect("open embedding db");
        let hi = vec![1.0_f32, 0.0, 0.0];
        let near = vec![0.9_f32, 0.1, 0.0];
        let far = vec![0.0_f32, 1.0, 0.0];
        semantic_index::upsert_doc_chunk(&conn, "cur.md#0", "cur.md", 0, "t", &hi, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "near.md#0", "near.md", 0, "t", &near, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "linked.md#0", "linked.md", 0, "t", &near, "m").unwrap();
        semantic_index::upsert_doc_chunk(&conn, "far.md#0", "far.md", 0, "t", &far, "m").unwrap();

        let tconn = Connection::open_in_memory().unwrap();
        let rec = suggest_related_notes(
            root,
            "cur.md",
            &conn,
            &tconn,
            5,
            Some("x [[linked]]\n"),
        )
        .unwrap();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].target_rel_path, "near.md");
    }

    #[test]
    fn merge_llm_reason_items_respects_whitelist() {
        let mut c = vec![
            LinkRecommendation {
                target_rel_path: "notes/a.md".into(),
                score: 0.9,
                shared_topics: vec![],
                existing_link: false,
                reason: None,
            },
            LinkRecommendation {
                target_rel_path: "notes/b.md".into(),
                score: 0.8,
                shared_topics: vec![],
                existing_link: false,
                reason: None,
            },
        ];
        let json = r#"{"items":[{"relPath":"notes/a.md","reason":"Both discuss Rust modules."},{"relPath":"evil.md","reason":"ignored"}]}"#;
        super::merge_llm_reason_items(&mut c, json);
        assert_eq!(
            c[0].reason.as_deref(),
            Some("Both discuss Rust modules.")
        );
        assert!(c[1].reason.is_none());
    }

    #[test]
    fn extract_json_object_tolerates_surrounding_text() {
        let raw = "Here:\n```\n{\"items\":[]}\n```\n";
        let s = super::extract_json_object(raw).unwrap();
        let v: super::LinkReasonBatchJson = serde_json::from_str(&s).unwrap();
        assert!(v.items.is_empty());
    }

    #[tokio::test]
    async fn enrich_openai_branch_completes_ok_without_reason_until_completion_wired() {
        let mut ai = vault_config::AiConfig::default();
        ai.active_provider = ActiveProvider::Openai;
        let mut c = vec![LinkRecommendation {
            target_rel_path: "x.md".into(),
            score: 0.5,
            shared_topics: vec![],
            existing_link: false,
            reason: None,
        }];
        super::enrich_recommendations_with_reasons(&mut c, "excerpt", &ai)
            .await
            .unwrap();
        assert!(c[0].reason.is_none());
    }
}
