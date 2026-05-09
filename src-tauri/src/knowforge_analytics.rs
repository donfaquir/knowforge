//! 本地埋点：向 `.knowforge/analytics.jsonl` 追加一行 JSON（供被动高亮等模块复用）

use crate::lock_workspace_root;
use crate::vault_config;
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use tauri::State;

#[derive(Serialize)]
struct AnalyticsLine {
    event: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendKnowforgeAnalyticsArgs {
    pub event: String,
    #[serde(default)]
    pub payload: Option<Value>,
}

fn append_line_blocking(root: &std::path::Path, event: String, payload: Option<Value>) -> Result<(), String> {
    let path = vault_config::analytics_jsonl_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create .knowforge: {e}"))?;
    }
    let line = AnalyticsLine {
        event,
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        payload,
    };
    let json = serde_json::to_string(&line).map_err(|e| format!("failed to serialize analytics: {e}"))?;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("failed to open analytics file: {e}"))?;
    writeln!(f, "{json}").map_err(|e| format!("failed to write analytics: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn append_knowforge_analytics(
    workspace: State<'_, crate::WorkspaceState>,
    args: AppendKnowforgeAnalyticsArgs,
) -> Result<(), String> {
    let root = lock_workspace_root(&workspace)?;
    let event = args.event;
    let payload = args.payload;
    tauri::async_runtime::spawn_blocking(move || append_line_blocking(&root, event, payload))
        .await
        .map_err(|e| e.to_string())?
}
