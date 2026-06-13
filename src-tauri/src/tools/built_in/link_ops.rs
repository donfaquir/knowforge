use async_trait::async_trait;
use serde_json::Value;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

// ─── LinkSuggestRelatedTool ────────────────────────────────────────────────────

pub struct LinkSuggestRelatedTool {
    manifest: ToolManifest,
}

impl LinkSuggestRelatedTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "link.suggest_related".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "基于向量相似度为指定笔记推荐相关笔记链接".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["rel_path"],
                    "properties": {
                        "rel_path": { "type": "string" },
                        "max_results": { "type": "integer" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "recommendations": { "type": "array" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["link".to_string(), "recommendation".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for LinkSuggestRelatedTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Graph
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let rel_path = match input.get("rel_path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "rel_path is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        // 路径安全性校验
        if let Err(e) = crate::note_privacy::validate_workspace_rel_path(&rel_path) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: e,
                    retryable: false,
                    cause: None,
                },
            };
        }

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(20) as usize;

        let root = ctx.workspace_root.clone();
        let rel = rel_path.clone();

        let result = tauri::async_runtime::spawn_blocking(
            move || -> Result<Vec<crate::link_recommendation::LinkRecommendation>, String> {
                let full_path = root.join(&rel);

                // 检查文件是否存在
                if !full_path.exists() {
                    return Err("__NOT_FOUND__".to_string());
                }

                // 检查是否私密
                if crate::note_privacy::peek_kf_private_from_md_file(&full_path) {
                    return Err("__PRIVACY_BLOCKED__".to_string());
                }

                let emb_conn = crate::semantic_index::open_embedding_db(&root)?;
                let thoughts_conn = crate::vault_thoughts_db::open_thoughts_db(&root)?;
                crate::link_recommendation::suggest_related_notes(
                    &root,
                    &rel,
                    &emb_conn,
                    &thoughts_conn,
                    max_results,
                    None,
                )
            },
        )
        .await;

        let recommendations = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) if e == "__NOT_FOUND__" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NotFound,
                        message: format!("note not found: {rel_path}"),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) if e == "__PRIVACY_BLOCKED__" => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::PrivacyBlocked,
                        message: "note is marked as private".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            Ok(Err(e)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e,
                        retryable: true,
                        cause: None,
                    },
                }
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: e.to_string(),
                        retryable: true,
                        cause: None,
                    },
                }
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::json!({
            "recommendations": serde_json::to_value(&recommendations).unwrap_or(serde_json::json!([]))
        });

        ToolResult::Ok {
            data,
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}
