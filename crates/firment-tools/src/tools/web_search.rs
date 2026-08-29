use super::html::strip_html;
use super::util::truncate;
use async_trait::async_trait;
use firment_core::{Tool, ToolContext, ToolError, ToolOutput};
use serde_json::{Value, json};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

pub struct WebSearch;

#[derive(Debug, Clone)]
struct ResultItem {
    title: String,
    url: String,
    snippet: String,
}

/// Format search results for the model.
fn format_results(query: &str, items: &[ResultItem], max_results: usize) -> String {
    let mut out = format!("web search results for \"{query}\" ({}):", items.len());
    for (i, item) in items.iter().take(max_results).enumerate() {
        out.push_str(&format!(
            "\n{}. {}\n   {}\n   {}",
            i + 1,
            item.title,
            item.url,
            truncate(&item.snippet, 300)
        ));
    }
    if items.is_empty() {
        out.push_str("\n(no results)");
    }
    out
}

/// Browser-like user agent and headers: DuckDuckGo flags custom UAs (e.g.
/// `... Firment/0.4`) with an HTTP 202 anomaly page, so we present as a normal
/// browser and detect the challenge page when it still appears.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Minimum gap between DuckDuckGo requests. The free endpoint rate-limits
/// aggressively (first request succeeds, then a burst of follow-ups all get
/// served a challenge/empty page), so we pace requests process-wide.
const DDG_MIN_INTERVAL: Duration = Duration::from_secs(3);
/// Backoff before retrying a blocked/empty DuckDuckGo response.
const DDG_RETRY_DELAY: Duration = Duration::from_secs(4);

/// One shared client (browser UA, headers, in-memory cookie jar) reused across
/// calls so DDG sees a single browsing session instead of a fresh bot client
/// per request.
static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

static DDG_THROTTLE: StdMutex<Option<Instant>> = StdMutex::new(None);

fn http_client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(BROWSER_UA)
            .cookie_store(true)
            .default_headers(
                std::iter::once((
                    reqwest::header::ACCEPT,
                    reqwest::header::HeaderValue::from_static(
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                    ),
                ))
                .chain(std::iter::once((
                    reqwest::header::ACCEPT_LANGUAGE,
                    reqwest::header::HeaderValue::from_static("en-US,en;q=0.9"),
                )))
                .collect(),
            )
            .build()
            .expect("reqwest client build cannot fail")
    })
}

/// Sleep until `DDG_MIN_INTERVAL` has passed since the last DuckDuckGo request.
/// Only meaningful for real DDG hosts; mock servers in tests stay fast.
async fn ddg_throttle() {
    let wait = {
        let mut guard = DDG_THROTTLE.lock().unwrap();
        let since = guard.map(|last| last.elapsed()).unwrap_or_default();
        let wait = DDG_MIN_INTERVAL.saturating_sub(since);
        *guard = Some(Instant::now());
        wait
    };
    if wait > Duration::ZERO {
        tokio::time::sleep(wait).await;
    }
}

fn is_real_duckduckgo(url: &str) -> bool {
    url.contains("duckduckgo.com")
}

/// DuckDuckGo serves a challenge/anomaly page instead of results when it thinks
/// the request is a bot or the IP is being throttled; detect that so we can
/// error instead of returning a misleading empty result set.
fn looks_blocked(body: &str) -> bool {
    let probe = body.to_ascii_lowercase();
    probe.contains("anomaly") || probe.contains("captcha") || probe.contains("challenge")
}

/// A real zero-result page on DDG says so explicitly; a page with no result
/// markers *and* no such message is a throttled/empty challenge page, not a
/// genuine "no results".
fn looks_like_no_results(body: &str) -> bool {
    let probe = body.to_ascii_lowercase();
    probe.contains("no results")
        || probe.contains("no more results")
        || probe.contains("did not match")
}

/// DuckDuckGo HTML endpoint (no API key). Results are parsed from the raw
/// HTML: each entry has a `result__a` link (href carries the real URL in the
/// `uddg=` parameter) and a `result__snippet` text.
async fn duckduckgo_html(
    base_url: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<ResultItem>, ToolError> {
    let client = http_client();
    let url = format!("{base_url}?q={}&kl=us-en", url_encode(query));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ToolError::new(format!("[Net] duckduckgo request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ToolError::new(format!(
            "[Net] duckduckgo returned HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ToolError::new(format!("[Net] read failed: {e}")))?;
    if looks_blocked(&body) {
        return Err(ToolError::new(
            "[Blocked] duckduckgo served its anti-bot challenge page (no results in the \
             response). The search provider may be rate-limited or blocked from this \
             network/region. Fall back to web_fetch on a known URL (vendor page, datasheet, \
             RSS feed) instead.",
        ));
    }
    let mut items = Vec::new();
    let mut rest = body.as_str();
    while items.len() < max_results {
        let Some(anchor) = find_after(rest, "result__a") else {
            break;
        };
        let Some(href) = find_between(anchor, "href=\"", "\"") else {
            rest = anchor;
            continue;
        };
        let url = uddg_url(href).unwrap_or(href.to_string());
        let Some(title_pos) = anchor.find('>') else {
            rest = anchor;
            continue;
        };
        let title_body = &anchor[title_pos + 1..];
        let Some(title_end_pos) = title_body.find("</a>") else {
            rest = anchor;
            continue;
        };
        let title = strip_html(&title_body[..title_end_pos]).trim().to_string();
        let snippet_rest = &title_body[title_end_pos + 4..];
        let snippet = match find_after(snippet_rest, "result__snippet") {
            Some(snippet_start) => find_between(snippet_start, ">", "</a>")
                .map(|s| strip_html(s).trim().to_string())
                .unwrap_or_default(),
            None => String::new(),
        };
        items.push(ResultItem {
            title,
            url,
            snippet,
        });
        rest = anchor;
    }
    if items.is_empty() && !looks_like_no_results(&body) {
        return Err(ToolError::new(
            "[Blocked] duckduckgo returned a page without any results or a \"no results\" \
             message — typical of rate limiting after a burst of requests. Space out searches; \
             or fall back to web_fetch on a known URL (vendor page, datasheet, RSS feed).",
        ));
    }
    Ok(items)
}

/// DuckDuckGo Lite endpoint (no API key): a minimal table layout that is less
/// aggressively challenged than the HTML endpoint. Used as a fallback when the
/// HTML endpoint is blocked. Hrefs are direct (no `uddg=` redirect).
async fn duckduckgo_lite(
    base_url: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<ResultItem>, ToolError> {
    let client = http_client();
    let url = format!("{base_url}?q={}", url_encode(query));
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ToolError::new(format!("[Net] duckduckgo lite request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ToolError::new(format!(
            "[Net] duckduckgo lite returned HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ToolError::new(format!("[Net] read failed: {e}")))?;
    if looks_blocked(&body) {
        return Err(ToolError::new(
            "[Blocked] duckduckgo lite served its anti-bot challenge page. The search provider \
             may be rate-limited or blocked from this network/region. Fall back to web_fetch on \
             a known URL (vendor page, datasheet, RSS feed) instead.",
        ));
    }
    let mut items = Vec::new();
    let mut rest = body.as_str();
    while items.len() < max_results {
        let Some(link) = find_after(rest, "result-link") else {
            break;
        };
        let Some(href) = find_between(link, "href=\"", "\"") else {
            rest = link;
            continue;
        };
        let Some(title_pos) = link.find('>') else {
            rest = link;
            continue;
        };
        let title_body = &link[title_pos + 1..];
        let Some(title_end_pos) = title_body.find("</a>") else {
            rest = link;
            continue;
        };
        let title = strip_html(&title_body[..title_end_pos]).trim().to_string();
        let row_tail = &title_body[title_end_pos + 4..];
        let snippet = match find_after(row_tail, "result-snippet") {
            Some(snippet_start) => find_between(snippet_start, ">", "</td>")
                .map(|s| strip_html(s).trim().to_string())
                .unwrap_or_default(),
            None => String::new(),
        };
        items.push(ResultItem {
            title,
            url: href.to_string(),
            snippet,
        });
        rest = link;
    }
    Ok(items)
}

/// DuckDuckGo with throttling, one retry, and a Lite fallback: pace requests
/// (the free endpoint rate-limits after a burst), retry once with a backoff
/// when the HTML endpoint is blocked, then try the Lite endpoint before giving
/// up with a combined error.
/// Bing (cn.bing.com) — no API key, reachable from mainland China where
/// DuckDuckGo is unreliable. Results are plain `<li class="b_algo">` blocks
/// with direct hrefs (no redirect wrapper), so parsing is simple. The
/// regional market is zh-CN; swap the base_url to www.bing.com for the
/// international index.
async fn bing_html(
    base_url: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<ResultItem>, ToolError> {
    let client = http_client();
    let url = format!(
        "{base_url}?q={}&count={}&mkt=zh-CN",
        url_encode(query),
        max_results
    );
    let response = client
        .get(&url)
        .header(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
        )
        .send()
        .await
        .map_err(|e| ToolError::new(format!("[Net] bing request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ToolError::new(format!(
            "[Net] bing returned HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ToolError::new(format!("[Net] read failed: {e}")))?;
    let mut items = Vec::new();
    let mut rest = body.as_str();
    while items.len() < max_results {
        let Some(block) = find_after(rest, "b_algo") else {
            break;
        };
        // Title anchor is the first <h2> in the block; the first href
        // inside it is the result link (attributes vary, so match loosely).
        let Some(h2) = find_after(block, "<h2") else {
            rest = block;
            continue;
        };
        let Some(href) = find_between(h2, "href=\"", "\"") else {
            rest = block;
            continue;
        };
        // Bing result links are direct hrefs (no redirect wrapper for
        // organic results); /ck/a wrappers (ads) are rare and left as-is.
        let url = href.to_string();
        let Some(title_open) = h2.find('>') else {
            rest = block;
            continue;
        };
        let title_body = &h2[title_open + 1..];
        let Some(title_end) = title_body.find("</a>") else {
            rest = block;
            continue;
        };
        let title = strip_html(&title_body[..title_end]).trim().to_string();
        if title.is_empty() {
            rest = block;
            continue;
        }
        // Snippet: the first <p> inside the block.
        let snippet = match find_after(block, "<p") {
            Some(p_start) => match p_start.find('>') {
                Some(p_open) => {
                    let p_body = &p_start[p_open + 1..];
                    p_body
                        .split("</p>")
                        .next()
                        .map(|s| strip_html(s).trim().to_string())
                        .unwrap_or_default()
                }
                None => String::new(),
            },
            None => String::new(),
        };
        items.push(ResultItem {
            title,
            url,
            snippet,
        });
        rest = block;
    }
    if items.is_empty() {
        if looks_blocked(&body) {
            return Err(ToolError::new(
                "[Blocked] bing served its anti-bot challenge page (no results in the \
                 response). The search provider may be rate-limited or blocked from \
                 this network/region. Fall back to web_fetch on a known URL (vendor \
                 page, datasheet, RSS feed) instead.",
            ));
        }
        if !looks_like_no_results(&body) {
            return Err(ToolError::new(
                "[Blocked] bing returned a page without any results or a \"no results\" \
                 message — typical of rate limiting after a burst of requests or a CN \
                 network hiccup. Retry the query or fall back to web_fetch on a known \
                 URL (vendor page, datasheet, RSS feed).",
            ));
        }
    }
    Ok(items)
}

async fn duckduckgo(
    html_url: &str,
    lite_url: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<ResultItem>, ToolError> {
    let real = is_real_duckduckgo(html_url);
    let mut html_err = None;
    for attempt in 0..2 {
        if real {
            ddg_throttle().await;
        }
        match duckduckgo_html(html_url, query, max_results).await {
            Ok(items) => return Ok(items),
            Err(e) => {
                html_err = Some(e);
                if real && attempt == 0 {
                    tokio::time::sleep(DDG_RETRY_DELAY).await;
                }
            }
        }
    }
    if real {
        ddg_throttle().await;
    }
    match duckduckgo_lite(lite_url, query, max_results).await {
        Ok(items) => Ok(items),
        Err(lite_err) => Err(ToolError::new(format!(
            "[Net] duckduckgo unavailable: {}\n{}",
            html_err
                .map(|e| e.message)
                .unwrap_or_else(|| "html endpoint failed".to_string()),
            lite_err.message
        ))),
    }
}

/// Find `needle` and return the slice starting right after it.
fn find_after<'a>(haystack: &'a str, needle: &str) -> Option<&'a str> {
    let idx = haystack.find(needle)?;
    Some(&haystack[idx + needle.len()..])
}

/// Find the text between `start` and `end` markers.
fn find_between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let after = find_after(haystack, start)?;
    let idx = after.find(end)?;
    Some(&after[..idx])
}

/// Extract the real URL from a DuckDuckGo redirect (`uddg=` param).
fn uddg_url(href: &str) -> Option<String> {
    let after = find_after(href, "uddg=")?;
    let encoded = after.split('&').next()?;
    Some(url_decode(encoded))
}

fn url_decode(encoded: &str) -> String {
    let mut out = Vec::new();
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                let hex: String = [bytes.next().unwrap_or(b'0'), bytes.next().unwrap_or(b'0')]
                    .iter()
                    .map(|b| *b as char)
                    .collect();
                if let Ok(code) = u8::from_str_radix(&hex, 16) {
                    out.push(code);
                }
            }
            b'+' => out.push(b' '),
            other => out.push(other),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Tavily search API (requires an API key).
async fn tavily(
    base_url: &str,
    query: &str,
    max_results: usize,
    api_key: &str,
) -> Result<Vec<ResultItem>, ToolError> {
    if api_key.is_empty() {
        return Err(ToolError::new(
            "[Net] tavily requires an API key: set [tools] web_search_api_key_env (e.g. \
             TAVILY_API_KEY) or web_search_api_key in config.toml",
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| ToolError::new(format!("[Io] http client init failed: {e}")))?;
    let response = client
        .post(base_url)
        .json(&json!({
            "api_key": api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": "basic"
        }))
        .send()
        .await
        .map_err(|e| ToolError::new(format!("[Net] tavily request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ToolError::new(format!(
            "[Net] tavily returned HTTP {}",
            response.status()
        )));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|e| ToolError::new(format!("[Net] invalid tavily response: {e}")))?;
    let mut items = Vec::new();
    if let Some(results) = payload.get("results").and_then(|r| r.as_array()) {
        for result in results {
            items.push(ResultItem {
                title: result
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: result
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                snippet: result
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(items)
}

/// Brave Search API (requires an API key).
async fn brave(
    base_url: &str,
    query: &str,
    max_results: usize,
    api_key: &str,
) -> Result<Vec<ResultItem>, ToolError> {
    if api_key.is_empty() {
        return Err(ToolError::new(
            "[Net] brave requires an API key: set [tools] web_search_api_key_env (e.g. \
             BRAVE_API_KEY) or web_search_api_key in config.toml",
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| ToolError::new(format!("[Io] http client init failed: {e}")))?;
    let url = format!("{base_url}?q={}&count={}", url_encode(query), max_results);
    let response = client
        .get(&url)
        .header("X-Subscription-Token", api_key)
        .send()
        .await
        .map_err(|e| ToolError::new(format!("[Net] brave request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(ToolError::new(format!(
            "[Net] brave returned HTTP {}",
            response.status()
        )));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|e| ToolError::new(format!("[Net] invalid brave response: {e}")))?;
    let mut items = Vec::new();
    if let Some(results) = payload
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        for result in results {
            items.push(ResultItem {
                title: result
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: result
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                snippet: result
                    .get("description")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(items)
}

fn url_encode(query: &str) -> String {
    let mut out = String::new();
    for byte in query.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web and return the top results (title, URL, snippet). Provider is configured via [tools] web_search (duckduckgo needs no key; tavily and brave need a key). The free duckduckgo endpoint rate-limits aggressively: space out searches, combine keywords into one query, and prefer fetching known URLs directly with web_fetch (datasheets, vendor pages, RSS)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "The search query, e.g. STM32F407 EXTI rising edge glitch"},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 8, "default": 5}
            },
            "required": ["query"]
        })
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|q| q.as_str())
            .ok_or_else(|| ToolError::new("[InvalidInput] missing query"))?
            .trim();
        if query.is_empty() {
            return Err(ToolError::new("[InvalidInput] empty query"));
        }
        let max_results = args
            .get("max_results")
            .and_then(|m| m.as_u64())
            .unwrap_or(5)
            .clamp(1, 8) as usize;
        let provider = ctx.web_search_provider.as_deref().unwrap_or("duckduckgo");
        let api_key = ctx.web_search_api_key.as_deref().unwrap_or("");
        let items = match provider {
            "tavily" => {
                tavily("https://api.tavily.com/search", query, max_results, api_key).await?
            }
            "brave" => {
                brave(
                    "https://api.search.brave.com/res/v1/web/search",
                    query,
                    max_results,
                    api_key,
                )
                .await?
            }
            "duckduckgo" => {
                duckduckgo(
                    "https://html.duckduckgo.com/html",
                    "https://lite.duckduckgo.com/lite",
                    query,
                    max_results,
                )
                .await?
            }
            "bing" => bing_html("https://cn.bing.com/search", query, max_results).await?,
            other => {
                return Err(ToolError::new(format!(
                    "[InvalidInput] unknown web_search provider '{other}' (expected duckduckgo / bing / tavily / brave)"
                )));
            }
        };
        Ok(ToolOutput {
            text: format_results(query, &items, max_results),
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

    fn ctx(dir: &Path, provider: Option<&str>, key: Option<&str>) -> ToolContext {
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
            web_search_provider: provider.map(|s| s.to_string()),
            web_search_api_key: key.map(|s| s.to_string()),
            session_dir: None,
            ledger_path: None,
            providers: Vec::new(),
            allowed_roots: Vec::new(),
            cancel: firment_core::Cancellable::new(),
        }
    }

    #[tokio::test]
    async fn unknown_provider_is_an_error() {
        let dir = tempdir().unwrap();
        let err = WebSearch
            .run(
                json!({"query": "stm32"}),
                &ctx(dir.path(), Some("yandex"), None),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("[InvalidInput]"), "got: {err}");
    }

    #[tokio::test]
    async fn tavily_requires_a_key() {
        let dir = tempdir().unwrap();
        let err = WebSearch
            .run(
                json!({"query": "stm32"}),
                &ctx(dir.path(), Some("tavily"), None),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("API key"), "got: {err}");
    }

    #[tokio::test]
    async fn tavily_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "query": "stm32",
                "results": [
                    {"title": "STM32 Website", "url": "https://www.st.com/stm32", "content": "Official STM32 page"},
                    {"title": "Datasheet", "url": "https://x.com/ds.pdf", "content": "Full datasheet"}
                ]
            })))
            .mount(&server)
            .await;
        let items = tavily(&format!("{}/search", server.uri()), "stm32", 5, "test-key")
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "STM32 Website");
        assert_eq!(items[0].url, "https://www.st.com/stm32");
        assert_eq!(items[1].snippet, "Full datasheet");
    }

    #[tokio::test]
    async fn brave_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "web": {"results": [
                    {"title": "STM32", "url": "https://st.com", "description": "Official"},
                    {"title": "Errata", "url": "https://st.com/es", "description": "Errata sheet"}
                ]}
            })))
            .mount(&server)
            .await;
        let items = brave(
            &format!("{}/v1/web/search", server.uri()),
            "stm32",
            5,
            "test-key",
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].snippet, "Errata sheet");
    }

    #[tokio::test]
    async fn duckduckgo_parses_uddg_links() {
        let server = MockServer::start().await;
        let encoded = "https%3A%2F%2Fwww.st.com%2Fen%2Fmicrocontrollers";
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    "<html><div class=\"result\"><a rel=\"nofollow\" class=\"result__a\" href=\"//duckduckgo.com/l/?uddg={encoded}&amp;rut=abc\">STM32 <b>microcontrollers</b></a><a class=\"result__snippet\" href=\"//duckduckgo.com/l/?uddg={encoded}\">Official page</a></div></html>"
                )
                .into_bytes(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let items = duckduckgo_html(&format!("{}/html", server.uri()), "stm32", 5)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "STM32 microcontrollers");
        assert_eq!(items[0].url, "https://www.st.com/en/microcontrollers");
        assert_eq!(items[0].snippet, "Official page");
    }

    #[tokio::test]
    async fn duckduckgo_lite_parses_result_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/lite"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<table><tr><td><a rel=\"nofollow\" class=\"result-link\" href=\"https://www.st.com/stm32\">STM32 Website</a></td></tr><tr><td class=\"result-snippet\">Official STM32 page</td></tr><tr><td><a rel=\"nofollow\" class=\"result-link\" href=\"https://x.com/ds.pdf\">Datasheet</a></td></tr></table>"
                    .as_bytes()
                    .to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let items = duckduckgo_lite(&format!("{}/lite", server.uri()), "stm32", 5)
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "STM32 Website");
        assert_eq!(items[0].url, "https://www.st.com/stm32");
        assert_eq!(items[0].snippet, "Official STM32 page");
        assert_eq!(items[1].title, "Datasheet");
    }

    #[tokio::test]
    async fn duckduckgo_anomaly_page_is_reported_not_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><body><h1>duckduckgo.com</h1><p>Anomaly</p><p>If this is your first visit...</p></body></html>"
                    .as_bytes()
                    .to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let err = duckduckgo_html(&format!("{}/html", server.uri()), "stm32", 5)
            .await
            .unwrap_err();
        assert!(err.message.contains("[Blocked]"), "got: {err}");
        assert!(err.message.contains("web_fetch"), "got: {err}");
    }

    #[tokio::test]
    async fn empty_page_without_no_results_message_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><head><title>DuckDuckGo Search</title></head><body><div id=\"links\"></div></body></html>"
                    .as_bytes()
                    .to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let err = duckduckgo_html(&format!("{}/html", server.uri()), "stm32", 5)
            .await
            .unwrap_err();
        assert!(err.message.contains("[Blocked]"), "got: {err}");
        assert!(err.message.contains("rate limit"), "got: {err}");
    }

    #[tokio::test]
    async fn genuine_no_results_page_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(
                    "<html><body><div>No results found for your query</div></body></html>"
                        .as_bytes()
                        .to_vec(),
                    "text/html",
                ),
            )
            .mount(&server)
            .await;
        let items = duckduckgo_html(&format!("{}/html", server.uri()), "x", 5)
            .await
            .unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn duckduckgo_html_blocked_falls_back_to_lite() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(ResponseTemplate::new(202).set_body_raw(
                "<html><p>Anomaly</p></html>".as_bytes().to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/lite"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<table><tr><td><a rel=\"nofollow\" class=\"result-link\" href=\"https://st.com\">STM32</a></td></tr><tr><td class=\"result-snippet\">Official</td></tr></table>"
                    .as_bytes()
                    .to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let items = duckduckgo(
            &format!("{}/html", server.uri()),
            &format!("{}/lite", server.uri()),
            "stm32",
            5,
        )
        .await
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "STM32");
    }

    #[tokio::test]
    async fn bing_html_parses_b_algo_results() {
        let server = MockServer::start().await;
        let html = r#"<html><body><ol id="b_results">
<li class="b_algo"><h2 class=""><a target="_blank" href="https://www.st.com/en/microcontrollers-microprocessors/stm32g4-series.html"><strong>STM32G4 Series</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">High-performance microcontrollers with DSP and FPU.</p></div></li>
<li class="b_algo"><h2 class=""><a target="_blank" href="https://github.com/STMicroelectronics/STM32CubeG4"><strong>STM32CubeG4</strong></a></h2><div class="b_caption"><p class="b_lineclamp2">HAL drivers for the G4 family.</p></div></li>
</ol></body></html>"#;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(html.as_bytes().to_vec(), "text/html"),
            )
            .mount(&server)
            .await;
        let items = bing_html(&server.uri(), "stm32g4", 5).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "STM32G4 Series");
        assert_eq!(
            items[0].url,
            "https://www.st.com/en/microcontrollers-microprocessors/stm32g4-series.html"
        );
        assert!(
            items[0].snippet.contains("DSP"),
            "got: {}",
            items[0].snippet
        );
        assert_eq!(items[1].title, "STM32CubeG4");
    }

    #[tokio::test]
    async fn bing_html_empty_results_is_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                b"<html><body><p>No results found.</p></body></html>".to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let items = bing_html(&server.uri(), "zzz", 5).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn bing_challenge_page_is_reported_not_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><body><h1>Bing</h1><p>Anomaly detected. Please verify you are a human.</p></body></html>"
                    .as_bytes()
                    .to_vec(),
                "text/html",
            ))
            .mount(&server)
            .await;
        let err = bing_html(&server.uri(), "stm32", 5).await.unwrap_err();
        assert!(err.message.contains("[Blocked]"), "got: {err}");
        assert!(err.message.contains("web_fetch"), "got: {err}");
    }

    #[tokio::test]
    async fn bing_empty_page_without_no_results_message_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(
                    "<html><body><ol id=\"b_results\"></ol></body></html>"
                        .as_bytes()
                        .to_vec(),
                    "text/html",
                ),
            )
            .mount(&server)
            .await;
        let err = bing_html(&server.uri(), "zzz", 5).await.unwrap_err();
        assert!(err.message.contains("[Blocked]"), "got: {err}");
        assert!(err.message.contains("rate limit"), "got: {err}");
    }

    #[tokio::test]
    #[ignore = "live network smoke test: cargo test -p firment-tools duckduckgo_live -- --ignored"]
    async fn duckduckgo_live() {
        // Reproduce the reported scenario: a burst of consecutive searches.
        let queries = [
            "STM32F103 USART DMA",
            "STM32F407 EXTI rising edge glitch",
            "stm32 timer remap",
            "STM32L4 low power mode",
            "nucleo stm32",
        ];
        for (i, query) in queries.iter().enumerate() {
            match duckduckgo(
                "https://html.duckduckgo.com/html",
                "https://lite.duckduckgo.com/lite",
                query,
                5,
            )
            .await
            {
                Ok(items) => println!("live[{i}] {:?}: {} results", query, items.len()),
                Err(e) => println!("live[{i}] {:?}: ERROR {}", query, e.message),
            }
        }
    }
}
