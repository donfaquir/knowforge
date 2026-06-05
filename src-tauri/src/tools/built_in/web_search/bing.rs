use serde::Deserialize;

use super::{SearchError, SearchResult};

const BING_ENDPOINT: &str = "https://api.bing.microsoft.com/v7.0/search";

pub struct BingProvider {
    api_key: String,
}

impl BingProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }

    pub async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        max_results: u8,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let response = client
            .get(BING_ENDPOINT)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .query(&[
                ("q", query),
                ("count", &max_results.to_string()),
                ("mkt", "zh-CN"),
                ("textFormat", "Raw"),
            ])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    SearchError {
                        message: format!("Bing API request timed out: {e}"),
                        retryable: true,
                    }
                } else {
                    SearchError {
                        message: format!("Bing API network error: {e}"),
                        retryable: true,
                    }
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SearchError {
                message: "Invalid Bing API key (401 Unauthorized)".to_string(),
                retryable: false,
            });
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SearchError {
                message: "Bing API rate limited (429)".to_string(),
                retryable: true,
            });
        }
        if !status.is_success() {
            return Err(SearchError {
                message: format!("Bing API error: HTTP {status}"),
                retryable: status.is_server_error(),
            });
        }

        let body: BingResponse = response.json().await.map_err(|e| SearchError {
            message: format!("Failed to parse Bing API response: {e}"),
            retryable: false,
        })?;

        let results = body
            .web_pages
            .map(|wp| {
                wp.value
                    .into_iter()
                    .map(|page| SearchResult {
                        title: page.name,
                        url: page.url,
                        snippet: page.snippet,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BingResponse {
    web_pages: Option<BingWebPages>,
}

#[derive(Deserialize)]
struct BingWebPages {
    value: Vec<BingWebPage>,
}

#[derive(Deserialize)]
struct BingWebPage {
    name: String,
    url: String,
    snippet: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bing_response_parsing() {
        let json = r#"{
            "webPages": {
                "value": [
                    {"name": "Rust Lang", "url": "https://rust-lang.org", "snippet": "A systems language"},
                    {"name": "Crates.io", "url": "https://crates.io", "snippet": "Rust packages"}
                ]
            }
        }"#;
        let resp: BingResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = resp
            .web_pages
            .unwrap()
            .value
            .into_iter()
            .map(|p| SearchResult {
                title: p.name,
                url: p.url,
                snippet: p.snippet,
            })
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Lang");
        assert_eq!(results[1].url, "https://crates.io");
    }

    #[test]
    fn test_bing_empty_response() {
        let json = r#"{}"#;
        let resp: BingResponse = serde_json::from_str(json).unwrap();
        let results: Vec<SearchResult> = resp
            .web_pages
            .map(|wp| {
                wp.value
                    .into_iter()
                    .map(|p| SearchResult {
                        title: p.name,
                        url: p.url,
                        snippet: p.snippet,
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(results.len(), 0);
    }
}
