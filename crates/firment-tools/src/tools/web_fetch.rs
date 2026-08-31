use super::html::strip_html;
use super::util::truncate;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::net::IpAddr;
use std::time::Duration;

pub struct WebFetch;

/// Max bytes read from the response body.
const MAX_BODY_BYTES: usize = 200_000;
/// Max chars returned to the model.
const MAX_TEXT_CHARS: usize = 60_000;

impl WebFetch {
    /// Extract the bare host from a URL string: strips scheme, userinfo,
    /// port, path/query/fragment, and IPv6 brackets.
    fn parse_host(url: &str) -> Option<String> {
        let after_scheme = url.split_once("://")?.1;
        let host_port = after_scheme.split(['/', '?', '#']).next()?;
        let host = host_port.rsplit('@').next().unwrap_or(host_port);
        if let Some(rest) = host.strip_prefix('[') {
            return rest.split(']').next().map(|s| s.to_string());
        }
        Some(host.split(':').next().unwrap_or(host).to_string())
    }

    /// True when `host` is a literal IP in an internal / link-local range
    /// that an agent must not fetch on its own: private RFC1918, CGNAT,
    /// link-local / cloud metadata (169.254/16), 0.0.0.0/8. Loopback is
    /// intentionally allowed (local dev services; tests use 127.0.0.1
    /// mocks). Hostnames are not DNS-resolved here (DNS rebinding is out of
    /// scope; documented in the audit report).
    fn is_blocked_literal_ip(host: &str) -> bool {
        let Ok(ip) = host.parse::<IpAddr>() else {
            return false;
        };
        match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                o[0] == 0
                    || o[0] == 10
                    || (o[0] == 100 && (64..=127).contains(&o[1]))
                    || (o[0] == 169 && o[1] == 254)
                    || (o[0] == 172 && (16..=31).contains(&o[1]))
                    || (o[0] == 192 && o[1] == 168)
                    || v4.is_broadcast()
            }
            IpAddr::V6(v6) => {
                let seg0 = v6.segments()[0];
                v6.is_unspecified()
                    || (seg0 & 0xffc0) == 0xfe80  // fe80::/10 link-local
                    || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique-local
            }
        }
    }

    /// Host of `url` if it is a blocked literal internal IP.
    fn blocked_host(url: &str) -> Option<String> {
        let host = Self::parse_host(url)?;
        if Self::is_blocked_literal_ip(&host) {
            Some(host)
        } else {
            None
        }
    }

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
        if let Some(blocked) = Self::blocked_host(url) {
            return Err(ToolError::new(format!(
                "[InvalidInput] url host '{blocked}' is an internal/link-local address; \
                 web_fetch refuses private, metadata, and link-local endpoints"
            )));
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

        let client = firment_core::http_builder()
            .timeout(Duration::from_secs(20))
            .user_agent("Firment/0.4 (firmware coding agent)")
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let blocked = attempt
                    .url()
                    .host_str()
                    .map(|h| h.trim_start_matches('[').trim_end_matches(']').to_string())
                    .filter(|h| Self::is_blocked_literal_ip(h));
                match blocked {
                    Some(host) => attempt.error(std::io::Error::other(format!(
                        "redirect to internal address '{host}' is blocked"
                    ))),
                    None => attempt.follow(),
                }
            }))
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
        let mut truncated = false;
        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ToolError::new(format!("[Net] read failed: {e}")))?;
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                truncated = true;
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
        let mut text = truncate(&text, MAX_TEXT_CHARS);
        if truncated {
            // Say so explicitly: silently returning a partial page makes the
            // model treat truncated content as the complete document.
            text = format!(
                "[truncated: page exceeds {MAX_BODY_BYTES} bytes; content below is the first part]\n{text}"
            );
        }
        Ok(ToolOutput { text })
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
            device_log_dir: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            ledger_path: None,
            providers: Vec::new(),
            la: None,
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

    #[test]
    fn rejects_internal_and_link_local_addresses() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://192.168.1.1/admin",
            "http://172.16.0.1/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://user:pass@10.0.0.1/x",
        ] {
            let err = WebFetch::validate_url(url).unwrap_err();
            assert!(
                err.message.contains("internal/link-local"),
                "should reject: {url} got: {err}"
            );
        }
    }

    #[test]
    fn allows_loopback_and_public_addresses() {
        for url in [
            "http://127.0.0.1:8080/x",
            "http://localhost:3000/",
            "http://[::1]:8080/",
            "https://example.com/",
            "https://docs.rs/foo/bar",
            "http://user:pass@example.com/x",
            "https://8.8.8.8/",
        ] {
            assert!(WebFetch::validate_url(url).is_ok(), "should allow: {url}");
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
