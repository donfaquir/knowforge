use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB
const DEFAULT_MAX_CHARS: u32 = 4000;
const USER_AGENT: &str = "KnowForge/0.6.1";

// ─── SSRF protection ──────────────────────────────────────────────────────────

pub(crate) fn is_private_address(url: &url::Url) -> bool {
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

// ─── Text processing ─────────────────────────────────────────────────────────

pub(crate) fn collapse_blank_lines(text: &str) -> String {
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

pub(crate) fn truncate_at_paragraph(text: &str, max_chars: usize) -> (String, bool) {
    if text.len() <= max_chars {
        return (text.to_string(), false);
    }

    let boundary = floor_char_boundary(text, max_chars);
    let search_region = &text[..boundary];
    if let Some(pos) = search_region.rfind("\n\n") {
        if pos > boundary / 4 {
            return (text[..pos].to_string(), true);
        }
    }

    (search_region.to_string(), true)
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ─── SPA template detection ──────────────────────────────────────────────────

const TEMPLATE_MARKERS: &[&str] = &[
    "{{",
    "v-if=",
    "v-for=",
    "v-bind:",
    "*ngIf",
    "*ngFor",
    "ng-repeat=",
    "ng-if=",
];

fn looks_like_js_template(body: &str, extracted_text: &str) -> bool {
    if body.len() < 5_000 {
        return false;
    }
    // If the extracted output itself contains template markers, page is definitely unrendered
    if TEMPLATE_MARKERS.iter().any(|m| extracted_text.contains(m)) {
        return true;
    }
    // If extraction yielded very little text but body is large and has template syntax
    if extracted_text.trim().len() < 100 {
        return TEMPLATE_MARKERS.iter().any(|m| body.contains(m));
    }
    false
}

// ─── Tool implementation ──────────────────────────────────────────────────────

pub struct WebReadPageTool {
    manifest: ToolManifest,
    app: Option<tauri::AppHandle>,
}

impl WebReadPageTool {
    pub fn new(app: Option<tauri::AppHandle>) -> Self {
        Self {
            app,
            manifest: ToolManifest {
                name: "web.read_page".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description: "Fetch a specific URL and extract its article content as Markdown. Use this whenever the user provides a URL to read"
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
                default_approval: ApprovalPolicy::Auto,
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

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
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
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
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
                            message: format!(
                                "Request timed out after 10s — the page may be unreachable: {e}"
                            ),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NetworkDenied,
                        message: format!("Network error (will not retry): {e}"),
                        retryable: false,
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
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let bytes_in = body.len() as u64;
        let body_for_detection = body.clone();

        // ── Extract content + convert to markdown ─────────────────────
        // legible (Readability algorithm) extracts article content in one pass,
        // then htmd converts the clean HTML to markdown.
        let extract_url = url_str.clone();
        let convert_fut = tokio::task::spawn_blocking(move || {
            let (title, content_html) = match legible::parse(&body, Some(&extract_url), None) {
                Ok(article) => (article.title, article.content),
                Err(_) => (String::new(), body),
            };
            let md = htmd::convert(&content_html).unwrap_or_default();
            let md = collapse_blank_lines(&md);
            (title, md)
        });

        let (mut title, mut markdown) =
            match tokio::time::timeout(Duration::from_secs(5), convert_fut).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::Internal,
                            message: "Content extraction panicked".to_string(),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
                Err(_) => {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::Timeout,
                            message:
                                "Content extraction timed out (page too complex)".to_string(),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
            };

        // ── WebView fallback for JS-rendered pages ────────────────────
        let mut warnings = vec![];
        if let Some(app) = &self.app {
            if looks_like_js_template(&body_for_detection, &markdown) {
                let render_url = url_str.clone();
                match super::webview_renderer::render_page(
                    app,
                    &render_url,
                    Duration::from_secs(15),
                )
                .await
                {
                    Ok((rendered_html, rendered_title)) => {
                        let re_url = url_str.clone();
                        let re_result = tokio::task::spawn_blocking(move || {
                            let (t, c) =
                                match legible::parse(&rendered_html, Some(&re_url), None) {
                                    Ok(article) => (article.title, article.content),
                                    Err(_) => (rendered_title, rendered_html),
                                };
                            let md = htmd::convert(&c).unwrap_or_default();
                            (t, collapse_blank_lines(&md))
                        })
                        .await;
                        if let Ok((new_title, new_md)) = re_result {
                            if new_md.trim().len() > markdown.trim().len() {
                                title = new_title;
                                markdown = new_md;
                            }
                        }
                    }
                    Err(e) => {
                        warnings.push(crate::tools::types::ToolWarning {
                            code: "webview_fallback_failed".to_string(),
                            message: format!(
                                "JS rendering fallback failed: {e}; returning static content"
                            ),
                        });
                    }
                }
            }
        }

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
            warnings,
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

    // ── Content extraction tests ─────────────────────────────────────

    fn extract(html: &str) -> (String, String) {
        let article = legible::parse(html, None, None).unwrap();
        let md = htmd::convert(&article.content).unwrap_or_default();
        (article.title, collapse_blank_lines(&md))
    }

    #[test]
    fn test_extraction_strips_scripts_and_styles() {
        let html = r#"<html><head><style>body{color:red}</style></head>
            <body><script>alert('x')</script>
            <p>Hello world, this paragraph has enough text for the readability
               algorithm to consider it meaningful content worth extracting.</p>
            <p>A second paragraph provides additional signal that this is real
               article content and not just boilerplate navigation text.</p>
            </body></html>"#;
        let (_, md) = extract(html);
        assert!(!md.contains("alert"), "script content should be removed");
        assert!(!md.contains("color:red"), "style content should be removed");
        assert!(md.contains("Hello world"), "content should remain");
    }

    #[test]
    fn test_extraction_strips_nav_and_footer() {
        let html = r#"<html><body>
            <nav><a href="/">Home</a><a href="/about">About</a></nav>
            <article>
                <h1>Main Content Here</h1>
                <p>This is the main article body with enough text for the
                   readability algorithm to detect it as the primary content.</p>
                <p>Additional paragraphs help the scoring heuristic identify
                   this as the content area rather than boilerplate.</p>
            </article>
            <footer><p>Copyright 2024 Example Corp</p></footer>
            </body></html>"#;
        let (_, md) = extract(html);
        assert!(md.contains("Main Content"), "main content should remain");
        assert!(!md.contains("Copyright"), "footer should be excluded");
    }

    #[test]
    fn test_extraction_handles_large_head_styles() {
        let big_css = ".cls{width:100%;}".repeat(5000);
        let html = format!(
            r#"<html><head><style type="text/css">{big_css}</style></head>
            <body>
                <article>
                    <h1>Article With Big CSS</h1>
                    <p>This content must survive despite enormous CSS in head.
                       The readability algorithm should ignore all style data.</p>
                    <p>More content here to provide enough text for reliable
                       extraction by the scoring algorithm.</p>
                </article>
            </body></html>"#
        );
        let (_, md) = extract(&html);
        assert!(md.contains("content must survive"), "content should survive");
        assert!(!md.contains("width"), "CSS should not leak into output");
    }

    #[test]
    fn test_extraction_preserves_article_structure() {
        let html = r#"<html><head><title>Test Article</title></head><body>
            <article>
                <h1>Article Title</h1>
                <p>First paragraph of the article content with enough words
                   for the algorithm to consider it substantial.</p>
                <p>Second paragraph with <a href="https://example.com">a link</a>
                   and more explanatory text for extraction.</p>
                <h2>Subheading</h2>
                <p>Content under the subheading providing more detail.</p>
                <ul><li>Point one</li><li>Point two</li></ul>
            </article>
        </body></html>"#;
        let (title, md) = extract(html);
        assert_eq!(title, "Test Article");
        assert!(md.contains("First paragraph"), "body should remain");
        assert!(md.contains("example.com"), "links should survive");
    }

    #[test]
    fn test_title_extraction() {
        let html = r#"<html><head><title>  My Page Title  </title></head>
            <body><article>
                <p>Enough content text for the readability algorithm to work
                   properly and return a valid extraction result here.</p>
                <p>Second paragraph for scoring signal.</p>
            </article></body></html>"#;
        let (title, _) = extract(html);
        assert_eq!(title, "My Page Title");
    }

    #[test]
    fn test_title_missing_returns_empty() {
        let html = r#"<html><head></head><body><article>
                <h1>Heading As Title</h1>
                <p>Content text for the readability algorithm to extract.
                   This needs to be long enough to pass scoring.</p>
                <p>More content for reliable extraction.</p>
            </article></body></html>"#;
        let (title, md) = extract(html);
        assert!(title.is_empty() || title == "Heading As Title");
        assert!(md.contains("Content text"), "body should still be extracted");
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

    #[test]
    fn test_truncation_respects_cjk_char_boundary() {
        let text = "人工智能".repeat(100); // 1200 bytes, 400 chars
        let (result, truncated) = truncate_at_paragraph(&text, 10);
        assert!(truncated);
        assert!(result.len() <= 10);
        // Must not panic and must be valid UTF-8
        assert!(result.is_char_boundary(result.len()));
    }

    // ── Template detection ─────────────────────────────────────────

    #[test]
    fn test_template_detection_vue() {
        let body = format!(
            r#"<html><body><div id="app">{{{{article.title}}}}</div>{}</body></html>"#,
            "x".repeat(5000)
        );
        assert!(looks_like_js_template(&body, ""));
    }

    #[test]
    fn test_template_detection_angular() {
        let body = format!(
            r#"<html><body><div *ngIf="loaded">{}</div></body></html>"#,
            "x".repeat(5000)
        );
        assert!(looks_like_js_template(&body, ""));
    }

    #[test]
    fn test_template_detection_not_triggered_for_static() {
        let body = "<html><body><p>Normal static content</p></body></html>";
        assert!(!looks_like_js_template(body, "Normal static content"));
    }

    #[test]
    fn test_template_detection_not_triggered_when_clean_content_extracted() {
        let body = format!(
            r#"<html><body><p>Real content here</p>{}</body></html>"#,
            "x".repeat(5000)
        );
        let extracted = "a".repeat(150);
        assert!(!looks_like_js_template(&body, &extracted));
    }

    #[test]
    fn test_template_detection_triggered_when_output_contains_markers() {
        let body = format!(
            r#"<html><body>{{{{article.title}}}}{}</body></html>"#,
            "x".repeat(5000)
        );
        // Even with long extracted text, if it contains {{ it's unrendered
        let extracted = "Some title here {{article.zhaiyao_cn}} and more text over 100 chars padding padding padding padding";
        assert!(looks_like_js_template(&body, extracted));
    }

    #[test]
    fn test_template_detection_not_triggered_for_small_pages() {
        let body = "<html><body>{{data}}</body></html>";
        assert!(!looks_like_js_template(body, ""));
    }

    // ── Manifest ──────────────────────────────────────────────────────

    #[test]
    fn test_manifest_shape() {
        let tool = WebReadPageTool::new(None);
        let m = tool.manifest();
        assert_eq!(m.name, "web.read_page");
        assert_eq!(m.effects, vec![Effect::Network]);
        assert_eq!(m.risk, Risk::Caution);
        assert_eq!(m.default_approval, ApprovalPolicy::Auto);
        assert!(!m.privacy_aware);
        assert!(!m.requires_workspace);
    }
}
