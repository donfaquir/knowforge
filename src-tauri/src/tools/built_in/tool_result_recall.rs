use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

pub struct ToolResultRecallTool {
    manifest: ToolManifest,
}

impl ToolResultRecallTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "tool.recall".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Retrieve the full raw content of a previously summarized tool \
                              result. Use when the summary (marked with [summarized from ... | \
                              ref:XXX]) is insufficient and you need the original details."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["ref"],
                    "properties": {
                        "ref": {
                            "type": "string",
                            "description": "The ref ID from the [summarized from ... | ref:XXX] marker"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string" },
                        "tool_name": { "type": "string" },
                        "len": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["utility".to_string(), "recall".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for ToolResultRecallTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Utility
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let ref_id = match input.get("ref").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return ToolResult::Err { error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: "missing required field: ref".to_string(),
                    retryable: false,
                    cause: None,
                } };
            }
        };

        let session_dir = build_session_dir(ctx);

        let file_path = match find_by_prefix(&session_dir, ref_id).await {
            Some(p) => p,
            None => {
                return ToolResult::Err { error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!(
                        "no stored tool result matching ref '{}' in {}",
                        ref_id,
                        session_dir.display()
                    ),
                    retryable: false,
                    cause: None,
                } };
            }
        };

        let raw = match tokio::fs::read_to_string(&file_path).await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err { error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("failed to read {}: {}", file_path.display(), e),
                    retryable: false,
                    cause: None,
                } };
            }
        };

        let record: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::Err { error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("corrupted stored result: {}", e),
                    retryable: false,
                    cause: None,
                } };
            }
        };

        let content = record
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tool_name = record
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let len = record
            .get("len")
            .and_then(|v| v.as_u64())
            .unwrap_or(content.len() as u64);

        ToolResult::Ok {
            data: serde_json::json!({
                "content": content,
                "tool_name": tool_name,
                "len": len,
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms: 0,
                ..Default::default()
            },
        }
    }
}

fn build_session_dir(ctx: &ToolContext) -> PathBuf {
    let sid = if ctx.session_id.is_empty() {
        &ctx.conversation_id
    } else {
        &ctx.session_id
    };
    ctx.workspace_root
        .join(".knowforge")
        .join("tool-results")
        .join(sid)
}

async fn find_by_prefix(session_dir: &PathBuf, ref_id: &str) -> Option<PathBuf> {
    let mut entries = match tokio::fs::read_dir(session_dir).await {
        Ok(e) => e,
        Err(_) => return None,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(ref_id) && name_str.ends_with(".json") {
            return Some(entry.path());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_by_prefix_matches() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = tempfile::tempdir().unwrap();
            let file = tmp.path().join("abcdef12-3456-7890.json");
            tokio::fs::write(
                &file,
                r#"{"call_id":"abcdef12-3456-7890","tool_name":"web.search","content":"hello","len":5}"#,
            )
            .await
            .unwrap();

            let found = find_by_prefix(&tmp.path().to_path_buf(), "abcdef12").await;
            assert!(found.is_some());
            assert_eq!(found.unwrap(), file);

            let not_found = find_by_prefix(&tmp.path().to_path_buf(), "zzz").await;
            assert!(not_found.is_none());
        });
    }
}
