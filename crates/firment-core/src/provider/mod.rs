pub mod anthropic;
pub mod openai;

use crate::ToolCall;
use async_trait::async_trait;
use futures::stream::BoxStream;

pub use crate::types::ChatRequest;
pub use anthropic::AnthropicProvider;
pub use openai::OpenAIProvider;

pub type ProviderStream = BoxStream<'static, Result<ProviderEvent, ProviderError>>;

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    Text(String),
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

pub(crate) fn collect_tool_arguments(raw: &str) -> serde_json::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::Value::String(trimmed.to_string()))
}
