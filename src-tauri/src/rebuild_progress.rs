//! 语义索引「重建」断点进度：落盘于 vault `.knowforge/semantic/rebuild_progress.json`，供 UI 进度条与续建。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

pub const CHECKPOINT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RebuildProgress {
    pub version: u32,
    pub rebuild_id: String,
    /// scanning | documents | thoughts | completed | failed
    pub phase: String,
    pub started_at: String,
    pub updated_at: String,
    pub docs_total: usize,
    pub docs_completed: usize,
    pub thoughts_total: usize,
    /// 想法阶段下一批起始行下标（与 `INDEX_BATCH` 对齐递增）
    pub thoughts_next_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl RebuildProgress {
    pub fn new_session(rebuild_id: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            version: CHECKPOINT_VERSION,
            rebuild_id,
            phase: "scanning".to_string(),
            started_at: now.clone(),
            updated_at: now,
            docs_total: 0,
            docs_completed: 0,
            thoughts_total: 0,
            thoughts_next_index: 0,
            last_message: None,
            last_error: None,
        }
    }

    pub fn mark_completed(&mut self) {
        self.phase = "completed".to_string();
        self.last_error = None;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn mark_failed(&mut self, err: &str) {
        self.phase = "failed".to_string();
        self.last_error = Some(err.to_string());
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /// 0–100，供进度条（前端可复用相同公式，或将来 IPC 直接返回）
    #[allow(dead_code)]
    pub fn display_percent(&self) -> u8 {
        match self.phase.as_str() {
            "completed" => 100,
            "scanning" => 0,
            "documents" => {
                let d = self.docs_total.max(1);
                let p = (self.docs_completed.min(d) as f64) / (d as f64);
                (p * 92.0).round().clamp(0.0, 92.0) as u8
            }
            "thoughts" => {
                let t = self.thoughts_total.max(1);
                let th = (self.thoughts_next_index.min(t) as f64) / (t as f64);
                (92.0 + 8.0 * th).round().clamp(0.0, 100.0) as u8
            }
            "failed" => {
                // 失败时仍反映已走过比例
                if self.thoughts_total > 0 && self.docs_completed >= self.docs_total {
                    let t = self.thoughts_total.max(1);
                    let th = (self.thoughts_next_index.min(t) as f64) / (t as f64);
                    (92.0 + 8.0 * th).round().clamp(0.0, 99.0) as u8
                } else {
                    let d = self.docs_total.max(1);
                    let p = (self.docs_completed.min(d) as f64) / (d as f64);
                    (p * 92.0).round().clamp(0.0, 99.0) as u8
                }
            }
            _ => 0,
        }
    }
}

pub fn progress_file_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".knowforge/semantic/rebuild_progress.json")
}

pub fn read_rebuild_progress(vault_root: &Path) -> Option<RebuildProgress> {
    let p = progress_file_path(vault_root);
    let s = fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn write_rebuild_progress(vault_root: &Path, rp: &RebuildProgress) -> Result<(), String> {
    let p = progress_file_path(vault_root);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create rebuild progress dir: {e}"))?;
    }
    let tmp = p.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(rp).map_err(|e| format!("serialize rebuild progress: {e}"))?;
    fs::write(&tmp, data).map_err(|e| format!("write rebuild progress tmp: {e}"))?;
    fs::rename(&tmp, &p).map_err(|e| format!("rename rebuild progress: {e}"))?;
    Ok(())
}

pub fn emit_checkpoint(app: &AppHandle, rp: &RebuildProgress) {
    let v = serde_json::to_value(rp).unwrap_or(serde_json::Value::Null);
    let _ = app.emit("semantic:rebuild-checkpoint", v);
}
