use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolError, ToolErrorCode, ToolManifest, ToolMetrics,
    ToolResult,
};

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB
const MAX_HTML_FOR_CONVERT: usize = 200_000; // 200 KB cap before markdown conversion
const DEFAULT_MAX_CHARS: u32 = 4000;
const USER_AGENT: &str = "KnowForge/0.6.1";

// ─── SSRF protection ──────────────────────────────────────────────────────────

fn is_private_address(url: &url::Url) -> bool {
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return true;
    }

    match url.host() {
        None => true,
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(v4)) => {
            v4.is_loopback()            // 127.0.0.0/8
                || v4.is_private()      // 10/8, 172.16/12, 192.168/16
                || v4.is_unspecified()  // 0.0.0.0
                || v4.is_link_local()   // 169.254/16
                || v4.is_broadcast()    // 255.255.255.255
        }
        Some(url::Host::Ipv6(v6)) => {
            v6.is_loopback()            // ::1
                || v6.is_unspecified()  // ::
        }
    }
}

// ─── HTML processing ──────────────────────────────────────────────────────────

fn extract_title(document: &scraper::Html) -> String {
    let selector = scraper::Selector::parse("title").unwrap();
    document
        .select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default()
}

fn clean_html(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);
    let remove_tags = [
        "script", "style", "nav", "footer", "header", "aside", "iframe", "noscript",
    ];

    let mut cleaned = html.to_string();
    for tag in &remove_tags {
        if let Ok(sel) = Selector::parse(tag) {
            for element in document.select(&sel) {
                let fragment = element.html();
                cleaned = cleaned.replace(&fragment, "");
            }
        }
    }

    cleaned
}

fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut consecutive_newlines = 0u32;

    for ch in text.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                result.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            result.push(ch);
        }
    }

    result
}

fn truncate_at_paragraph(text: &str, max_chars: usize) -> (String, bool) {
    if text.len() <= max_chars {
        return (text.to_string(), false);
    }

    let search_region = &text[..max_chars];
    if let Some(pos) = search_region.rfind("\n\n") {
        if pos > max_chars / 4 {
            return (text[..pos].to_string(), true);
        }
    }

    // Fall back to char boundary
    let truncated = &text[..max_chars];
    (truncated.to_string(), true)
}

// ─── Tool implementation ──────────────────────────────────────────────────────

pub struct WebReadPageTool {
    manifest: ToolManifest,
}

impl WebReadPageTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "web.read_page".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Fetch a web page and extract its content as clean Markdown text"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Target URL to fetch and extract content from"
                        },
                        "max_chars": {
                            "type": "integer",
                            "description": "Maximum characters to return from extracted content",
                            "default": DEFAULT_MAX_CHARS,
                            "minimum": 500,
                            "maximum": 20000
                        }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
                output_schema: json!({}),
                effects: vec![Effect::Network],
                risk: Risk::Caution,
                privacy_aware: false,
                requires_workspace: false,
                default_approval: ApprovalPolicy::ConfirmOncePerSession,
                examples: vec![],
                tags: vec!["web".to_string(), "read".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for WebReadPageTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    async fn invoke(&self, _ctx: &ToolContext, input: Value) -> ToolResult {
        let start = Instant::now();

        // ── Parse input ────────────────────────────────────────────────
        let url_str = match input.get("url").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: "Missing or empty 'url' parameter".to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let max_chars = input
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(500, 20000) as usize)
            .unwrap_or(DEFAULT_MAX_CHARS as usize);

        // ── Validate URL ───────────────────────────────────────────────
        let parsed_url = match url::Url::parse(&url_str) {
            Ok(u) => u,
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::InvalidInput,
                        message: format!("Invalid URL: {e}"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        if is_private_address(&parsed_url) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NetworkDenied,
                    message: "Request to private/loopback address is not allowed".to_string(),
                    retryable: false,
                    cause: None,
                },
            };
        }

        // ── HTTP request ───────────────────────────────────────────────
        let client = match reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(15))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
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

        let response = match client.get(parsed_url.as_str()).send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::Timeout,
                            message: format!("Request timed out: {e}"),
                            retryable: true,
                            cause: None,
                        },
                    };
                }
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NetworkDenied,
                        message: format!("Network error: {e}"),
                        retryable: true,
                        cause: None,
                    },
                };
            }
        };

        // ── Check status ───────────────────────────────────────────────
        let status = response.status();
        if status.is_client_error() {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("HTTP {status}"),
                    retryable: false,
                    cause: None,
                },
            };
        }
        if status.is_server_error() {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("HTTP {status}"),
                    retryable: true,
                    cause: None,
                },
            };
        }

        // ── Check Content-Type ─────────────────────────────────────────
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if !content_type.contains("text/html") {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::InvalidInput,
                    message: format!(
                        "Response is not HTML (Content-Type: {content_type}). \
                         This tool only supports HTML pages."
                    ),
                    retryable: false,
                    cause: None,
                },
            };
        }

        // ── Check Content-Length (if available) ────────────────────────
        if let Some(len) = response.content_length() {
            if len > MAX_RESPONSE_BYTES {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::BudgetExceeded,
                        message: format!(
                            "Response too large ({} bytes, max {MAX_RESPONSE_BYTES})",
                            len
                        ),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        }

        // ── Read body ──────────────────────────────────────────────────
        let body = match response.text().await {
            Ok(t) => {
                if t.len() as u64 > MAX_RESPONSE_BYTES {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::BudgetExceeded,
                            message: format!(
                                "Response body too large ({} bytes, max {MAX_RESPONSE_BYTES})",
                                t.len()
                            ),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
                t
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: format!("Failed to read response body: {e}"),
                        retryable: true,
                        cause: None,
                    },
                };
            }
        };

        let bytes_in = body.len() as u64;

        // ── Extract title (must finish before spawn_blocking moves body) ─
        let title = {
            let document = scraper::Html::parse_document(&body);
            extract_title(&document)
        };

        // ── Clean and convert (spawn_blocking to avoid stalling tokio) ─
        let markdown = match tokio::task::spawn_blocking(move || {
            let mut cleaned = clean_html(&body);
            if cleaned.len() > MAX_HTML_FOR_CONVERT {
                cleaned.truncate(MAX_HTML_FOR_CONVERT);
            }
            let md = htmd::convert(&cleaned).unwrap_or_default();
            collapse_blank_lines(&md)
        })
        .await
        {
            Ok(md) => md,
            Err(_) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: "HTML-to-Markdown conversion panicked".to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };
        let content_length = markdown.len();
        let (content, truncated) = truncate_at_paragraph(&markdown, max_chars);

        let bytes_out = content.len() as u64;
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: json!({
                "url": url_str,
                "title": title,
                "content": content,
                "content_length": content_length,
                "truncated": truncated
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                bytes_in,
                bytes_out,
                network_bytes: bytes_in,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SSRF tests ────────────────────────────────────────────────────

    #[test]
    fn test_ssrf_rejects_private_ips() {
        let cases = [
            "http://127.0.0.1/path",
            "http://127.0.0.42/path",
            "http://10.0.0.1/path",
            "http://10.255.255.255/path",
            "http://172.16.0.1/path",
            "http://172.31.255.255/path",
            "http://192.168.1.1/path",
            "http://192.168.0.100/path",
            "http://0.0.0.0/path",
            "http://localhost/path",
            "http://LOCALHOST/path",
            "http://[::1]/path",
        ];
        for case in &cases {
            let u = url::Url::parse(case).unwrap();
            assert!(is_private_address(&u), "should reject: {case}");
        }
    }

    #[test]
    fn test_ssrf_allows_public_urls() {
        let cases = [
            "https://www.rust-lang.org",
            "https://example.com/page",
            "http://8.8.8.8/dns",
            "https://1.1.1.1",
        ];
        for case in &cases {
            let u = url::Url::parse(case).unwrap();
            assert!(!is_private_address(&u), "should allow: {case}");
        }
    }

    #[test]
    fn test_rejects_non_http_scheme() {
        let cases = ["file:///etc/passwd", "ftp://example.com/file", "data:text/html,<h1>hi</h1>"];
        for case in &cases {
            let u = url::Url::parse(case).unwrap();
            assert!(is_private_address(&u), "should reject scheme: {case}");
        }
    }

    // ── HTML cleaning tests ───────────────────────────────────────────

    #[test]
    fn test_html_cleaning_removes_script_and_style() {
        let html = r#"<html><head><style>body{color:red}</style></head>
            <body><script>alert('x')</script><p>Hello world</p></body></html>"#;
        let cleaned = clean_html(html);
        assert!(!cleaned.contains("<script>"), "script should be removed");
        assert!(!cleaned.contains("<style>"), "style should be removed");
        assert!(cleaned.contains("Hello world"), "content should remain");
    }

    #[test]
    fn test_html_cleaning_removes_nav_footer() {
        let html = r#"<html><body>
            <nav><a href="/">Home</a></nav>
            <main><p>Main content</p></main>
            <footer><p>Copyright</p></footer>
            </body></html>"#;
        let cleaned = clean_html(html);
        assert!(!cleaned.contains("<nav>"), "nav should be removed");
        assert!(!cleaned.contains("<footer>"), "footer should be removed");
        assert!(cleaned.contains("Main content"), "main content should remain");
    }

    // ── Title extraction ──────────────────────────────────────────────

    #[test]
    fn test_title_extraction() {
        let html = "<html><head><title>  My Page Title  </title></head><body></body></html>";
        let doc = scraper::Html::parse_document(html);
        assert_eq!(extract_title(&doc), "My Page Title");
    }

    #[test]
    fn test_title_extraction_missing() {
        let html = "<html><head></head><body></body></html>";
        let doc = scraper::Html::parse_document(html);
        assert_eq!(extract_title(&doc), "");
    }

    // ── Blank line collapsing ─────────────────────────────────────────

    #[test]
    fn test_collapse_blank_lines() {
        let input = "line1\n\n\n\n\nline2\n\nline3";
        let result = collapse_blank_lines(input);
        assert_eq!(result, "line1\n\nline2\n\nline3");
    }

    #[test]
    fn test_collapse_preserves_single_newlines() {
        let input = "line1\nline2\n\nline3";
        let result = collapse_blank_lines(input);
        assert_eq!(result, "line1\nline2\n\nline3");
    }

    // ── Truncation ────────────────────────────────────────────────────

    #[test]
    fn test_truncation_no_truncation_needed() {
        let text = "Short text";
        let (result, truncated) = truncate_at_paragraph(text, 100);
        assert_eq!(result, "Short text");
        assert!(!truncated);
    }

    #[test]
    fn test_truncation_at_paragraph_boundary() {
        let text = "First paragraph.\n\nSecond paragraph that is much longer.\n\nThird paragraph.";
        let (result, truncated) = truncate_at_paragraph(text, 40);
        assert_eq!(result, "First paragraph.");
        assert!(truncated);
    }

    #[test]
    fn test_truncation_falls_back_when_no_boundary() {
        let text = "A".repeat(100);
        let (result, truncated) = truncate_at_paragraph(&text, 50);
        assert_eq!(result.len(), 50);
        assert!(truncated);
    }

    // ── Manifest ──────────────────────────────────────────────────────

    #[test]
    fn test_manifest_shape() {
        let tool = WebReadPageTool::new();
        let m = tool.manifest();
        assert_eq!(m.name, "web.read_page");
        assert_eq!(m.effects, vec![Effect::Network]);
        assert_eq!(m.risk, Risk::Caution);
        assert_eq!(m.default_approval, ApprovalPolicy::ConfirmOncePerSession);
        assert!(!m.privacy_aware);
        assert!(!m.requires_workspace);
    }
}
