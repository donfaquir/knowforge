use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};
use crate::vault_context_search::{
    SearchWorkspaceContextArgs, SearchWorkspaceLimits, VaultSnippetKind,
};
use crate::semantic_index::SemanticSearchArgs;

// ─── VaultSearchKeywordTool ────────────────────────────────────────────────────

pub struct VaultSearchKeywordTool {
    manifest: ToolManifest,
}

impl VaultSearchKeywordTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "vault.search_keyword".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "在工作区笔记中进行关键词全文扫描搜索，返回相关文本片段".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string", "minLength": 2 },
                        "exclude_paths": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "max_snippets": { "type": "integer" },
                        "max_chars_per_snippet": { "type": "integer" }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "snippets": { "type": "array" },
                        "meta": { "type": "object" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["vault".to_string(), "search".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for VaultSearchKeywordTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if q.len() >= 2 => q.to_string(),
            Some(_) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "query must be at least 2 characters".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "query is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let exclude_rel_paths: Vec<String> = input
            .get("exclude_paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let max_snippets = input
            .get("max_snippets")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as usize;
        let max_chars_per_snippet = input
            .get("max_chars_per_snippet")
            .and_then(|v| v.as_u64())
            .unwrap_or(1200) as usize;

        let root = ctx.workspace_root.clone();
        let args = SearchWorkspaceContextArgs {
            query,
            exclude_rel_paths,
            limits: Some(SearchWorkspaceLimits {
                max_snippets: Some(max_snippets),
                max_chars_per_snippet: Some(max_chars_per_snippet),
                ..Default::default()
            }),
        };

        let result = tauri::async_runtime::spawn_blocking(move || {
            crate::vault_context_search::search_workspace_context_blocking(&root, args)
        })
        .await;

        let resp = match result {
            Ok(Ok(r)) => r,
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

        // 统计被隐私过滤的 snippet 数量（vault_context_search 已将私密文件设为 PrivateOmitted）
        let redacted_count = resp
            .snippets
            .iter()
            .filter(|s| s.kind == VaultSnippetKind::PrivateOmitted)
            .count() as u32;

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::to_value(&resp).unwrap_or(serde_json::json!({}));

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

// ─── VaultSemanticSearchTool ───────────────────────────────────────────────────

pub struct VaultSemanticSearchTool {
    manifest: ToolManifest,
}

impl VaultSemanticSearchTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "vault.semantic_search".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "使用向量嵌入对工作区进行语义相似度搜索（基于 BGE 模型，不调用 LLM）"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "top_k": { "type": "integer" },
                        "scope": { "type": "string", "enum": ["docs", "thoughts", "all"] }
                    },
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "hits": { "type": "array" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec![
                    "vault".to_string(),
                    "semantic".to_string(),
                    "search".to_string(),
                ],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for VaultSemanticSearchTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::NoteRead
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = std::time::Instant::now();

        let cache_dir = match &ctx.app_cache_dir {
            Some(d) => d.clone(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::WorkspaceNotOpen,
                        message: "app cache directory is not available".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let bundle_dir = match &ctx.app_bundle_resource_dir {
            Some(d) => d.clone(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::WorkspaceNotOpen,
                        message: "app bundle resource directory is not available".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "query is required".to_string(),
                        retryable: false,
                        cause: None,
                    },
                }
            }
        };

        let top_k = input
            .get("top_k")
            .and_then(|v| v.as_u64())
            .unwrap_or(8) as usize;
        let search_scope = input
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let root = ctx.workspace_root.clone();
        let privacy_filter = Arc::clone(&ctx.privacy_filter);
        let workspace_root_for_filter = root.clone();

        let args = SemanticSearchArgs {
            query,
            top_k,
            search_scope,
        };

        let result = tauri::async_runtime::spawn_blocking(move || {
            crate::semantic_index::run_semantic_search(&root, &cache_dir, &bundle_dir, args)
        })
        .await;

        let hits = match result {
            Ok(Ok(h)) => h,
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

        // 过滤私密文件命中
        let mut redacted_count = 0u32;
        let filtered_hits: Vec<_> = hits
            .into_iter()
            .filter(|hit| {
                if let Some(ref rel_path) = hit.rel_path {
                    if privacy_filter.is_private_path(rel_path, &workspace_root_for_filter) {
                        redacted_count += 1;
                        return false;
                    }
                }
                true
            })
            .collect();

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::json!({
            "hits": serde_json::to_value(&filtered_hits).unwrap_or(serde_json::json!([]))
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
