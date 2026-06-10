use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Listener, WebviewUrl, WebviewWindowBuilder};

static WEBVIEW_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_label() -> String {
    let id = WEBVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("kf-render-{id}")
}

/// Render a URL in a hidden Tauri WebView and extract the rendered DOM content.
///
/// Returns `(rendered_html, title)` on success.
pub async fn render_page(
    app: &AppHandle,
    url: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let label = next_label();
    let event_name = format!("kf-render-result-{label}");

    let (tx, rx) = tokio::sync::oneshot::channel::<(String, String)>();
    let tx = std::sync::Mutex::new(Some(tx));

    let listener_id = app.listen(&event_name, move |event| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let payload = event.payload();
            let (html, title) = parse_render_payload(payload);
            let _ = tx.send((html, title));
        }
    });

    let parsed_url: url::Url = url
        .parse()
        .map_err(|e| format!("invalid URL for webview: {e}"))?;

    let extract_event_name = event_name.clone();
    let webview = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(parsed_url))
        .visible(false)
        .focused(false)
        .inner_size(1280.0, 800.0)
        .on_page_load(move |wv, payload| {
            if let tauri::webview::PageLoadEvent::Finished = payload.event() {
                let ev = extract_event_name.clone();
                let wv = wv.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let js = format!(
                        r#"(function(){{
                            var html = document.body ? document.body.innerHTML : '';
                            var title = document.title || '';
                            window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {{
                                event: '{ev}',
                                payload: JSON.stringify({{ html: html, title: title }})
                            }}).catch(function() {{}});
                        }})()"#,
                        ev = ev
                    );
                    let _ = wv.eval(&js);
                });
            }
        })
        .build()
        .map_err(|e| format!("webview creation failed: {e}"))?;

    let result = tokio::time::timeout(timeout, rx).await;

    app.unlisten(listener_id);
    let _ = webview.destroy();

    match result {
        Ok(Ok((html, title))) => {
            if html.is_empty() {
                Err("webview rendered empty content".to_string())
            } else {
                Ok((html, title))
            }
        }
        Ok(Err(_)) => Err("render channel closed unexpectedly".to_string()),
        Err(_) => Err("webview render timed out".to_string()),
    }
}

fn parse_render_payload(payload: &str) -> (String, String) {
    #[derive(serde::Deserialize)]
    struct RenderResult {
        html: String,
        #[serde(default)]
        title: String,
    }

    // The event payload from Tauri is a JSON string wrapping our JSON
    let inner: &str = match serde_json::from_str::<String>(payload) {
        Ok(ref s) => {
            // leaked reference hack avoided — just parse twice
            return match serde_json::from_str::<RenderResult>(s) {
                Ok(r) => (r.html, r.title),
                Err(_) => (String::new(), String::new()),
            };
        }
        Err(_) => payload,
    };

    match serde_json::from_str::<RenderResult>(inner) {
        Ok(r) => (r.html, r.title),
        Err(_) => (String::new(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_render_payload_direct_json() {
        let payload = r#"{"html":"<p>Hello</p>","title":"Test"}"#;
        let (html, title) = parse_render_payload(payload);
        assert_eq!(html, "<p>Hello</p>");
        assert_eq!(title, "Test");
    }

    #[test]
    fn test_parse_render_payload_wrapped_string() {
        let inner = r#"{"html":"<div>Content</div>","title":"Page"}"#;
        let payload = serde_json::to_string(inner).unwrap();
        let (html, title) = parse_render_payload(&payload);
        assert_eq!(html, "<div>Content</div>");
        assert_eq!(title, "Page");
    }

    #[test]
    fn test_parse_render_payload_invalid() {
        let (html, title) = parse_render_payload("not json at all");
        assert!(html.is_empty());
        assert!(title.is_empty());
    }

    #[test]
    fn test_label_uniqueness() {
        let a = next_label();
        let b = next_label();
        assert_ne!(a, b);
    }
}
