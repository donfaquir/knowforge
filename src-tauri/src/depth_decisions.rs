//! 深度决策日志：记录 auto-depth 的自动解析结果及用户覆盖。
//! 持久化到 `.knowforge/depth-decisions.jsonl`，每行一条 JSON。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Seek, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::vault_config::DepthMode;

const LOG_FILE: &str = ".knowforge/depth-decisions.jsonl";
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024; // 5 MB
const ROTATE_KEEP_DAYS: i64 = 30;

/// 一条决策日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthDecisionEntry {
    pub timestamp: DateTime<Utc>,
    pub auto_resolved: DepthMode,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_override: Option<DepthMode>,
}

fn log_path(root: &Path) -> PathBuf {
    root.join(LOG_FILE)
}

/// 追加一条决策到日志文件
pub fn append_decision(root: &Path, entry: &DepthDecisionEntry) -> Result<(), String> {
    let path = log_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let line = serde_json::to_string(entry).map_err(|e| format!("json: {e}"))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("write: {e}"))?;
    // 用已打开句柄取末尾位置，避免对路径再 stat；不维护跨调用计数器，以免轮转/外部改写导致漂移
    let new_len = file.stream_position().map_err(|e| format!("tell: {e}"))?;
    drop(file);
    // 仅在超阈值时轮转，避免每次追加都全文件扫描
    if new_len >= MAX_FILE_SIZE {
        let _ = rotate_log(root);
    }
    Ok(())
}

/// 读取最近 N 条决策
pub fn read_recent_decisions(root: &Path, max: usize) -> Vec<DepthDecisionEntry> {
    let path = log_path(root);
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut entries: Vec<DepthDecisionEntry> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<DepthDecisionEntry>(line) {
            entries.push(entry);
        }
    }
    // 取最后 max 条
    if entries.len() > max {
        entries.drain(..entries.len() - max);
    }
    entries
}

/// 日志轮转：文件 > 5MB 时，删除 30 天前的条目
pub fn rotate_log(root: &Path) -> Result<(), String> {
    let path = log_path(root);
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return Ok(()), // 文件不存在无需轮转
    };
    if meta.len() < MAX_FILE_SIZE {
        return Ok(());
    }
    let cutoff = Utc::now() - chrono::Duration::days(ROTATE_KEEP_DAYS);
    let file = fs::File::open(&path).map_err(|e| format!("open: {e}"))?;
    let reader = BufReader::new(file);
    let mut kept: Vec<String> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<DepthDecisionEntry>(trimmed) {
            if entry.timestamp >= cutoff {
                kept.push(line);
            }
        } else {
            // 解析失败的行保留
            kept.push(line);
        }
    }
    // 历史固定名临时文件：崩溃残留，轮转前尽量删掉以免干扰
    let legacy_tmp = path.with_extension("jsonl.tmp");
    let _ = fs::remove_file(&legacy_tmp);

    let parent = path
        .parent()
        .ok_or_else(|| "log path has no parent directory".to_string())?;
    let tmp = parent.join(format!(
        "depth-decisions.rotate.{}.tmp",
        Uuid::new_v4()
    ));

    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("create tmp: {e}"))?;

    for line in &kept {
        if let Err(e) = writeln!(out, "{line}") {
            let _ = fs::remove_file(&tmp);
            return Err(format!("write tmp: {e}"));
        }
    }
    if let Err(e) = out.flush() {
        let _ = fs::remove_file(&tmp);
        return Err(format!("flush tmp: {e}"));
    }
    drop(out);

    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("rename: {e}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_entry(mode: DepthMode, reason: &str) -> DepthDecisionEntry {
        DepthDecisionEntry {
            timestamp: Utc::now(),
            auto_resolved: mode,
            reason: reason.to_string(),
            user_override: None,
        }
    }

    #[test]
    fn append_and_read_back() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        append_decision(root, &make_entry(DepthMode::Medium, "short query")).unwrap();
        append_decision(root, &make_entry(DepthMode::Deep, "long query")).unwrap();
        let entries = read_recent_decisions(root, 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].reason, "long query");
    }

    #[test]
    fn read_recent_limits() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        for i in 0..10 {
            append_decision(root, &make_entry(DepthMode::Shallow, &format!("q{i}"))).unwrap();
        }
        let entries = read_recent_decisions(root, 3);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].reason, "q7");
    }

    #[test]
    fn rotate_removes_old() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // 写一条 60 天前的
        let mut old = make_entry(DepthMode::Auto, "old");
        old.timestamp = Utc::now() - chrono::Duration::days(60);
        append_decision(root, &old).unwrap();
        // 写一条新的
        append_decision(root, &make_entry(DepthMode::Medium, "new")).unwrap();

        // 不超过 5MB，rotate 不触发
        rotate_log(root).unwrap();
        let all = read_recent_decisions(root, 100);
        assert_eq!(all.len(), 2);

        // 人为增大文件使之超 5MB
        let path = root.join(LOG_FILE);
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let padding = "x".repeat(1024);
        for _ in 0..5200 {
            writeln!(f, "{padding}").unwrap();
        }
        rotate_log(root).unwrap();
        let after = read_recent_decisions(root, 100);
        // old 条目应被清除，只剩 new
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].reason, "new");
    }

    #[test]
    fn empty_file_read() {
        let tmp = TempDir::new().unwrap();
        let entries = read_recent_decisions(tmp.path(), 10);
        assert!(entries.is_empty());
    }

}
