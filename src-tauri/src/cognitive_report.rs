//! 认知成长报告：全 Vault 扫描 kf-thoughts、成熟度分布与快照（迭代 4）。

use crate::note_privacy;
use crate::thought_parser::{self, ThoughtMaturity};
use crate::vault_context_search;
use crate::sanitize_io_error;
use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 600;
const READ_CAP: usize = 512 * 1024;

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaturityCounts {
    pub seedling: usize,
    pub growing: usize,
    pub mature: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryOut {
    pub date: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineThoughtOut {
    pub rel_path: String,
    pub thought_id: String,
    pub excerpt: String,
    pub history: Vec<HistoryEntryOut>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MonthlySnapshot {
    pub year_month: String,
    pub seedling: usize,
    pub growing: usize,
    pub mature: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CognitiveReportForUi {
    pub scanned_files: usize,
    pub total_thoughts: usize,
    pub new_this_month: usize,
    pub updated_this_month: usize,
    pub maturity: MaturityCounts,
    pub prev_month_maturity: Option<MaturityCounts>,
    pub total_ai_references: usize,
    pub timelines: Vec<TimelineThoughtOut>,
    pub monthly_snapshots: Vec<MonthlySnapshot>,
}

#[derive(Deserialize, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct StoredMonthRow {
    year_month: String,
    seedling: usize,
    growing: usize,
    mature: usize,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SnapshotFile {
    months: Vec<StoredMonthRow>,
}

fn snapshot_path(root: &Path) -> PathBuf {
    root.join(".knowforge/report-snapshots/monthly.json")
}

fn this_year_month() -> String {
    let d = Utc::now().date_naive();
    format!("{}-{:02}", d.year(), d.month())
}

fn prev_year_month_label(current: &str) -> Option<String> {
    let mut parts = current.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    if m <= 1 {
        Some(format!("{}-12", y - 1))
    } else {
        Some(format!("{}-{:02}", y, m - 1))
    }
}

fn month_window(ym: &str) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::parse_from_str(&format!("{ym}-01"), "%Y-%m-%d").ok()?;
    let (ny, nm) = if start.month() == 12 {
        (start.year() + 1, 1u32)
    } else {
        (start.year(), start.month() + 1)
    };
    let end_excl = NaiveDate::from_ymd_opt(ny, nm, 1)?;
    Some((start, end_excl))
}

fn parse_doc_date(s: &str) -> Option<NaiveDate> {
    let t = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.date_naive());
    }
    if t.len() >= 10 {
        return NaiveDate::parse_from_str(&t[..10], "%Y-%m-%d").ok();
    }
    None
}

/// 汇总全 Vault；写回 `.knowforge/report-snapshots/monthly.json` 中当月行。
pub fn generate_cognitive_report_blocking(root: &Path) -> Result<CognitiveReportForUi, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    vault_context_search::walk_markdown_files(root, root, &mut paths, MAX_FILES)?;
    let scanned_files = paths.len();

    let ym = this_year_month();
    let (month_start, month_end_excl) = month_window(&ym).ok_or_else(|| "invalid month".to_string())?;

    let mut maturity = MaturityCounts::default();
    let mut new_this_month = 0usize;
    let mut updated_this_month = 0usize;
    let mut total_thoughts = 0usize;
    let mut total_refs = 0usize;
    let mut timeline_candidates: Vec<(usize, TimelineThoughtOut)> = Vec::new();

    for abs in &paths {
        let Some(rel) = vault_context_search::rel_path_from_root(root, abs) else {
            continue;
        };
        let bytes = fs::read(abs).map_err(|e| sanitize_io_error(e, "reading markdown"))?;
        if bytes.len() > READ_CAP {
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
        for (i, meta) in parsed.meta.iter().enumerate() {
            if meta.id.is_empty() {
                continue;
            }
            total_thoughts += 1;
            total_refs += meta.references.len();
            match meta.maturity {
                ThoughtMaturity::Seedling => maturity.seedling += 1,
                ThoughtMaturity::Growing => maturity.growing += 1,
                ThoughtMaturity::Mature => maturity.mature += 1,
            }
            if let Some(cdt) = parse_doc_date(&meta.created) {
                if cdt >= month_start && cdt < month_end_excl {
                    new_this_month += 1;
                }
            }
            if let Some(udt) = parse_doc_date(&meta.updated) {
                if udt >= month_start && udt < month_end_excl {
                    updated_this_month += 1;
                }
            }
            let excerpt = parsed
                .blocks
                .get(i)
                .map(|b| b.excerpt.as_str())
                .unwrap_or("")
                .chars()
                .take(120)
                .collect::<String>();
            let hist_count = meta.history.len();
            let history: Vec<HistoryEntryOut> = meta
                .history
                .iter()
                .take(16)
                .map(|e| HistoryEntryOut {
                    date: e.date.clone(),
                    entry_type: e.entry_type.clone(),
                    source: e.source.clone(),
                    diff_summary: e.diff_summary.clone(),
                })
                .collect();
            timeline_candidates.push((
                hist_count,
                TimelineThoughtOut {
                    rel_path: rel.clone(),
                    thought_id: meta.id.clone(),
                    excerpt,
                    history,
                },
            ));
        }
    }

    timeline_candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let timelines: Vec<_> = timeline_candidates.into_iter().take(3).map(|(_, t)| t).collect();

    let snap_path = snapshot_path(root);
    let mut snap = if snap_path.exists() {
        fs::read_to_string(&snap_path)
            .ok()
            .and_then(|s| serde_json::from_str::<SnapshotFile>(&s).ok())
            .unwrap_or_default()
    } else {
        SnapshotFile::default()
    };

    let prev_month_maturity = prev_year_month_label(&ym).and_then(|pk| {
        snap.months.iter().find(|m| m.year_month == pk).map(|m| MaturityCounts {
            seedling: m.seedling,
            growing: m.growing,
            mature: m.mature,
        })
    });

    if let Some(row) = snap.months.iter_mut().find(|m| m.year_month == ym) {
        row.seedling = maturity.seedling;
        row.growing = maturity.growing;
        row.mature = maturity.mature;
    } else {
        snap.months.push(StoredMonthRow {
            year_month: ym.clone(),
            seedling: maturity.seedling,
            growing: maturity.growing,
            mature: maturity.mature,
        });
    }
    snap.months.sort_by(|a, b| a.year_month.cmp(&b.year_month));
    if snap.months.len() > 24 {
        let drain = snap.months.len() - 24;
        snap.months.drain(0..drain);
    }
    if let Some(parent) = snap_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&snap) {
        let _ = fs::write(&snap_path, bytes);
    }

    let monthly_snapshots: Vec<MonthlySnapshot> = snap
        .months
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|m| MonthlySnapshot {
            year_month: m.year_month.clone(),
            seedling: m.seedling,
            growing: m.growing,
            mature: m.mature,
        })
        .collect();

    Ok(CognitiveReportForUi {
        scanned_files,
        total_thoughts,
        new_this_month,
        updated_this_month,
        maturity,
        prev_month_maturity,
        total_ai_references: total_refs,
        timelines,
        monthly_snapshots,
    })
}

#[tauri::command]
pub async fn generate_cognitive_report(
    state: tauri::State<'_, crate::WorkspaceState>,
) -> Result<CognitiveReportForUi, String> {
    let canonical_root = crate::lock_workspace_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || generate_cognitive_report_blocking(&canonical_root))
        .await
        .map_err(|e| e.to_string())?
}
