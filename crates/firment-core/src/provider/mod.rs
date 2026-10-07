pub mod anthropic;
pub mod openai;
mod sse;

use crate::ToolCall;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

pub use crate::types::ChatRequest;
pub use anthropic::AnthropicProvider;
pub use openai::OpenAIProvider;
pub use sse::CAPTURE_CAP_BYTES;

pub type ProviderStream = BoxStream<'static, Result<ProviderEvent, ProviderError>>;

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    Text(String),
    /// Extended-thinking delta (anthropic `thinking` / OpenRouter
    /// `reasoning` blocks). Never persisted into the transcript — surfaced
    /// to the UI so "the model is reasoning" is observable.
    Thinking(String),
    /// A COMPLETE thinking block (text + signature, or redacted), captured
    /// at content_block_stop. Persisted on the assistant message and
    /// replayed first in the next request — the Anthropic API rejects
    /// thinking-enabled tool turns whose assistant messages lost them.
    ThinkingBlock(serde_json::Value),
    ToolCall(ToolCall),
    Stop(StopReason),
    /// Liveness heartbeat: the stream delivered another network chunk, which
    /// is all it proves. Emitted once per chunk by the SSE parsers so a model
    /// that is slowly generating one huge tool payload keeps the inactivity
    /// timer armed. Never persisted, never forwarded to the UI.
    Activity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    Other(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// The provider reported an error *inside* an already-open stream (HTTP
    /// 200 was spent on the response headers, so there is no status worth
    /// showing) — overloaded capacity, a content-filter abort, a gateway
    /// failure mid-response. Carries the server's own message.
    #[error("stream error: {0}")]
    StreamEnded(String),
}

/// Compact an HTTP error body for display. Error payloads occasionally come
/// back as entire HTML pages (proxies, wrong paths) — collapse whitespace,
/// reduce markup-only bodies to a one-line note, and hard-cap the length so
/// a single failure can't flood the transcript with hundreds of lines.
pub fn sanitize_error_body(body: &str) -> String {
    let one_line = body.split_whitespace().collect::<Vec<&str>>().join(" ");
    let trimmed = one_line.trim();
    if trimmed.starts_with('<') {
        let lower = trimmed.to_ascii_lowercase();
        if let Some(pos) = lower.find("<title>") {
            let rest = &trimmed[pos + "<title>".len()..];
            if let Some(end) = rest.to_ascii_lowercase().find("</title>") {
                let title = rest[..end].trim();
                return format!("non-JSON HTML response ({title})");
            }
        }
        return format!("non-JSON HTML response ({} bytes)", trimmed.len());
    }
    const CAP: usize = 400;
    if trimmed.chars().count() <= CAP {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(CAP).collect();
    format!(
        "{cut}… [truncated, {} chars total]",
        trimmed.chars().count()
    )
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError>;
    fn model(&self) -> &str;
}

pub(crate) fn serialize_tool_arguments(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments).unwrap_or_default()
}

/// Final defense: guarantee a tool-call `arguments` value is a JSON
/// **object** before it is sent to an API, regardless of where it came
/// from (fresh stream, or a persisted session saved by an older build
/// whose arguments were degraded to strings).
pub(crate) fn normalize_tool_arguments(arguments: &serde_json::Value) -> serde_json::Value {
    if arguments.is_object() {
        return arguments.clone();
    }
    match arguments {
        Value::String(s) => collect_tool_arguments(s),
        other => collect_tool_arguments(&other.to_string()),
    }
}

/// Coerce a model's raw `arguments` string into a JSON **object**.
///
/// OpenAI-compatible APIs reject assistant `tool_calls` whose `arguments`
/// is not a JSON object, and some models stream `arguments` with
/// markdown fences, leading/trailing prose or trailing commas. Every
/// failure path below degrades to `{}` (never a string/array), so the
/// round-tripped history stays valid; the tool's own schema validation
/// then tells the model exactly what to fix.
pub(crate) fn collect_tool_arguments(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    // Direct parse: only a clean object is accepted as-is.
    if let Ok(Value::Object(_)) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return serde_json::from_str(trimmed).unwrap_or_default();
    }
    // Recover the object portion (handles ```json fences, prose around
    // the JSON, and extra trailing tokens).
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    let candidate = match (start, end) {
        (Some(s), Some(e)) if e > s => trimmed[s..=e].to_string(),
        _ => return serde_json::Value::Object(Default::default()),
    };
    if let Ok(Value::Object(_)) = serde_json::from_str::<serde_json::Value>(&candidate) {
        return serde_json::from_str(&candidate).unwrap_or_default();
    }
    // Last resort: strip trailing commas before } or ] (a common
    // hallucinated artifact) and try again.
    let mut fixed = candidate;
    loop {
        let prev = fixed.clone();
        fixed = fixed
            .replace(",}", "}")
            .replace(",]", "]")
            .replace(",\n}", "\n}")
            .replace(",\n]", "\n]");
        if fixed == prev {
            break;
        }
    }
    if let Ok(Value::Object(_)) = serde_json::from_str::<serde_json::Value>(&fixed) {
        return serde_json::from_str(&fixed).unwrap_or_default();
    }
    serde_json::Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_body_html_collapses_to_one_line() {
        let html =
            "<!DOCTYPE html><html>\n  <head> <title>404 Not Found</title> </head>\n".repeat(50);
        let out = sanitize_error_body(&html);
        assert_eq!(out, "non-JSON HTML response (404 Not Found)");
        assert!(!out.contains('\n'));
    }

    #[test]
    fn error_body_html_without_title_reports_size() {
        let out = sanitize_error_body("<div><br></div><div>x</div>");
        assert!(out.starts_with("non-JSON HTML response ("));
        assert!(out.ends_with("bytes)"));
    }

    #[test]
    fn error_body_long_json_is_capped() {
        let body = format!("{{\"error\":{{\"message\":\"{}\"}}}}", "x".repeat(5000));
        let out = sanitize_error_body(&body);
        assert!(out.contains("… [truncated"));
        assert!(out.chars().count() < 460);
    }

    #[test]
    fn error_body_short_json_passes_through() {
        let out = sanitize_error_body("{\"error\":{\"type\":\"invalid_model\"}}");
        assert_eq!(out, "{\"error\":{\"type\":\"invalid_model\"}}");
    }

    #[test]
    fn collect_accepts_clean_object() {
        let v = collect_tool_arguments(r#"{"path": "main.c", "line": 3}"#);
        assert_eq!(v, json!({"path": "main.c", "line": 3}));
    }

    #[test]
    fn collect_accepts_empty_and_whitespace() {
        assert_eq!(collect_tool_arguments(""), json!({}));
        assert_eq!(collect_tool_arguments("  "), json!({}));
    }

    #[test]
    fn collect_recovers_object_from_fenced_json() {
        let v = collect_tool_arguments("```json\n{\"path\": \"main.c\"}\n```");
        assert_eq!(v, json!({"path": "main.c"}));
    }

    #[test]
    fn collect_recovers_object_from_prose_around_json() {
        let v = collect_tool_arguments(
            "The file to edit is:\n{\"path\": \"main.c\", \"new_string\": \"x\"}\nPlease apply it.",
        );
        assert_eq!(v, json!({"path": "main.c", "new_string": "x"}));
    }

    #[test]
    fn collect_recovers_object_with_trailing_commas() {
        let v = collect_tool_arguments("{\"path\": \"main.c\",}");
        assert_eq!(v, json!({"path": "main.c"}));
        let v = collect_tool_arguments("{\"a\": [1, 2,],}");
        assert_eq!(v, json!({"a": [1, 2]}));
    }

    #[test]
    fn collect_never_returns_non_object() {
        // A plain-text argument (previously became a JSON string and made
        // OpenAI-compatible APIs reject the round-tripped history with
        // "Assistant tool call arguments must be a JSON object").
        assert_eq!(collect_tool_arguments("just go ahead"), json!({}));
        assert_eq!(collect_tool_arguments("\"quoted text\""), json!({}));
        assert_eq!(collect_tool_arguments("[1, 2, 3]"), json!({}));
        assert_eq!(collect_tool_arguments("{broken json"), json!({}));
    }

    #[test]
    fn normalize_repairs_persisted_string_arguments() {
        // Old sessions saved by a previous build may hold string-shaped
        // arguments; the send path must repair them, never forward them.
        assert_eq!(
            normalize_tool_arguments(&json!("{\"path\": \"main.c\"}")),
            json!({"path": "main.c"})
        );
        assert_eq!(normalize_tool_arguments(&json!("just go ahead")), json!({}));
        assert_eq!(
            normalize_tool_arguments(&json!({"path": "main.c"})),
            json!({"path": "main.c"})
        );
        assert_eq!(normalize_tool_arguments(&json!([1, 2])), json!({}));
    }

    // ---------------------------------------------------------------------
    // The SSE parsers, driven the way the network loop drives them.
    //
    // `openai.rs` had no test of any kind before this, because there was no way
    // to get bytes into it: the parse lived inside the `async_stream` macro,
    // which needs a socket, a runtime and a server. Splitting the per-line half
    // out (`OpenAiSse`, `AnthropicSse`) is what makes the two frames that matter
    // testable -- and the table below is why it is worth more than two tests: the
    // properties belong to every dialect, so a third parser added later joins the
    // list and inherits them instead of discovering the tail-flush rule the hard
    // way for the fourth time.
    // ---------------------------------------------------------------------

    use super::sse::SseLineBuffer;
    use crate::provider::anthropic::AnthropicSse;
    use crate::provider::openai::OpenAiSse;

    /// What the production loop does with a chunk stream, so the tests cannot
    /// diverge from it without the divergence being visible here.
    trait Dialect {
        fn on_line(
            &mut self,
            line: &[u8],
            out: &mut Vec<ProviderEvent>,
        ) -> Result<(), ProviderError>;
        fn finish(&mut self, out: &mut Vec<ProviderEvent>);
        /// True once the reader stops consuming further lines (`[DONE]`).
        fn stopped(&self) -> bool {
            false
        }
    }

    impl Dialect for OpenAiSse {
        fn on_line(
            &mut self,
            line: &[u8],
            out: &mut Vec<ProviderEvent>,
        ) -> Result<(), ProviderError> {
            OpenAiSse::on_line(self, line, out)
        }
        fn finish(&mut self, out: &mut Vec<ProviderEvent>) {
            OpenAiSse::finish(self, out)
        }
        fn stopped(&self) -> bool {
            self.done
        }
    }

    impl Dialect for AnthropicSse {
        fn on_line(
            &mut self,
            line: &[u8],
            out: &mut Vec<ProviderEvent>,
        ) -> Result<(), ProviderError> {
            AnthropicSse::on_line(self, line, out)
        }
        fn finish(&mut self, out: &mut Vec<ProviderEvent>) {
            AnthropicSse::finish(self, out)
        }
    }

    fn drive<D: Dialect + Default>(
        chunks: &[&[u8]],
    ) -> (Vec<ProviderEvent>, Option<ProviderError>) {
        let mut buf = SseLineBuffer::default();
        let mut parser = D::default();
        let mut events: Vec<ProviderEvent> = Vec::new();
        for &chunk in chunks {
            let lines = match buf.push(chunk) {
                Ok(lines) => lines,
                Err(e) => return (events, Some(e)),
            };
            for line in lines {
                if let Err(e) = parser.on_line(&line, &mut events) {
                    return (events, Some(e));
                }
            }
            // A body may carry frames after the sentinel; the reader stops
            // where the production loop stops.
            if parser.stopped() {
                let mut out = Vec::new();
                parser.finish(&mut out);
                events.extend(out);
                return (events, None);
            }
        }
        for line in buf.finish() {
            if let Err(e) = parser.on_line(&line, &mut events) {
                return (events, Some(e));
            }
        }
        parser.finish(&mut events);
        (events, None)
    }

    type DialectFn = fn(&[&[u8]]) -> (Vec<ProviderEvent>, Option<ProviderError>);

    fn dialects() -> Vec<(&'static str, DialectFn)> {
        vec![
            ("openai", drive::<OpenAiSse> as DialectFn),
            ("anthropic", drive::<AnthropicSse> as DialectFn),
        ]
    }

    /// A complete reply that ends with a stop reason: text, a tool call in two
    /// fragments, and the frame that says which of the two the model chose.
    /// Every dialect's tail is its own, and in both dialects losing it is
    /// expensive: `content_block_stop` is the ONLY place anthropic emits a
    /// `ToolCall`, and `message_delta` the only place it learns the stop reason.
    fn body(dialect: &str, newline: bool) -> Vec<u8> {
        let frames: Vec<&str> = match dialect {
            "openai" => vec![
                r#"data: {"choices":[{"delta":{"content":"reading it now"}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"read_file","arguments":"{\"path\""}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":": \"src/main.c\"}"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            ],
            _ => vec![
                r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"read_file"}}"#,
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"src/main.c\"}"}}"#,
                r#"data: {"type":"content_block_stop","index":0}"#,
                r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            ],
        };
        let mut text = frames.join("\n\n");
        if newline {
            text.push('\n');
        }
        text.into_bytes()
    }

    fn render(events: &[ProviderEvent]) -> String {
        events
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn a_reply_ending_without_a_newline_parses_as_the_same_reply() {
        // The defect: bytes were parsed only when a `\n` arrived, so the last
        // frame of a body that ends cleanly was dropped. Both dialects, one
        // property -- the newline is a framing detail, not part of the answer.
        for (name, run) in dialects() {
            let with = run(&[&body(name, true)]);
            let without = run(&[&body(name, false)]);
            assert!(
                with.1.is_none(),
                "{name}: the well-formed body must not error: {:?}",
                with.1
            );
            assert!(
                without.1.is_none(),
                "{name}: a body missing only its final newline must not error either: {:?}",
                without.1
            );
            assert_eq!(
                render(&with.0),
                render(&without.0),
                "{name}: the final frame was parsed differently with and without its newline"
            );
            let text = render(&with.0);
            assert!(
                text.contains("read_file") && text.contains("src/main.c"),
                "{name}: the tool call must survive, got {text}"
            );
            assert!(
                text.contains("ToolUse"),
                "{name}: the stop reason the server sent must be the one reported, not the \
                 fallback for \"nothing arrived\": {text}"
            );
        }
    }

    #[test]
    fn a_reply_ending_on_a_newline_is_not_parsed_twice() {
        // The other direction of the same repair: flushing what is buffered is
        // only correct if there is nothing buffered. Without this assertion the
        // tail flush could re-emit the last frame on every well-formed stream,
        // which is the worse of the two failures and the one a fix would produce
        // by forgetting `finish()` empties the buffer.
        for (name, run) in dialects() {
            let events = run(&[&body(name, true)]).0;
            let stops = events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::Stop(_)))
                .count();
            let calls = events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::ToolCall(_)))
                .count();
            assert_eq!(
                stops, 1,
                "{name}: a duplicated stop reason ends the turn twice"
            );
            assert_eq!(
                calls, 1,
                "{name}: a duplicated tool call runs the tool twice"
            );
        }
    }

    #[test]
    fn bytes_that_never_form_a_line_are_refused_rather_than_collected() {
        // The ceiling. An endpoint that trickles a line without ever ending it
        // used to grow the buffer until the process could not, while the
        // activity heartbeat re-armed the inactivity timer for every chunk of
        // it -- so the loop was never interrupted either.
        for (name, run) in dialects() {
            let filler = vec![b'x'; CAPTURE_CAP_BYTES / 8];
            let chunks: Vec<&[u8]> = vec![filler.as_slice(); 9];
            let (events, error) = run(&chunks);
            let error = error.unwrap_or_else(|| panic!("{name}: the ceiling never fired"));
            assert!(
                error.to_string().contains("capture ceiling"),
                "{name}: the refusal must name the ceiling it hit: {error}"
            );
            assert!(
                !events.iter().any(|e| matches!(e, ProviderEvent::Stop(_))),
                "{name}: an unfinished stream must not report a finished turn: {:?}",
                render(&events)
            );
        }
    }

    #[test]
    fn the_stream_table_covers_every_parser_that_reads_a_body() {
        // The table above is the instrument only if it holds every dialect: two
        // tests written for the two parsers that exist today are the same list
        // someone has to remember to extend, which is what this round keeps
        // finding. So the list is checked against the source, not against memory.
        let mut parsers = Vec::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/provider");
        for entry in std::fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // The test module below quotes the needle to look for it; a scrape
            // that counted its own search string would report this file as a
            // parser that nobody tested.
            let product = match text.find("#[cfg(test)]") {
                Some(at) => &text[..at],
                None => &text[..],
            };
            if product.contains("bytes_stream(") {
                parsers.push(
                    path.file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .replace('-', "_"),
                );
            }
        }
        parsers.sort();
        for name in &parsers {
            assert!(
                dialects().iter().any(|(dialect, _)| dialect == name),
                "{name} reads a stream but is not in the table, so the tail-flush and ceiling \
                 properties below do not cover it"
            );
        }
        assert!(
            parsers.len() >= 2,
            "the scrape found {} stream parsers; it is not reading this directory",
            parsers.len()
        );
    }
}
