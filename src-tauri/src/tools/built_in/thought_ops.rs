use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};
use crate::vault_thoughts_db;

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
                description: "Create a short, independent Thought entry (a few sentences). \
                              A thought captures a fleeting idea, spark of inspiration, or \
                              quick hypothesis — content too small to warrant its own note \
                              file. Do NOT use for structured documents, research reports, \
                              or long-form content (use note.create for those). Do NOT use \
                              for behavioral instructions or preferences (use memory.save).".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "content": { "type": "string", "description": "The thought content" },
                        "summary": { "type": "string", "description": "One-line summary (optional)" }
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

// ─── ThoughtReadTool ─────────────────────────────────────────────────────────

pub struct ThoughtReadTool {
    manifest: ToolManifest,
}

impl ThoughtReadTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "thought.read".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Read the full body and metadata of a single Thought by its ID. \
                              Use thought.list first to discover thought IDs, then call this \
                              tool to retrieve the complete content for deeper analysis, \
                              challenge review, or linking."
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["thought_id"],
                    "properties": {
                        "thought_id": {
                            "type": "string",
                            "description": "The thought ID returned by thought.list"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "thought_id": { "type": "string" },
                        "body": { "type": "string" },
                        "summary": { "type": ["string", "null"] },
                        "maturity": { "type": "string" },
                        "temporary": { "type": "boolean" },
                        "standalone": { "type": "boolean" },
                        "note_rel_path": { "type": "string" },
                        "created_at": { "type": "string" },
                        "updated_at": { "type": "string" },
                        "challenge_pass_count": { "type": "integer" },
                        "last_reviewed_at": { "type": ["string", "null"] }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["thought".to_string(), "read".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for ThoughtReadTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let thought_id = match input.get("thought_id").and_then(|v| v.as_str()) {
            Some(id) if !id.trim().is_empty() => id.to_string(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "thought_id is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let root = ctx.workspace_root.clone();
        let privacy_filter = Arc::clone(&ctx.privacy_filter);
        let workspace_root_for_filter = root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || {
            let conn = vault_thoughts_db::open_thoughts_db(&root)?;
            vault_thoughts_db::get_thought_detail(&conn, &thought_id)
        })
        .await;

        let detail = match result {
            Ok(Ok(Some(d))) => d,
            Ok(Ok(None)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NotFound,
                        message: "thought not found".to_string(),
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

        if should_block_private_thought(&detail, &*privacy_filter, &workspace_root_for_filter) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::PrivacyBlocked,
                    message: "this thought is linked to a private note".to_string(),
                    retryable: false,
                    cause: None,
                },
            };
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::json!({
            "thought_id": detail.thought_id,
            "body": detail.body,
            "summary": detail.summary,
            "maturity": detail.maturity,
            "temporary": detail.temporary,
            "standalone": detail.standalone,
            "note_rel_path": detail.note_rel_path,
            "created_at": detail.created_at,
            "updated_at": detail.updated_at,
            "challenge_pass_count": detail.challenge_pass_count,
            "last_reviewed_at": detail.last_reviewed_at,
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

fn should_block_private_thought(
    detail: &vault_thoughts_db::ThoughtDetail,
    privacy_filter: &dyn crate::tools::context::PrivacyFilter,
    workspace_root: &std::path::Path,
) -> bool {
    !detail.standalone
        && !detail.note_rel_path.is_empty()
        && privacy_filter.is_private_path(&detail.note_rel_path, workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn blocks_thought_linked_to_private_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let private = root.join("secret.md");
        let mut f = std::fs::File::create(&private).unwrap();
        writeln!(f, "---\nkf-private: true\n---\nSecret content").unwrap();

        let detail = vault_thoughts_db::ThoughtDetail {
            thought_id: "t-001".into(),
            note_stable_id: "s-001".into(),
            note_rel_path: "secret.md".into(),
            body: "some private idea".into(),
            summary: None,
            maturity: "seedling".into(),
            temporary: false,
            standalone: false,
            created_at: "2026-06-01T00:00:00Z".into(),
            updated_at: "2026-06-01T00:00:00Z".into(),
            challenge_pass_count: 0,
            last_reviewed_at: None,
        };

        let filter = crate::tools::privacy::KfPrivateFilter;
        assert!(should_block_private_thought(&detail, &filter, &root));
    }

    #[test]
    fn allows_thought_linked_to_public_note() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let public = root.join("public.md");
        let mut f = std::fs::File::create(&public).unwrap();
        writeln!(f, "---\ntitle: Public\n---\nPublic content").unwrap();

        let detail = vault_thoughts_db::ThoughtDetail {
            thought_id: "t-002".into(),
            note_stable_id: "s-002".into(),
            note_rel_path: "public.md".into(),
            body: "a public thought".into(),
            summary: Some("public".into()),
            maturity: "budding".into(),
            temporary: false,
            standalone: false,
            created_at: "2026-06-01T00:00:00Z".into(),
            updated_at: "2026-06-01T00:00:00Z".into(),
            challenge_pass_count: 1,
            last_reviewed_at: Some("2026-06-01T00:00:00Z".into()),
        };

        let filter = crate::tools::privacy::KfPrivateFilter;
        assert!(!should_block_private_thought(&detail, &filter, &root));
    }

    #[test]
    fn allows_standalone_thought_regardless() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let detail = vault_thoughts_db::ThoughtDetail {
            thought_id: "t-003".into(),
            note_stable_id: "t-003".into(),
            note_rel_path: ".knowforge/standalone/t-003".into(),
            body: "standalone idea".into(),
            summary: None,
            maturity: "seedling".into(),
            temporary: false,
            standalone: true,
            created_at: "2026-06-01T00:00:00Z".into(),
            updated_at: "2026-06-01T00:00:00Z".into(),
            challenge_pass_count: 0,
            last_reviewed_at: None,
        };

        let filter = crate::tools::privacy::KfPrivateFilter;
        assert!(!should_block_private_thought(&detail, &filter, &root));
    }
}
