pub mod anthropic;
pub mod openai;

use crate::ToolCall;
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::Value;

pub use crate::types::ChatRequest;
pub use anthropic::AnthropicProvider;
pub use openai::OpenAIProvider;

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
    #[error("stream ended unexpectedly")]
    StreamEnded,
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
}
