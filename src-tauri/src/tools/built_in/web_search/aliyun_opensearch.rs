use serde::Deserialize;
use serde_json::json;

use super::{SearchError, SearchResult};

#[derive(Debug)]
pub struct AliyunOpensearchProvider {
    endpoint: String,
    api_key: String,
}

impl AliyunOpensearchProvider {
    pub fn new(endpoint: String, api_key: String) -> Result<Self, String> {
        let trimmed = endpoint.trim_end_matches('/').to_string();
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(format!(
                "OpenSearch endpoint must start with http:// or https://, got: {trimmed}"
            ));
        }
        Ok(Self {
            endpoint: trimmed,
            api_key,
        })
    }

    pub async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        max_results: u8,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = json!({
            "query": query,
            "query_rewrite": false,
            "top_k": max_results,
            "content_type": "snippet"
        });

        let response = client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError {
                        message: "Aliyun OpenSearch request timed out".to_string(),
                        retryable: true,
                    }
                } else {
                    SearchError {
                        message: format!("Aliyun OpenSearch network error: {e}"),
                        retryable: true,
                    }
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError {
                message: "Invalid Aliyun OpenSearch API key (401 Unauthorized)".to_string(),
                retryable: false,
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError {
                message: "Aliyun OpenSearch rate limited (429) — QPS limit is 3".to_string(),
                retryable: true,
            });
        }
        if !status.is_success() {
            return Err(SearchError {
                message: format!("Aliyun OpenSearch API error: HTTP {status}"),
                retryable: status.is_server_error(),
            });
        }

        let body: ApiResponse = response.json().await.map_err(|e| SearchError {
            message: format!("Failed to parse Aliyun OpenSearch response: {e}"),
            retryable: false,
        })?;

        let results: Vec<SearchResult> = body
            .result
            .search_result
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.link,
                snippet: r.snippet.unwrap_or_default(),
            })
            .collect();

        Ok(results)
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    result: ApiResult,
}

#[derive(Deserialize)]
struct ApiResult {
    search_result: Vec<ApiSearchItem>,
}

#[derive(Deserialize)]
struct ApiSearchItem {
    title: String,
    link: String,
    snippet: Option<String>,
    #[allow(dead_code)]
    content: Option<String>,
    #[allow(dead_code)]
    position: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_parsing() {
        let json = r#"{
            "result": {
                "search_result": [
                    {
                        "title": "Rust Programming",
                        "link": "https://rust-lang.org",
                        "snippet": "A systems programming language",
                        "content": "Full content here",
                        "position": 1
                    },
                    {
                        "title": "Go Language",
                        "link": "https://go.dev",
                        "snippet": "An open source language",
                        "content": null,
                        "position": 2
                    }
                ]
            },
            "usage": {
                "search_count": 1,
                "rewrite_model.total_tokens": 40,
                "filter_model.total_tokens": 150
            }
        }"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = resp
            .result
            .search_result
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.link,
                snippet: r.snippet.unwrap_or_default(),
            })
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming");
        assert_eq!(results[0].url, "https://rust-lang.org");
        assert_eq!(results[0].snippet, "A systems programming language");
        assert_eq!(results[1].url, "https://go.dev");
    }

    #[test]
    fn test_empty_response() {
        let json = r#"{"result": {"search_result": []}}"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(resp.result.search_result.is_empty());
    }

    #[test]
    fn test_endpoint_normalization() {
        let p = AliyunOpensearchProvider::new(
            "http://example-hangzhou.opensearch.aliyuncs.com/v3/test/".to_string(),
            "key".to_string(),
        )
        .unwrap();
        assert_eq!(
            p.endpoint,
            "http://example-hangzhou.opensearch.aliyuncs.com/v3/test"
        );
    }

    #[test]
    fn test_rejects_invalid_scheme() {
        let err = AliyunOpensearchProvider::new("ftp://bad.url".to_string(), "key".to_string());
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("http://"));
    }

    #[test]
    fn test_missing_snippet_defaults_to_empty() {
        let json = r#"{
            "result": {
                "search_result": [
                    {"title": "No Snippet", "link": "https://example.com", "position": 1}
                ]
            }
        }"#;
        let resp: ApiResponse = serde_json::from_str(json).unwrap();
        let item = &resp.result.search_result[0];
        assert!(item.snippet.is_none());
    }
}
