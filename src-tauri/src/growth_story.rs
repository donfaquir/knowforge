//! Thought 成长故事：从 Thought 的 history 时间线构建可导出的成长旅程。

use crate::note_privacy;
use crate::thought_parser::{self, KfThoughtMeta, ThoughtMaturity};
use crate::vault_context_search;
use chrono::{Datelike, NaiveDate, Utc};
use serde::Serialize;
use std::path::Path;

/// 成长故事摘要
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthStory {
    pub thought_id: String,
    pub thought_title: String,
    pub content_preview: String,
    pub source_file: String,
    pub created_at: String,
    pub current_maturity: String,
    pub journey: Vec<JourneyMilestone>,
    pub total_challenges: usize,
    pub total_days: usize,
    pub pass_rate: f64,
}

/// 成长旅程中的单个里程碑
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JourneyMilestone {
    pub date: String,
    pub event_type: String,
    pub description: String,
}

/// 从 Markdown 文件中查找指定 thought 的元数据
fn find_thought_meta(root: &Path, thought_id: &str) -> Result<Option<(KfThoughtMeta, String, String)>, String> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(root, root, &mut paths, 600)?;

    for abs in &paths {
        let Some(rel) = vault_context_search::rel_path_from_root(root, abs) else {
            continue;
        };
        let bytes = std::fs::read(abs).map_err(|e| format!("reading {rel}: {e}"))?;
        if bytes.len() > 512 * 1024 {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if note_privacy::markdown_treat_as_kf_private(&text) {
            continue;
        }
        if !text.contains("kf-thoughts") {
            continue;
        }
        let parsed = thought_parser::parse_note_thoughts_for_workspace(root, &rel, &text);
        for meta in &parsed.meta {
            if meta.id == thought_id {
                return Ok(Some((meta.clone(), rel, text)));
            }
        }
    }
    Ok(None)
}

/// 构建成长故事
pub fn build_growth_story(root: &Path, thought_id: &str) -> Result<GrowthStory, String> {
    let (meta, rel_path, _markdown) = find_thought_meta(root, thought_id)?
        .ok_or_else(|| format!("thought {thought_id} not found"))?;

    let title = extract_title_from_body(&meta, root, &rel_path);
    let content_preview = extract_content_preview(&meta, root, &rel_path);

    let maturity_str = match meta.maturity {
        ThoughtMaturity::Seedling => "seedling",
        ThoughtMaturity::Growing => "growing",
        ThoughtMaturity::Mature => "mature",
    };

    let mut journey = Vec::new();

    // created 事件
    journey.push(JourneyMilestone {
        date: format_date_short(&meta.created),
        event_type: "created".to_string(),
        description: "开始追踪这个想法".to_string(),
    });

    // history 事件
    let mut challenge_count = 0usize;
    let mut pass_count = 0usize;

    for entry in &meta.history {
        let desc = match entry.entry_type.as_str() {
            "created" => continue, // 已添加
            "substantial-change" => {
                entry.diff_summary.clone().unwrap_or_else(|| "内容更新".to_string())
            }
            "challenge-review-pass" => {
                challenge_count += 1;
                pass_count += 1;
                format!("第 {} 次挑战通过", challenge_count)
            }
            "challenge-review-attempt" => {
                challenge_count += 1;
                format!("第 {} 次挑战尝试", challenge_count)
            }
            _ => entry.diff_summary.clone().unwrap_or_else(|| "事件".to_string()),
        };
        journey.push(JourneyMilestone {
            date: format_date_short(&entry.date),
            event_type: entry.entry_type.clone(),
            description: desc,
        });
    }

    // 成熟度晋升事件（从 history 中的 challenge-review-pass 推断）
    if meta.maturity != ThoughtMaturity::Seedling {
        // 检查是否有晋升事件（通过 pass_count 推断）
        if meta.challenge_pass_count >= 1 && meta.maturity as u8 >= ThoughtMaturity::Growing as u8 {
            // 找到第一个 challenge-review-pass 的日期作为晋升到 Growing 的时间
            if let Some(first_pass) = meta.history.iter().find(|h| h.entry_type == "challenge-review-pass") {
                journey.push(JourneyMilestone {
                    date: format_date_short(&first_pass.date),
                    event_type: "promoted".to_string(),
                    description: "🌱→🌿 理解加深".to_string(),
                });
            }
        }
        if meta.maturity == ThoughtMaturity::Mature {
            // 找到最后一个 challenge-review-pass 的日期作为晋升到 Mature 的时间
            if let Some(last_pass) = meta.history.iter().rev().find(|h| h.entry_type == "challenge-review-pass") {
                journey.push(JourneyMilestone {
                    date: format_date_short(&last_pass.date),
                    event_type: "promoted".to_string(),
                    description: "🌿🌳 融会贯通".to_string(),
                });
            }
        }
    }

    // 按日期排序
    journey.sort_by(|a, b| a.date.cmp(&b.date));

    // 计算总天数
    let total_days = compute_total_days(&meta.created);

    // 计算通过率
    let pass_rate = if challenge_count > 0 {
        pass_count as f64 / challenge_count as f64
    } else {
        0.0
    };

    Ok(GrowthStory {
        thought_id: meta.id,
        thought_title: title,
        content_preview,
        source_file: rel_path,
        created_at: meta.created,
        current_maturity: maturity_str.to_string(),
        journey,
        total_challenges: challenge_count,
        total_days,
        pass_rate,
    })
}

/// 从 thought body 中提取标题（第一行或前 50 字符）
fn extract_title_from_body(meta: &KfThoughtMeta, root: &Path, rel_path: &str) -> String {
    let body = read_thought_body(root, rel_path, &meta.id);
    if body.is_empty() {
        return meta.id.clone();
    }
    let first_line = body.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        meta.id.clone()
    } else if first_line.len() > 50 {
        format!("{}…", &first_line[..50])
    } else {
        first_line.to_string()
    }
}

/// 提取内容预览（前 100 字符）
fn extract_content_preview(meta: &KfThoughtMeta, root: &Path, rel_path: &str) -> String {
    let body = read_thought_body(root, rel_path, &meta.id);
    if body.is_empty() {
        return String::new();
    }
    let preview: String = body.chars().take(100).collect();
    if body.len() > 100 {
        format!("{preview}…")
    } else {
        preview
    }
}

/// 从 SQLite 读取 thought body
fn read_thought_body(root: &Path, _rel_path: &str, thought_id: &str) -> String {
    let conn = match crate::vault_thoughts_db::open_thoughts_db(root) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    crate::vault_thoughts_db::get_body(&conn, thought_id)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// 格式化日期为短格式 "M/D"
fn format_date_short(rfc3339: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(rfc3339) {
        let local = dt.with_timezone(&chrono::Local);
        format!("{}/{}", local.month(), local.day())
    } else if rfc3339.len() >= 10 {
        // 尝试 YYYY-MM-DD 格式
        NaiveDate::parse_from_str(&rfc3339[..10], "%Y-%m-%d")
            .map(|d| format!("{}/{}", d.month(), d.day()))
            .unwrap_or_else(|_| rfc3339[..10].to_string())
    } else {
        rfc3339.to_string()
    }
}

/// 计算从创建到现在的天数
fn compute_total_days(created_at: &str) -> usize {
    let created = chrono::DateTime::parse_from_rfc3339(created_at)
        .ok()
        .map(|dt| dt.date_naive());
    let now = Utc::now().date_naive();
    match created {
        Some(c) => (now - c).num_days().max(0) as usize,
        None => 0,
    }
}

/// 生成 HTML 卡片格式的成长故事（用于图片导出）
pub fn to_html_card(story: &GrowthStory) -> String {
    let maturity_emoji = match story.current_maturity.as_str() {
        "seedling" => "🌱",
        "growing" => "🌿",
        "mature" => "🌳",
        _ => "🌱",
    };
    let maturity_label = match story.current_maturity.as_str() {
        "seedling" => "萌芽",
        "growing" => "成长",
        "mature" => "融会贯通",
        _ => "萌芽",
    };

    let journey_html: String = story
        .journey
        .iter()
        .map(|m| {
            let icon = match m.event_type.as_str() {
                "created" => "💡",
                "substantial-change" => "✏️",
                "challenge-review-pass" => "✅",
                "challenge-review-attempt" => "🔄",
                "promoted" => "⬆️",
                _ => "📌",
            };
            format!(
                r#"<div class="milestone"><span class="milestone-icon">{icon}</span><span class="milestone-date">{date}</span><span class="milestone-desc">{desc}</span></div>"#,
                icon = icon,
                date = m.date,
                desc = m.description
            )
        })
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ 
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    padding: 40px;
  }}
  .card {{
    background: white;
    border-radius: 20px;
    padding: 40px;
    max-width: 600px;
    margin: 0 auto;
    box-shadow: 0 20px 60px rgba(0,0,0,0.3);
  }}
  .header {{
    text-align: center;
    margin-bottom: 30px;
  }}
  .maturity {{
    font-size: 48px;
    margin-bottom: 10px;
  }}
  .title {{
    font-size: 24px;
    font-weight: 600;
    color: #333;
    margin-bottom: 5px;
  }}
  .label {{
    font-size: 14px;
    color: #666;
  }}
  .timeline {{
    margin: 30px 0;
    padding: 20px;
    background: #f8f9fa;
    border-radius: 12px;
  }}
  .milestone {{
    display: flex;
    align-items: center;
    padding: 10px 0;
    border-bottom: 1px solid #eee;
  }}
  .milestone:last-child {{
    border-bottom: none;
  }}
  .milestone-icon {{
    font-size: 20px;
    margin-right: 12px;
  }}
  .milestone-date {{
    font-size: 14px;
    color: #666;
    width: 60px;
    flex-shrink: 0;
  }}
  .milestone-desc {{
    font-size: 14px;
    color: #333;
  }}
  .stats {{
    text-align: center;
    font-size: 16px;
    color: #666;
    margin: 20px 0;
  }}
  .footer {{
    text-align: center;
    font-size: 12px;
    color: #999;
    margin-top: 20px;
    padding-top: 20px;
    border-top: 1px solid #eee;
  }}
</style>
</head>
<body>
  <div class="card">
    <div class="header">
      <div class="maturity">{emoji}</div>
      <div class="title">{title}</div>
      <div class="label">{label}</div>
    </div>
    <div class="timeline">
      {journey}
    </div>
    <div class="stats">
      经历 {challenges} 次挑战 · 通过率 {pass_rate:.0}% · 历时 {days} 天
    </div>
    <div class="footer">
      ─── KnowForge · 理解你写下的每个想法 ───
    </div>
  </div>
</body>
</html>"#,
        emoji = maturity_emoji,
        title = story.thought_title,
        label = maturity_label,
        journey = journey_html,
        challenges = story.total_challenges,
        pass_rate = story.pass_rate * 100.0,
        days = story.total_days
    )
}

/// 生成 Markdown 格式的成长故事
pub fn to_markdown(story: &GrowthStory) -> String {
    let maturity_emoji = match story.current_maturity.as_str() {
        "seedling" => "🌱",
        "growing" => "🌿",
        "mature" => "🌳",
        _ => "🌱",
    };
    let maturity_label = match story.current_maturity.as_str() {
        "seedling" => "萌芽",
        "growing" => "成长",
        "mature" => "融会贯通",
        _ => "萌芽",
    };

    let mut md = format!("## {} {} — 成长故事（{}）\n\n", maturity_emoji, story.thought_title, maturity_label);

    if !story.content_preview.is_empty() {
        md.push_str(&format!("> {}\n\n", story.content_preview));
    }

    md.push_str(&format!(
        "从 {} 开始追踪，历时 {} 天：\n\n",
        format_date_short(&story.created_at),
        story.total_days
    ));

    for m in &story.journey {
        md.push_str(&format!("- {} {}\n", m.date, m.description));
    }

    md.push_str(&format!(
        "\n共经历 {} 次挑战 · 通过率 {:.0}% · 历时 {} 天\n",
        story.total_challenges,
        story.pass_rate * 100.0,
        story.total_days
    ));

    md.push_str("\n---\n*Generated by KnowForge*\n");

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_date_short() {
        assert_eq!(format_date_short("2026-07-15T10:30:00+08:00"), "7/15");
    }

    #[test]
    fn test_format_date_short_plain() {
        assert_eq!(format_date_short("2026-01-05"), "1/5");
    }

    #[test]
    fn test_compute_total_days() {
        let now = Utc::now();
        let created = (now - chrono::Duration::days(10)).to_rfc3339();
        let days = compute_total_days(&created);
        assert!((9..=11).contains(&days));
    }

    #[test]
    fn test_to_markdown() {
        let story = GrowthStory {
            thought_id: "test".to_string(),
            thought_title: "Test Thought".to_string(),
            content_preview: "Preview".to_string(),
            source_file: "test.md".to_string(),
            created_at: "2026-07-01T00:00:00+00:00".to_string(),
            current_maturity: "growing".to_string(),
            journey: vec![
                JourneyMilestone {
                    date: "7/1".to_string(),
                    event_type: "created".to_string(),
                    description: "开始追踪".to_string(),
                },
                JourneyMilestone {
                    date: "7/8".to_string(),
                    event_type: "challenge-review-pass".to_string(),
                    description: "第 1 次挑战通过".to_string(),
                },
            ],
            total_challenges: 3,
            total_days: 15,
            pass_rate: 0.6667,
        };
        let md = to_markdown(&story);
        assert!(md.contains("成长故事"));
        assert!(md.contains("Test Thought"));
        assert!(md.contains("7/1"));
        assert!(md.contains("7/8"));
        assert!(md.contains("KnowForge"));
    }
}
