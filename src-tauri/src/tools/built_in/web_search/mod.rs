pub mod aliyun_opensearch;
pub mod searxng;
pub mod tavily;

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};
use crate::vault_config::{self, SearchProviderType};

use aliyun_opensearch::AliyunOpensearchProvider;
use searxng::SearxngProvider;
use tavily::TavilyProvider;

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug)]
pub struct SearchError {
    pub message: String,
    pub retryable: bool,
}

// ─── Provider factory ─────────────────────────────────────────────────────────

const NOT_CONFIGURED_MSG: &str = r#"Search provider not configured.

To enable web search, add a "search" section to .knowforge/config.json:

{
  "search": {
    "provider": "searxng",
    "searxng": { "baseUrl": "http://localhost:8080" }
  }
}

Supported providers: searxng (self-hosted), tavily (api key from tavily.com), aliyun-opensearch (Alibaba Cloud AI Search)."#;

// ─── Tool implementation ──────────────────────────────────────────────────────

pub struct WebSearchTool {
    manifest: ToolManifest,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "web.search".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Search the web by keyword query (NOT a URL). Use only when you need to discover pages — if the user already provided a URL, use web.read_page instead".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Keyword search query (must NOT be a URL)",
                            "minLength": 2
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of search results to return",
                            "default": 5,
                            "minimum": 1,
                            "maximum": 10
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                output_schema: json!({}),
                effects: vec![Effect::Network],
                risk: Risk::Caution,
                privacy_aware: false,
                requires_workspace: true,
                default_approval: ApprovalPolicy::Auto,
                examples: vec![],
                tags: vec!["web".to_string(), "search".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = Instant::now();

        // ── Parse input ────────────────────────────────────────────────
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(s) if s.len() >= 2 => s.to_string(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "Missing or too short 'query' parameter (min 2 chars)".to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 10) as u8)
            .unwrap_or(5);

        // ── Load search config ─────────────────────────────────────────
        let workspace_root = &ctx.workspace_root;

        let config = match vault_config::load_search_config(workspace_root) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: format!("Failed to load search config: {e}"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        // ── Resolve provider ───────────────────────────────────────────
        let provider_type = match config.provider {
            Some(p) => p,
            None => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: NOT_CONFIGURED_MSG.to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        // ── Build HTTP client ──────────────────────────────────────────
        let timeout_secs = match provider_type {
            SearchProviderType::Tavily => 15,
            SearchProviderType::AliyunOpensearch => 8,
            _ => 10,
        };
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: format!("Failed to create HTTP client: {e}"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        // ── Execute search ─────────────────────────────────────────────
        let (results, provider_name) = match provider_type {
            SearchProviderType::Searxng => {
                let base_url = match &config.searxng {
                    Some(cfg) if !cfg.base_url.is_empty() => cfg.base_url.clone(),
                    _ => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: ToolErrorCode::InvalidInput,
                                message: "SearXNG provider selected but searxng.baseUrl not configured in .knowforge/config.json".to_string(),
                                retryable: false,
                                cause: None,
                            },
                        };
                    }
                };
                let provider = match SearxngProvider::new(base_url) {
                    Ok(p) => p,
                    Err(e) => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: ToolErrorCode::InvalidInput,
                                message: e,
                                retryable: false,
                                cause: None,
                            },
                        };
                    }
                };
                match provider.search(&client, &query, max_results).await {
                    Ok(r) => (r, "searxng"),
                    Err(e) => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: if e.retryable {
                                    ToolErrorCode::Timeout
                                } else {
                                    ToolErrorCode::PermissionDenied
                                },
                                message: e.message,
                                retryable: e.retryable,
                                cause: None,
                            },
                        };
                    }
                }
            }
            SearchProviderType::Tavily => {
                let api_key = match &config.tavily {
                    Some(cfg) if !cfg.api_key.is_empty() => cfg.api_key.clone(),
                    _ => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: ToolErrorCode::InvalidInput,
                                message: "Tavily provider selected but tavily.apiKey not configured in .knowforge/config.json".to_string(),
                                retryable: false,
                                cause: None,
                            },
                        };
                    }
                };
                let provider = TavilyProvider::new(api_key);
                match provider.search(&client, &query, max_results).await {
                    Ok(r) => (r, "tavily"),
                    Err(e) => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: if e.retryable {
                                    ToolErrorCode::Timeout
                                } else {
                                    ToolErrorCode::PermissionDenied
                                },
                                message: e.message,
                                retryable: e.retryable,
                                cause: None,
                            },
                        };
                    }
                }
            }
            SearchProviderType::AliyunOpensearch => {
                let (endpoint, api_key) = match &config.aliyun_opensearch {
                    Some(cfg) if !cfg.endpoint.is_empty() && !cfg.api_key.is_empty() => {
                        (cfg.endpoint.clone(), cfg.api_key.clone())
                    }
                    _ => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: ToolErrorCode::InvalidInput,
                                message: "aliyun-opensearch provider selected but aliyunOpensearch.endpoint/apiKey not configured in .knowforge/config.json".to_string(),
                                retryable: false,
                                cause: None,
                            },
                        };
                    }
                };
                let provider = match AliyunOpensearchProvider::new(endpoint, api_key) {
                    Ok(p) => p,
                    Err(e) => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: ToolErrorCode::InvalidInput,
                                message: e,
                                retryable: false,
                                cause: None,
                            },
                        };
                    }
                };
                match provider.search(&client, &query, max_results).await {
                    Ok(r) => (r, "aliyun-opensearch"),
                    Err(e) => {
                        return ToolResult::Err {
                            error: ToolError {
                                code: if e.retryable {
                                    ToolErrorCode::Timeout
                                } else {
                                    ToolErrorCode::PermissionDenied
                                },
                                message: e.message,
                                retryable: e.retryable,
                                cause: None,
                            },
                        };
                    }
                }
            }
        };

        let total_results = results.len();
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: json!({
                "results": results,
                "provider": provider_name,
                "total_results": total_results
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::types::Effect;

    #[test]
    fn test_manifest_shape() {
        let tool = WebSearchTool::new();
        let m = tool.manifest();
        assert_eq!(m.name, "web.search");
        assert_eq!(m.effects, vec![Effect::Network]);
        assert_eq!(m.risk, Risk::Caution);
        assert!(m.requires_workspace);
    }

    #[test]
    fn test_not_configured_message_is_helpful() {
        assert!(NOT_CONFIGURED_MSG.contains("config.json"));
        assert!(NOT_CONFIGURED_MSG.contains("searxng"));
        assert!(NOT_CONFIGURED_MSG.contains("tavily"));
    }
}
