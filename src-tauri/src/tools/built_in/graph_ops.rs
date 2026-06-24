use async_trait::async_trait;
use serde_json::Value;

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

// ─── GraphQueryTopicNetworkTool ────────────────────────────────────────────────

pub struct GraphQueryTopicNetworkTool {
    manifest: ToolManifest,
}

impl GraphQueryTopicNetworkTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "graph-query_topic_network".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description:
                    "查询工作区的主题网络图（话题节点、文档节点及边关系）".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "topic_nodes": { "type": "array" },
                        "doc_nodes": { "type": "array" },
                        "topic_doc_edges": { "type": "array" },
                        "topic_topic_edges": { "type": "array" },
                        "meta": { "type": "object" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["graph".to_string(), "topic".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for GraphQueryTopicNetworkTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Graph
    }

    async fn invoke(&self, ctx: &ToolContext, _input: Value) -> ToolResult {
        let start = std::time::Instant::now();
        let root = ctx.workspace_root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || {
            let conn = crate::topic_network::open_topic_db(&root)?;
            crate::topic_network::load_topic_network_graph(&root, &conn)
        })
        .await;

        let network = match result {
            Ok(Ok(n)) => n,
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
        let data = serde_json::to_value(&network).unwrap_or(serde_json::json!({}));

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

// ─── IndexStatusTool ──────────────────────────────────────────────────────────

pub struct IndexStatusTool {
    manifest: ToolManifest,
}

impl IndexStatusTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "index-status".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description:
                    "查询工作区语义索引状态：文档块数量、想法嵌入数量及模型是否就绪"
                        .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "doc_chunk_count": { "type": "integer" },
                        "thought_embedding_count": { "type": "integer" },
                        "model_ready": { "type": "boolean" }
                    }
                }),
                effects: vec![Effect::Read],
                risk: Risk::Safe,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["index".to_string(), "status".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for IndexStatusTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Graph
    }

    async fn invoke(&self, ctx: &ToolContext, _input: Value) -> ToolResult {
        let start = std::time::Instant::now();
        let root = ctx.workspace_root.clone();
        let cache_dir_opt = ctx.app_cache_dir.clone();
        let bundle_dir_opt = ctx.app_bundle_resource_dir.clone();
        let embed_cache = ctx.embed_cache.clone();

        let result = tauri::async_runtime::spawn_blocking(
            move || -> Result<(usize, usize, bool), String> {
                let conn = crate::semantic_index::open_embedding_db(&root)?;
                let fallback_cache;
                let cache_ref = match embed_cache.as_ref() {
                    Some(c) => c.as_ref(),
                    None => {
                        fallback_cache = crate::semantic_index::EmbeddingCache::new();
                        &fallback_cache
                    }
                };
                let doc_chunks = cache_ref.get_docs(&conn);
                let thought_embeddings = cache_ref.get_thoughts(&conn);

                let doc_chunk_count = doc_chunks.len();
                let thought_embedding_count = thought_embeddings.len();

                // 检查模型是否就绪
                let model_ready = match (cache_dir_opt, bundle_dir_opt) {
                    (Some(cache_dir), Some(bundle_dir)) => {
                        crate::semantic_index::get_cached_or_load_model(&cache_dir, &bundle_dir)
                            .is_ok()
                    }
                    _ => false,
                };

                Ok((doc_chunk_count, thought_embedding_count, model_ready))
            },
        )
        .await;

        let (doc_chunk_count, thought_embedding_count, model_ready) = match result {
            Ok(Ok(t)) => t,
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

        ToolResult::Ok {
            data: serde_json::json!({
                "doc_chunk_count": doc_chunk_count,
                "thought_embedding_count": thought_embedding_count,
                "model_ready": model_ready
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                ..Default::default()
            },
        }
    }
}
