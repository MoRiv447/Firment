use super::{Provider, ProviderError, ProviderEvent, StopReason};
use crate::{ChatMessage, ChatRequest, ThinkingLevel, ToolCall};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vendor {
    OpenAI,
    DeepSeek,
    Other,
}

impl Vendor {
    fn detect(base_url: &str, model: &str) -> Self {
        let haystack = format!(
            "{} {}",
            base_url.to_ascii_lowercase(),
            model.to_ascii_lowercase()
        );
        if haystack.contains("deepseek") {
            Self::DeepSeek
        } else if haystack.contains("openai") {
            Self::OpenAI
        } else {
            Self::Other
        }
    }
}

#[derive(Clone)]
pub struct OpenAIProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    vendor: Vendor,
}

impl OpenAIProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let model = model.into();
        let vendor = Vendor::detect(&base_url, &model);
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key: api_key.into(),
            model,
            max_tokens,
            temperature,
            vendor,
        }
    }

    fn messages(&self, messages: &[ChatMessage]) -> Vec<Value> {
        // Some OpenAI-compatible gateways reject empty-string content
        // (e.g. agnes' Anthropic-compat layer surfaces messages.203).
        // A stalled/cancelled turn can leave an assistant message with
        // neither text nor tool calls; tool results may be empty — give
        // every role a non-empty content.
        fn non_empty(s: &str, fallback: &str) -> String {
            if s.trim().is_empty() {
                fallback.to_string()
            } else {
                s.to_string()
            }
        }
        messages
            .iter()
            .map(|m| match m {
                ChatMessage::System { content } => {
                    json!({"role": "system", "content": non_empty(content, "…")})
                }
                ChatMessage::User { content } => {
                    json!({"role": "user", "content": non_empty(content, "…")})
                }
                ChatMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut v = json!({
                        "role": "assistant",
                        "content": non_empty(content, "…")
                    });
                    if !tool_calls.is_empty() {
                        v["tool_calls"] = json!(
                            tool_calls
                                .iter()
                                .map(|tc| json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": super::serialize_tool_arguments(
                                            &super::normalize_tool_arguments(&tc.arguments),
                                        ),
                                    }
                                }))
                                .collect::<Vec<_>>()
                        );
                    }
                    v
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": non_empty(content, "(no output)")
                }),
            })
            .collect()
    }

    fn tools(&self, tools: &[crate::ToolSpec]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }

    fn request_body(&self, request: &ChatRequest) -> Value {
        let mut body = json!({
            "model": request.model,
            "stream": true,
            "messages": self.messages(&request.messages),
        });
        let tools = self.tools(&request.tools);
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        // Per-request caps (e.g. the 2048-token summarization cap) win over
        // the provider-level default, mirroring anthropic.rs — otherwise
        // build_provider's always-present default made request.max_tokens
        // dead code.
        if let Some(t) = request.max_tokens.or(self.max_tokens) {
            body["max_tokens"] = json!(t);
        }
        if let Some(t) = self.temperature.or(request.temperature) {
            body["temperature"] = json!(t);
        }
        if let Some(level) = request.thinking.filter(|l| *l != ThinkingLevel::Off) {
            match self.vendor {
                // DeepSeek V4: thinking must be explicitly enabled, and only
                // high/max are accepted today (low/medium map to high,
                // xhigh maps to max).
                Vendor::DeepSeek => {
                    body["thinking"] = json!({"type": "enabled"});
                    body["reasoning_effort"] = json!(match level {
                        ThinkingLevel::Low | ThinkingLevel::Medium | ThinkingLevel::High => "high",
                        ThinkingLevel::XHigh | ThinkingLevel::Max => "max",
                        ThinkingLevel::Off => "high",
                    });
                }
                _ => {
                    let effort = match level {
                        ThinkingLevel::Off => "high",
                        ThinkingLevel::Low => "low",
                        ThinkingLevel::Medium => "medium",
                        ThinkingLevel::High => "high",
                        ThinkingLevel::XHigh => "xhigh",
                        ThinkingLevel::Max => "max",
                    };
                    body["reasoning_effort"] = json!(effort);
                }
            }
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn stream(&self, request: ChatRequest) -> Result<super::ProviderStream, ProviderError> {
        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&self.request_body(&request))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let mut chunks = response.bytes_stream();
        let stream = async_stream::stream! {
            let mut line_buf: Vec<u8> = Vec::new();
            let mut tool_acc: HashMap<usize, AccumTool> = HashMap::new();
            let mut stop_emitted = false;
            let mut done = false;

            while let Some(chunk) = chunks.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ProviderError::Http(e));
                        return;
                    }
                };
                for &b in chunk.iter() {
                    line_buf.push(b);
                    if b != b'\n' {
                        continue;
                    }
                    let line = String::from_utf8_lossy(&line_buf).trim().to_string();
                    line_buf.clear();
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data.is_empty() {
                        // SSE heartbeat / keep-alive frame; legal, ignore it.
                        continue;
                    }
                    if data == "[DONE]" {
                        done = true;
                        break;
                    }
                    let payload: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            yield Err(ProviderError::InvalidResponse(format!(
                                "bad SSE payload: {e}"
                            )));
                            return;
                        }
                    };

                    if let Some(delta) = payload.pointer("/choices/0/delta") {
                        if let Some(text) = delta.get("content").and_then(|c| c.as_str())
                            && !text.is_empty()
                        {
                            yield Ok(ProviderEvent::Text(text.to_string()));
                        }
                        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                            for tc in calls {
                                let idx = tc
                                    .get("index")
                                    .and_then(|i| i.as_u64())
                                    .unwrap_or(0) as usize;
                                let entry = tool_acc.entry(idx).or_default();
                                if let Some(id) = tc.get("id").and_then(|i| i.as_str())
                                    && !id.is_empty()
                                {
                                    entry.id = id.to_string();
                                }
                                if let Some(f) = tc.get("function") {
                                    if let Some(name) = f.get("name").and_then(|n| n.as_str())
                                        && !name.is_empty()
                                    {
                                        entry.name = name.to_string();
                                    }
                                    if let Some(args) = f.get("arguments").and_then(|a| a.as_str()) {
                                        entry.arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }

                    if let Some(reason) = payload
                        .pointer("/choices/0/finish_reason")
                        .and_then(|f| f.as_str())
                        && !reason.is_empty()
                        && !stop_emitted
                    {
                        let reason = match reason {
                            "tool_calls" => StopReason::ToolUse,
                            "length" => StopReason::MaxTokens,
                            "stop" => StopReason::EndTurn,
                            other => StopReason::Other(other.to_string()),
                        };
                        yield Ok(ProviderEvent::Stop(reason));
                        stop_emitted = true;
                    }
                }
                if done {
                    break;
                }
            }

            let mut indexes: Vec<usize> = tool_acc.keys().copied().collect();
            indexes.sort_unstable();
            for idx in indexes {
                let entry = tool_acc.remove(&idx).unwrap_or_default();
                if entry.name.is_empty() {
                    continue;
                }
                yield Ok(ProviderEvent::ToolCall(ToolCall {
                    id: if entry.id.is_empty() {
                        format!("call_{idx}")
                    } else {
                        entry.id
                    },
                    name: entry.name,
                    arguments: super::collect_tool_arguments(&entry.arguments),
                }));
            }
            if !stop_emitted {
                yield Ok(ProviderEvent::Stop(StopReason::EndTurn));
            }
        };
        Ok(Box::pin(stream))
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[derive(Default)]
struct AccumTool {
    id: String,
    name: String,
    arguments: String,
}
