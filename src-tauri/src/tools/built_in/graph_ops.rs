use std::collections::HashSet;
use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;

use crate::topic_network::TopicNetworkForUi;

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
                name: "graph.query_topic_network".to_string(),
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
        let root_for_blocking = root.clone();

        let result = tauri::async_runtime::spawn_blocking(move || {
            let conn = crate::topic_network::open_topic_db(&root_for_blocking)?;
            crate::topic_network::load_topic_network_graph(&root_for_blocking, &conn)
        })
        .await;

        let mut network = match result {
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

        let redacted = filter_private_nodes(&mut network, &root);

        let duration_ms = start.elapsed().as_millis() as u64;
        let data = serde_json::to_value(&network).unwrap_or(serde_json::json!({}));

        ToolResult::Ok {
            data,
            redacted_count: redacted as u32,
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
                name: "index.status".to_string(),
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

// ─── Privacy filtering ──────────────────────────────────────────────────────

fn filter_private_nodes(network: &mut TopicNetworkForUi, workspace_root: &Path) -> u32 {
    let private_paths: HashSet<String> = network
        .doc_nodes
        .iter()
        .filter(|n| {
            let full = workspace_root.join(&n.rel_path);
            crate::note_privacy::peek_kf_private_from_md_file(&full)
        })
        .map(|n| n.rel_path.clone())
        .collect();

    let redacted = private_paths.len() as u32;
    network
        .doc_nodes
        .retain(|n| !private_paths.contains(&n.rel_path));
    network
        .topic_doc_edges
        .retain(|e| !private_paths.contains(&e.doc_rel_path));
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topic_network::{DocNode, TopicDocEdge, TopicNetworkMeta, TopicNode};
    use std::io::Write;

    fn make_network(doc_paths: &[&str], edges: &[(&str, &str)]) -> TopicNetworkForUi {
        TopicNetworkForUi {
            topic_nodes: vec![TopicNode {
                id: "t1".into(),
                name: "rust".into(),
                doc_count: doc_paths.len(),
                related_topic_count: 0,
            }],
            doc_nodes: doc_paths
                .iter()
                .map(|p| DocNode {
                    rel_path: p.to_string(),
                    topic_count: 1,
                    thought_count: 0,
                    max_maturity: "seed".into(),
                })
                .collect(),
            topic_doc_edges: edges
                .iter()
                .map(|(tid, dp)| TopicDocEdge {
                    topic_id: tid.to_string(),
                    doc_rel_path: dp.to_string(),
                })
                .collect(),
            topic_topic_edges: vec![],
            meta: TopicNetworkMeta {
                topic_node_cap: 50,
                doc_node_cap: 50,
                truncated_topic_count: 0,
                truncated_doc_count: 0,
                extract_skipped_no_llm: false,
            },
        }
    }

    #[test]
    fn private_doc_nodes_and_edges_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let private_path = dir.path().join("secret.md");
        let mut f = std::fs::File::create(&private_path).unwrap();
        writeln!(f, "---\nkf-private: true\n---\nSecret content").unwrap();

        let public_path = dir.path().join("public.md");
        let mut f2 = std::fs::File::create(&public_path).unwrap();
        writeln!(f2, "---\ntitle: Public\n---\nPublic content").unwrap();

        let mut network = make_network(
            &["secret.md", "public.md"],
            &[("t1", "secret.md"), ("t1", "public.md")],
        );

        let redacted = filter_private_nodes(&mut network, dir.path());

        assert_eq!(redacted, 1);
        assert_eq!(network.doc_nodes.len(), 1);
        assert_eq!(network.doc_nodes[0].rel_path, "public.md");
        assert_eq!(network.topic_doc_edges.len(), 1);
        assert_eq!(network.topic_doc_edges[0].doc_rel_path, "public.md");
    }

    #[test]
    fn no_private_docs_returns_zero_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("note.md");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "---\ntitle: Note\n---\nContent").unwrap();

        let mut network = make_network(&["note.md"], &[("t1", "note.md")]);

        let redacted = filter_private_nodes(&mut network, dir.path());

        assert_eq!(redacted, 0);
        assert_eq!(network.doc_nodes.len(), 1);
        assert_eq!(network.topic_doc_edges.len(), 1);
    }

    #[test]
    fn topic_nodes_preserved_when_doc_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("secret.md");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, "---\nkf-private: true\n---\nSecret").unwrap();

        let mut network = make_network(&["secret.md"], &[("t1", "secret.md")]);

        filter_private_nodes(&mut network, dir.path());

        assert!(network.doc_nodes.is_empty());
        assert!(network.topic_doc_edges.is_empty());
        assert_eq!(network.topic_nodes.len(), 1, "topic node must not be removed");
    }
}
