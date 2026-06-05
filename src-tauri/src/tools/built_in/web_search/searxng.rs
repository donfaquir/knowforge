use serde::Deserialize;

use super::{SearchError, SearchResult};

pub struct SearxngProvider {
    base_url: String,
}

impl SearxngProvider {
    pub fn new(base_url: String) -> Result<Self, String> {
        let trimmed = base_url.trim_end_matches('/').to_string();
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(format!(
                "SearXNG base URL must start with http:// or https://, got: {trimmed}"
            ));
        }
        Ok(Self { base_url: trimmed })
    }

    pub async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        max_results: u8,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let response = client
            .get(format!("{}/search", self.base_url))
            .query(&[
                ("q", query),
                ("format", "json"),
                ("categories", "general"),
                ("pageno", "1"),
            ])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError {
                        message: format!("SearXNG request timed out: {e}"),
                        retryable: true,
                    }
                } else {
                    SearchError {
                        message: format!(
                            "Cannot connect to SearXNG at {}: {e}",
                            self.base_url
                        ),
                        retryable: true,
                    }
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(SearchError {
                message: format!("SearXNG error: HTTP {status}"),
                retryable: status.is_server_error(),
            });
        }

        let body: SearxngResponse = response.json().await.map_err(|e| SearchError {
            message: format!("SearXNG returned non-JSON response: {e}"),
            retryable: false,
        })?;

        let results: Vec<SearchResult> = body
            .results
            .into_iter()
            .take(max_results as usize)
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
struct SearxngResponse {
    results: Vec<SearxngResult>,
}

#[derive(Deserialize)]
struct SearxngResult {
    title: String,
    url: String,
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_searxng_response_parsing() {
        let json = r#"{
            "results": [
                {"title": "Rust Lang", "url": "https://rust-lang.org", "content": "A systems language"},
                {"title": "Crates.io", "url": "https://crates.io", "content": "Rust packages"}
            ]
        }"#;
        let resp: SearxngResponse = serde_json::from_str(json).unwrap();
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
        assert_eq!(results[1].snippet, "Rust packages");
    }

    #[test]
    fn test_searxng_url_normalization() {
        let p = SearxngProvider::new("http://localhost:8080/".to_string()).unwrap();
        assert_eq!(p.base_url, "http://localhost:8080");

        let p = SearxngProvider::new("https://search.example.com///".to_string()).unwrap();
        assert_eq!(p.base_url, "https://search.example.com");
    }

    #[test]
    fn test_searxng_rejects_invalid_scheme() {
        let err = SearxngProvider::new("ftp://search.local".to_string());
        assert!(err.is_err());
        let msg = err.err().unwrap();
        assert!(msg.contains("http://"));
    }

    #[test]
    fn test_searxng_empty_response() {
        let json = r#"{"results": []}"#;
        let resp: SearxngResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }
}
