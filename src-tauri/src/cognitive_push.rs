//! 认知回顾桌面推送：基于 CognitiveReportForUi 生成紧凑摘要，驱动 OS 通知。
//! 仅在应用运行时触发（启动检查 + 30 分钟定期检查）。

use crate::cognitive_report::{self, CognitiveReportForUi};
use crate::vault_config::CognitiveConfig;
use chrono::{Datelike, Local, NaiveDate};
use serde::Serialize;
use std::path::Path;

/// 推送通知内容
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushSummary {
    pub title: String,
    pub body: String,
}

/// 判定是否需要发送周报
fn should_send_weekly(last_sent: Option<&NaiveDate>) -> bool {
    let now = Local::now().date_naive();
    match last_sent {
        None => true,
        Some(last) => (now - *last).num_days() >= 6,
    }
}

/// 判定是否需要发送月报
fn should_send_monthly(last_sent: Option<&NaiveDate>) -> bool {
    let now = Local::now().date_naive();
    match last_sent {
        None => true,
        Some(last) => last.month() != now.month() || last.year() != now.year(),
    }
}

/// 生成周报摘要；无活动返回 None（不推送）
fn build_weekly_summary(report: &CognitiveReportForUi) -> Option<PushSummary> {
    let activity = report.updated_this_month + report.new_this_month;
    if activity == 0 {
        return None;
    }

    let title = "本周认知回顾".to_string();

    // 从 timelines 中提取最活跃的 thought
    let top_thought = report.timelines.first().map(|t| {
        let excerpt: String = t.excerpt.chars().take(30).collect();
        let history_count = t.history.len();
        format!("「{excerpt}」经历 {history_count} 次变化")
    });

    let mut body_parts = vec![format!("本月活跃 {activity} 个想法")];

    // 成熟度分布
    let total = report.maturity.seedling + report.maturity.growing + report.maturity.mature;
    if total > 0 {
        body_parts.push(format!(
            "🌱{} 🌿{} 🌳{}",
            report.maturity.seedling, report.maturity.growing, report.maturity.mature
        ));
    }

    if let Some(thought) = top_thought {
        body_parts.push(thought);
    }

    Some(PushSummary {
        title,
        body: body_parts.join(" · "),
    })
}

/// 生成月报摘要；无晋升返回鼓励文案
fn build_monthly_summary(report: &CognitiveReportForUi) -> Option<PushSummary> {
    let title = "本月认知回顾".to_string();

    // 计算本月 vs 上月的成熟度变化
    let (promoted_to_growing, promoted_to_mature) =
        if let Some(ref prev) = report.prev_month_maturity {
            let to_growing = report.maturity.growing.saturating_sub(prev.growing);
            let to_mature = report.maturity.mature.saturating_sub(prev.mature);
            (to_growing, to_mature)
        } else {
            (0, 0)
        };

    let total_promotions = promoted_to_growing + promoted_to_mature;

    if total_promotions == 0 && report.new_this_month == 0 {
        // 无晋升也无新增 → 鼓励文案
        let total = report.maturity.seedling + report.maturity.growing + report.maturity.mature;
        if total == 0 {
            return None;
        }
        return Some(PushSummary {
            title,
            body: format!("本月复习了 {total} 个想法，继续坚持！"),
        });
    }

    let mut body_parts = vec![];

    if total_promotions > 0 {
        let mut promotion_desc = vec![];
        if promoted_to_growing > 0 {
            promotion_desc.push(format!("{promoted_to_growing} 个🌱→🌿"));
        }
        if promoted_to_mature > 0 {
            promotion_desc.push(format!("{promoted_to_mature} 个🌿→🌳"));
        }
        body_parts.push(format!("{} 理解加深", promotion_desc.join("，")));
    }

    if report.new_this_month > 0 {
        body_parts.push(format!("新增 {} 个想法", report.new_this_month));
    }

    // 成长最快的 thought
    if let Some(ref top) = report.timelines.first() {
        let excerpt: String = top.excerpt.chars().take(25).collect();
        body_parts.push(format!("成长最快：「{excerpt}」"));
    }

    Some(PushSummary {
        title,
        body: body_parts.join(" · "),
    })
}

/// 检查是否需要推送，返回待发送的通知列表
pub fn check_and_build_notifications(root: &Path, config: &CognitiveConfig) -> Vec<PushSummary> {
    if !config.cognitive_push_enabled {
        return vec![];
    }

    let report = match cognitive_report::generate_cognitive_report_blocking(root) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    let last_sent = config
        .cognitive_push_last_sent
        .as_ref()
        .and_then(|s| NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").ok());

    let mut notifications = vec![];

    // 周报判定
    if matches!(config.cognitive_push_frequency.as_str(), "weekly" | "both") {
        if should_send_weekly(last_sent.as_ref()) {
            if let Some(summary) = build_weekly_summary(&report) {
                notifications.push(summary);
            }
        }
    }

    // 月报判定
    if matches!(config.cognitive_push_frequency.as_str(), "monthly" | "both") {
        if should_send_monthly(last_sent.as_ref()) {
            if let Some(summary) = build_monthly_summary(&report) {
                notifications.push(summary);
            }
        }
    }

    notifications
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_send_weekly_no_last_sent() {
        assert!(should_send_weekly(None));
    }

    #[test]
    fn test_should_send_weekly_recent() {
        let now = Local::now().date_naive();
        assert!(!should_send_weekly(Some(&now)));
    }

    #[test]
    fn test_should_send_weekly_old() {
        let now = Local::now().date_naive();
        let old = now - chrono::Duration::days(7);
        assert!(should_send_weekly(Some(&old)));
    }

    #[test]
    fn test_should_send_monthly_same_month() {
        let now = Local::now().date_naive();
        assert!(!should_send_monthly(Some(&now)));
    }

    #[test]
    fn test_should_send_monthly_different_month() {
        let now = Local::now().date_naive();
        let old = now - chrono::Duration::days(35);
        assert!(should_send_monthly(Some(&old)));
    }

    #[test]
    fn test_build_weekly_summary_no_activity() {
        let report = CognitiveReportForUi {
            scanned_files: 10,
            total_thoughts: 5,
            new_this_month: 0,
            updated_this_month: 0,
            maturity: Default::default(),
            prev_month_maturity: None,
            total_ai_references: 0,
            timelines: vec![],
            monthly_snapshots: vec![],
        };
        assert!(build_weekly_summary(&report).is_none());
    }

    #[test]
    fn test_build_weekly_summary_with_activity() {
        let report = CognitiveReportForUi {
            scanned_files: 10,
            total_thoughts: 5,
            new_this_month: 2,
            updated_this_month: 3,
            maturity: Default::default(),
            prev_month_maturity: None,
            total_ai_references: 0,
            timelines: vec![],
            monthly_snapshots: vec![],
        };
        let summary = build_weekly_summary(&report).unwrap();
        assert!(summary.body.contains("5"));
    }

    #[test]
    fn test_build_monthly_summary_no_promotions() {
        let report = CognitiveReportForUi {
            scanned_files: 10,
            total_thoughts: 5,
            new_this_month: 0,
            updated_this_month: 0,
            maturity: crate::cognitive_report::MaturityCounts {
                seedling: 3,
                growing: 1,
                mature: 1,
            },
            prev_month_maturity: Some(crate::cognitive_report::MaturityCounts {
                seedling: 3,
                growing: 1,
                mature: 1,
            }),
            total_ai_references: 0,
            timelines: vec![],
            monthly_snapshots: vec![],
        };
        let summary = build_monthly_summary(&report).unwrap();
        assert!(summary.body.contains("坚持"));
    }

    #[test]
    fn test_build_monthly_summary_with_promotions() {
        let report = CognitiveReportForUi {
            scanned_files: 10,
            total_thoughts: 5,
            new_this_month: 1,
            updated_this_month: 2,
            maturity: crate::cognitive_report::MaturityCounts {
                seedling: 2,
                growing: 2,
                mature: 1,
            },
            prev_month_maturity: Some(crate::cognitive_report::MaturityCounts {
                seedling: 3,
                growing: 1,
                mature: 1,
            }),
            total_ai_references: 0,
            timelines: vec![],
            monthly_snapshots: vec![],
        };
        let summary = build_monthly_summary(&report).unwrap();
        assert!(summary.body.contains("🌿"));
        assert!(summary.body.contains("1"));
    }
}
