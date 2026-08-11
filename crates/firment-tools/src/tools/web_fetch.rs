use super::html::strip_html;
use super::util::truncate;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::time::Duration;

pub struct WebFetch;

/// Max bytes read from the response body.
const MAX_BODY_BYTES: usize = 200_000;
/// Max chars returned to the model.
const MAX_TEXT_CHARS: usize = 60_000;

impl WebFetch {
    fn validate_url(url: &str) -> Result<(), ToolError> {
        let lower = url.to_ascii_lowercase();
        let (scheme, host) = match lower.split_once("://") {
            Some((scheme, rest)) => (scheme, rest),
            None => {
                return Err(ToolError::new(
                    "[InvalidInput] url must be http:// or https://",
                ));
            }
        };
        if scheme != "http" && scheme != "https" {
            return Err(ToolError::new(format!(
                "[InvalidInput] unsupported scheme '{scheme}://' (only http/https)"
            )));
        }
        let host = host.split(['/', '?', '#']).next().unwrap_or(host);
        if host.is_empty() {
            return Err(ToolError::new("[InvalidInput] url has no host"));
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch a URL (http/https) and return its readable text. HTML pages are converted to plain text. Use for datasheets, errata, vendor documentation, GitHub pages, or any web resource the user points to. Content is truncated; very large pages may be cut off."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "The URL to fetch, e.g. https://docs.example.com/datasheet"}
            },
            "required": ["url"]
        })
    }

    async fn run(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let url = args
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing url"))?;
        Self::validate_url(url)?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("Firment/0.4 (firmware coding agent)")
            .build()
            .map_err(|e| ToolError::new(format!("[Io] http client init failed: {e}")))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|e| ToolError::new(format!("[Net] request failed: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ToolError::new(format!("[Net] HTTP {status} from {url}")));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();

        let mut stream = response.bytes_stream();
        let mut body: Vec<u8> = Vec::new();
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ToolError::new(format!("[Net] read failed: {e}")))?;
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                break;
            }
            body.extend_from_slice(&chunk);
        }
        let raw = String::from_utf8_lossy(&body).into_owned();
        let text = if content_type.contains("text/html") || raw.contains("<html") {
            strip_html(&raw)
        } else {
            raw
        };
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(ToolOutput {
                text: format!("{url}: empty response body"),
            });
        }
        Ok(ToolOutput {
            text: truncate(&text, MAX_TEXT_CHARS),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AutoApprove, EditJournal};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
            permission: Arc::new(AutoApprove::everything()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(dir.join("undo")))),
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            subagent: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            allowed_roots: Vec::new(),
            cancel: firment_core::Cancellable::new(),
        }
    }

    #[tokio::test]
    async fn rejects_non_http_schemes() {
        let dir = tempdir().unwrap();
        for url in [
            "file:///etc/passwd",
            "ftp://x",
            "javascript:alert(1)",
            "not a url",
        ] {
            let err = WebFetch
                .run(json!({"url": url}), &ctx(dir.path()))
                .await
                .unwrap_err();
            assert!(err.message.contains("[InvalidInput]"), "got: {url} {err}");
        }
    }

    #[tokio::test]
    async fn fetches_html_as_plain_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><body><h1>STM32 errata</h1><p>Some <b>important</b> note &amp; details.</p></body></html>",
                "text/html",
            ))
            .mount(&server)
            .await;
        let dir = tempdir().unwrap();
        let url = format!("{}/page", server.uri());
        let out = WebFetch
            .run(json!({"url": url}), &ctx(dir.path()))
            .await
            .unwrap();
        assert!(out.text.contains("STM32 errata"), "got: {}", out.text);
        assert!(
            out.text.contains("important note & details"),
            "got: {}",
            out.text
        );
        assert!(!out.text.contains("<h1>"), "got: {}", out.text);
    }

    #[tokio::test]
    async fn http_error_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let dir = tempdir().unwrap();
        let url = format!("{}/missing", server.uri());
        let err = WebFetch
            .run(json!({"url": url}), &ctx(dir.path()))
            .await
            .unwrap_err();
        assert!(err.message.contains("[Net]"), "got: {err}");
    }
}
