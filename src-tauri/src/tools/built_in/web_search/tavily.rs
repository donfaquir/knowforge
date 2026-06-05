use serde::Deserialize;
use serde_json::json;

use super::{SearchError, SearchResult};

const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";

pub struct TavilyProvider {
    api_key: String,
}

impl TavilyProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        max_results: u8,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = json!({
            "api_key": self.api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
            "include_answer": false,
            "include_raw_content": false
        });

        let response = client
            .post(TAVILY_ENDPOINT)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError {
                        message: "Connection timeout - Tavily may not be accessible from your network".to_string(),
                        retryable: true,
                    }
                } else {
                    SearchError {
                        message: format!("Tavily API network error: {e}"),
                        retryable: true,
                    }
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError {
                message: "Invalid Tavily API key (401 Unauthorized)".to_string(),
                retryable: false,
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError {
                message: "Tavily API rate limited (429)".to_string(),
                retryable: true,
            });
        }
        if !status.is_success() {
            return Err(SearchError {
                message: format!("Tavily API error: HTTP {status}"),
                retryable: status.is_server_error(),
            });
        }

        let body: TavilyResponse = response.json().await.map_err(|e| SearchError {
            message: format!("Failed to parse Tavily API response: {e}"),
            retryable: false,
        })?;

        let results: Vec<SearchResult> = body
            .results
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.content,
            })
            .collect();

        Ok(results)
    }
}

#[derive(Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

#[derive(Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
    #[allow(dead_code)]
    score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tavily_response_parsing() {
        let json = r#"{
            "results": [
                {"title": "Rust Lang", "url": "https://rust-lang.org", "content": "A systems programming language", "score": 0.95},
                {"title": "Go Lang", "url": "https://go.dev", "content": "An open source language", "score": 0.88}
            ]
        }"#;
        let resp: TavilyResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = resp
            .results
            .into_iter()
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.content,
            })
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Lang");
        assert_eq!(results[0].snippet, "A systems programming language");
        assert_eq!(results[1].url, "https://go.dev");
    }

    #[test]
    fn test_tavily_empty_response() {
        let json = r#"{"results": []}"#;
        let resp: TavilyResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }
}
