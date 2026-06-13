use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::web_ops::{collapse_blank_lines, is_private_address, truncate_at_paragraph};
use crate::tools::context::ToolContext;
use crate::tools::types::{
    ApprovalPolicy, Effect, Risk, Tool, ToolCategory, ToolError, ToolErrorCode, ToolManifest,
    ToolMetrics, ToolResult,
};

const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024; // 50 MB
const MAX_PDF_BYTES: u64 = 20 * 1024 * 1024; // 20 MB
const DEFAULT_PDF_MAX_CHARS: u32 = 8000;
const USER_AGENT: &str = "KnowForge/0.6.1";

fn infer_filename(url: &url::Url, content_disposition: Option<&str>, content_type: Option<&str>) -> String {
    if let Some(cd) = content_disposition {
        if let Some(pos) = cd.find("filename=") {
            let name = &cd[pos + 9..];
            let name = name.trim_matches('"').trim_matches('\'');
            if !name.is_empty() {
                return sanitize_filename(name);
            }
        }
    }
    let path = url.path();
    let segment = path.rsplit('/').next().unwrap_or("download");
    let name = if segment.is_empty() { "download".to_string() } else { sanitize_filename(segment) };
    if name.contains('.') {
        return name;
    }
    let ext = match content_type {
        Some(ct) if ct.contains("application/pdf") => ".pdf",
        Some(ct) if ct.contains("image/png") => ".png",
        Some(ct) if ct.contains("image/jpeg") => ".jpg",
        Some(ct) if ct.contains("image/gif") => ".gif",
        Some(ct) if ct.contains("image/webp") => ".webp",
        Some(ct) if ct.contains("text/html") => ".html",
        Some(ct) if ct.contains("application/zip") => ".zip",
        _ => "",
    };
    format!("{name}{ext}")
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

// ─── web.download ─────────────────────────────────────────────────────────────

pub struct WebDownloadTool {
    manifest: ToolManifest,
}

impl WebDownloadTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "web.download".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description:
                    "Download a file from a URL to the local workspace downloads/ folder. Use for saving PDF, images, or other files locally"
                        .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL of the file to download"
                        },
                        "filename": {
                            "type": "string",
                            "description": "Optional filename to save as (auto-detected from URL if omitted)"
                        }
                    },
                    "required": ["url"],
                    "additionalProperties": false
                }),
                output_schema: json!({}),
                effects: vec![Effect::Network, Effect::Write],
                risk: Risk::Caution,
                privacy_aware: true,
                requires_workspace: true,
                default_approval: ApprovalPolicy::ConfirmOncePerSession,
                examples: vec![],
                tags: vec!["web".to_string(), "download".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for WebDownloadTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = Instant::now();

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

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap();

        let response = match client.get(parsed_url.as_str()).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::NetworkDenied,
                        message: format!("Download failed: {e}"),
                        retryable: e.is_timeout(),
                        cause: None,
                    },
                };
            }
        };

        if !response.status().is_success() {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("HTTP {}", response.status()),
                    retryable: false,
                    cause: None,
                },
            };
        }

        if let Some(len) = response.content_length() {
            if len > MAX_DOWNLOAD_BYTES {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::BudgetExceeded,
                        message: format!("File too large ({len} bytes, max {MAX_DOWNLOAD_BYTES})"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let content_disposition = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let filename = input
            .get("filename")
            .and_then(|v| v.as_str())
            .map(|s| sanitize_filename(s))
            .unwrap_or_else(|| infer_filename(&parsed_url, content_disposition.as_deref(), Some(&content_type)));

        let bytes = match response.bytes().await {
            Ok(b) => {
                if b.len() as u64 > MAX_DOWNLOAD_BYTES {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::BudgetExceeded,
                            message: format!(
                                "File too large ({} bytes, max {MAX_DOWNLOAD_BYTES})",
                                b.len()
                            ),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
                b
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: format!("Failed to read response: {e}"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let download_dir = ctx.workspace_root.join("downloads");
        if let Err(e) = std::fs::create_dir_all(&download_dir) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("Failed to create downloads directory: {e}"),
                    retryable: false,
                    cause: None,
                },
            };
        }

        let file_path = download_dir.join(&filename);
        if let Err(e) = std::fs::write(&file_path, &bytes) {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::Internal,
                    message: format!("Failed to write file: {e}"),
                    retryable: false,
                    cause: None,
                },
            };
        }

        let rel_path = format!("downloads/{filename}");
        let size = bytes.len() as u64;
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: json!({
                "path": rel_path,
                "size": size,
                "content_type": content_type,
            }),
            redacted_count: 0,
            warnings: vec![],
            metrics: ToolMetrics {
                duration_ms,
                bytes_in: size,
                bytes_out: 0,
                network_bytes: size,
                ..Default::default()
            },
        }
    }
}

// ─── web.read_pdf ─────────────────────────────────────────────────────────────

pub struct WebReadPdfTool {
    manifest: ToolManifest,
}

impl WebReadPdfTool {
    pub fn new() -> Self {
        Self {
            manifest: ToolManifest {
                name: "web.read_pdf".to_string(),
                version: "1.0.0".to_string(),
                protocol_version: "1.0".to_string(),
                description:
                    "Download a PDF from a URL and extract its text content. Use for reading academic papers, reports, or any PDF document"
                        .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL of the PDF file to read"
                        },
                        "max_chars": {
                            "type": "integer",
                            "description": "Maximum characters to return from extracted text",
                            "default": DEFAULT_PDF_MAX_CHARS,
                            "minimum": 500,
                            "maximum": 50000
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
                tags: vec!["web".to_string(), "pdf".to_string(), "read".to_string()],
                deprecated: None,
            },
        }
    }
}

#[async_trait]
impl Tool for WebReadPdfTool {
    fn manifest(&self) -> &ToolManifest {
        &self.manifest
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Web
    }

    async fn invoke(&self, ctx: &ToolContext, input: Value) -> ToolResult {
        let start = Instant::now();

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
            .map(|v| v.clamp(500, 50000) as usize)
            .unwrap_or(DEFAULT_PDF_MAX_CHARS as usize);

        let resource_dir = ctx.app_bundle_resource_dir.clone();

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

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap();

        let response = match client.get(parsed_url.as_str()).send().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: if e.is_timeout() {
                            ToolErrorCode::Timeout
                        } else {
                            ToolErrorCode::NetworkDenied
                        },
                        message: format!("PDF download failed: {e}"),
                        retryable: e.is_timeout(),
                        cause: None,
                    },
                };
            }
        };

        if !response.status().is_success() {
            return ToolResult::Err {
                error: ToolError {
                    code: ToolErrorCode::NotFound,
                    message: format!("HTTP {}", response.status()),
                    retryable: false,
                    cause: None,
                },
            };
        }

        if let Some(len) = response.content_length() {
            if len > MAX_PDF_BYTES {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::BudgetExceeded,
                        message: format!("PDF too large ({len} bytes, max {MAX_PDF_BYTES})"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        }

        let bytes = match response.bytes().await {
            Ok(b) => {
                if b.len() as u64 > MAX_PDF_BYTES {
                    return ToolResult::Err {
                        error: ToolError {
                            code: ToolErrorCode::BudgetExceeded,
                            message: format!(
                                "PDF too large ({} bytes, max {MAX_PDF_BYTES})",
                                b.len()
                            ),
                            retryable: false,
                            cause: None,
                        },
                    };
                }
                b
            }
            Err(e) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: format!("Failed to read PDF response: {e}"),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let bytes_in = bytes.len() as u64;
        let pdf_bytes = bytes.to_vec();
        let warnings = vec![];

        // Try pdfium (bundled at resources/libpdfium.dylib), fallback to pdf-extract.
        // app_bundle_resource_dir points to the model subdirectory
        // (resources/models/bge-small-zh-v1.5); go up to resources/.
        let pdfium_lib = resource_dir
            .as_ref()
            .and_then(|dir| dir.parent()?.parent())
            .and_then(|res_root| super::pdfium_manager::find_library(res_root));

        let extract_fut = tokio::task::spawn_blocking(move || {
            if let Some(lib_path) = pdfium_lib {
                match super::pdfium_manager::extract_text_with_pdfium(&lib_path, &pdf_bytes) {
                    Ok(result) => return result,
                    Err(_) => {}
                }
            }
            extract_pdf_text_fallback(&pdf_bytes)
        });

        let (text, pages) = match tokio::time::timeout(Duration::from_secs(30), extract_fut).await
        {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Internal,
                        message: "PDF text extraction task failed unexpectedly".to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
            Err(_) => {
                return ToolResult::Err {
                    error: ToolError {
                        code: ToolErrorCode::Timeout,
                        message: "PDF text extraction timed out (file may be too large or complex)".to_string(),
                        retryable: false,
                        cause: None,
                    },
                };
            }
        };

        let text = collapse_blank_lines(&text);
        let content_length = text.len();
        let (content, truncated) = truncate_at_paragraph(&text, max_chars);

        let bytes_out = content.len() as u64;
        let duration_ms = start.elapsed().as_millis() as u64;

        ToolResult::Ok {
            data: json!({
                "url": url_str,
                "content": content,
                "content_length": content_length,
                "truncated": truncated,
                "pages": pages,
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

fn extract_pdf_text_fallback(pdf_bytes: &[u8]) -> (String, u32) {
    let result = std::panic::catch_unwind(|| {
        pdf_extract::extract_text_from_mem(pdf_bytes)
    });
    match result {
        Ok(Ok(text)) => {
            let pages = pdf_bytes
                .windows(7)
                .filter(|w| w == b"/Type /")
                .count()
                .max(1) as u32;
            (text, pages)
        }
        Ok(Err(e)) => (format!("[PDF extraction error: {e}]"), 0),
        Err(_) => (String::from("[PDF extraction failed: unsupported PDF format]"), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_filename_from_url() {
        let url = url::Url::parse("https://example.com/papers/article.pdf").unwrap();
        assert_eq!(infer_filename(&url, None, None), "article.pdf");
    }

    #[test]
    fn test_infer_filename_from_content_disposition() {
        let url = url::Url::parse("https://example.com/download?id=123").unwrap();
        let cd = "attachment; filename=\"my paper.pdf\"";
        assert_eq!(infer_filename(&url, Some(cd), None), "my paper.pdf");
    }

    #[test]
    fn test_infer_filename_empty_path() {
        let url = url::Url::parse("https://example.com/").unwrap();
        assert_eq!(infer_filename(&url, None, None), "download");
    }

    #[test]
    fn test_infer_filename_adds_pdf_ext_from_content_type() {
        let url = url::Url::parse("https://example.com/download/7").unwrap();
        assert_eq!(infer_filename(&url, None, Some("application/pdf")), "7.pdf");
    }

    #[test]
    fn test_infer_filename_no_ext_added_when_already_present() {
        let url = url::Url::parse("https://example.com/report.pdf").unwrap();
        assert_eq!(infer_filename(&url, None, Some("application/pdf")), "report.pdf");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("a/b\\c:d.pdf"), "a_b_c_d.pdf");
        assert_eq!(sanitize_filename("normal.pdf"), "normal.pdf");
    }

    #[test]
    fn test_manifest_download() {
        let tool = WebDownloadTool::new();
        let m = tool.manifest();
        assert_eq!(m.name, "web.download");
        assert!(m.effects.contains(&Effect::Network));
        assert!(m.effects.contains(&Effect::Write));
        assert!(m.requires_workspace);
    }

    #[test]
    fn test_manifest_read_pdf() {
        let tool = WebReadPdfTool::new();
        let m = tool.manifest();
        assert_eq!(m.name, "web.read_pdf");
        assert!(m.effects.contains(&Effect::Network));
        assert!(!m.requires_workspace);
    }
}
