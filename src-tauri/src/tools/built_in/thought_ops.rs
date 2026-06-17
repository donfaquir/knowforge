use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

// ─── ThoughtListTool ───────────────────────────────────────────────────────────

pub struct ThoughtListTool {
    manifest: ToolManifest,
}

impl ThoughtListTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "thought.list".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "列出工作区中的想法（Thought）条目，支持关键词过滤和分页".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "filter": {
                            "type": "string",
                            "enum": ["all", "standalone", "linked", "temporary"]
                        },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "rows": { "type": "array" },
                        "total": { "type": "integer" },
                        "redacted_count": { "type": "integer" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["thought".to_string(), "list".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for ThoughtListTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let filter = input
            .get("filter")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .min(200) as usize;
        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let root = ctx.workspace_root.clone();
        let privacy_filter = Arc::clone(&ctx.privacy_filter);
        let workspace_root_for_filter = root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || {
            let conn = crate::vault_thoughts_db::open_thoughts_db(&root)?;
            crate::vault_thoughts_db::list_vault_thought_rows_paged(
                &conn,
                limit,
                offset,
                query.as_deref(),
                filter.as_deref(),
            )
        })
        .await;

        let page = match result {
            Ok(Ok(p)) => p,
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

        // 隐私过滤：过滤掉关联了私密笔记的想法
        let mut redacted_count = 0u32;
        let filtered_rows: Vec<_> = page
            .rows
            .into_iter()
            .filter(|row| {
                // 独立想法（standalone）或空 rel_path 无需检查文件私密性
                if row.standalone || row.rel_path.is_empty() {
                    return true;
                }
                if privacy_filter.is_private_path(&row.rel_path, &workspace_root_for_filter) {
                    redacted_count += 1;
                    return false;
                }
                true
            })
            .collect();

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::json!({
            "rows": serde_json::to_value(&filtered_rows).unwrap_or(serde_json::json!([])),
            "total": page.total,
            "redacted_count": redacted_count
        });

        ToolResult::Ok {
            data,
            redacted_count,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}

// ─── ThoughtCreateTool ────────────────────────────────────────────────────────

pub struct ThoughtCreateTool {
    manifest: ToolManifest,
}

impl ThoughtCreateTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "thought.create".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "创建一条新的独立想法（Thought）条目。想法是用户对某个主题的思考、洞察、灵感或假设，不包括对 AI 的行为指令、个人偏好或记忆指令（如「记住…」「以后都…」「always…」「never…」）。".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "content": { "type": "string", "description": "想法正文" },
                        "summary": { "type": "string", "description": "一句话摘要（可选）" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "thought_id": { "type": "string" }
                    }
                }),
                effects: vec![Effect::Write],
                risk: Risk::Caution,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::ConfirmOncePerSession,
                examples: vec![],
                tags: vec!["thought".to_string(), "create".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for ThoughtCreateTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteWrite
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "content is required and must not be empty".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };
        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let root = ctx.workspace_root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
            let conn = crate::vault_thoughts_db::open_thoughts_db(&root)?;
            crate::vault_thoughts_db::create_standalone_thought(
                &conn,
                &content,
                summary.as_deref(),
            )
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(thought_id)) => ToolResult::Ok {
                data: serde_json::json!({ "thought_id": thought_id }),
                redacted_count: 0,
                warnings: vec![],
                metrics: ToolMetrics { duration_ms, ..Default::default() },
            },
            Ok(Err(e)) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e,
                    retryable: true,
                    cause: None,
                },
            },
            Err(e) => ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: e.to_string(),
                    retryable: true,
                    cause: None,
                },
            },
        }
    }
}
