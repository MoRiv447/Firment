use super::{Provider, ProviderError, ProviderEvent, StopReason};
use crate::{ChatMessage, ChatRequest, ThinkingLevel, ToolCall};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    temperature: Option<f32>,
}

impl AnthropicProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: max_tokens.unwrap_or(8192),
            temperature,
        }
    }

    fn convert(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
        let mut system = String::new();
        let mut out = Vec::new();
        for m in messages {
            match m {
                ChatMessage::System { content } => {
                    if !system.is_empty() {
                        system.push_str("\n\n");
                    }
                    system.push_str(content);
                }
                ChatMessage::User { content } => {
                    out.push(
                        json!({"role": "user", "content": [{"type": "text", "text": content}]}),
                    );
                }
                ChatMessage::Assistant {
                    content,
                    tool_calls,
                } => {
                    let mut blocks = Vec::new();
                    if !content.is_empty() {
                        blocks.push(json!({"type": "text", "text": content}));
                    }
                    for tc in tool_calls {
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                    out.push(json!({"role": "assistant", "content": blocks}));
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => {
                    out.push(json!({
                        "role": "user",
                        "content": [{"type": "tool_result", "tool_use_id": tool_call_id, "content": content}]
                    }));
                }
            }
        }
        let system = if system.is_empty() {
            None
        } else {
            Some(system)
        };
        (system, out)
    }

    fn body(&self, request: &ChatRequest) -> Value {
        let (system, messages) = self.convert(&request.messages);
        let mut body = json!({
            "model": request.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": messages,
        });
        if let Some(s) = system {
            body["system"] = json!(s);
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(
                request
                    .tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(t) = self.temperature.or(request.temperature) {
            body["temperature"] = json!(t);
        }
        if let Some(level) = request.thinking.filter(|l| *l != ThinkingLevel::Off) {
            let budget = match level {
                ThinkingLevel::Low => 1024,
                ThinkingLevel::Medium => 4096,
                ThinkingLevel::High => 8192,
                ThinkingLevel::XHigh => 16384,
                ThinkingLevel::Max => 32768,
                ThinkingLevel::Off => 1024,
            };
            body["max_tokens"] = json!(self.max_tokens.max(budget + 2048));
            body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            body.as_object_mut().unwrap().remove("temperature");
        }
        body
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream(&self, request: ChatRequest) -> Result<super::ProviderStream, ProviderError> {
        let url = format!("{}/v1/messages", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&self.body(&request))
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
            let mut blocks: HashMap<usize, Block> = HashMap::new();
            let mut stop_emitted = false;

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
                    let payload: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            yield Err(ProviderError::InvalidResponse(format!(
                                "bad SSE payload: {e}"
                            )));
                            return;
                        }
                    };
                    let event = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match event {
                        "content_block_start" => {
                            let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let block = payload.get("content_block").cloned().unwrap_or(Value::Null);
                            let entry = blocks.entry(idx).or_insert_with(|| Block::Text(String::new()));
                            if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                *entry = Block::ToolUse {
                                    id: block.get("id").and_then(|i| i.as_str()).unwrap_or_default().to_string(),
                                    name: block.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                                    arguments: String::new(),
                                };
                            }
                        }
                        "content_block_delta" => {
                            let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let delta = payload.get("delta").cloned().unwrap_or(Value::Null);
                            match delta.get("type").and_then(|t| t.as_str()) {
                                Some("text_delta") => {
                                    if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                        if let Some(Block::Text(buf)) = blocks.get_mut(&idx) {
                                            buf.push_str(text);
                                        } else {
                                            blocks.insert(idx, Block::Text(text.to_string()));
                                        }
                                        yield Ok(ProviderEvent::Text(text.to_string()));
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(partial) = delta.get("partial_json").and_then(|p| p.as_str())
                                        && let Some(Block::ToolUse { arguments, .. }) = blocks.get_mut(&idx)
                                    {
                                        arguments.push_str(partial);
                                    }
                                }
                                _ => {}
                            }
                        }
                        "content_block_stop" => {
                            let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            if let Some(Block::ToolUse { id, name, arguments }) = blocks.remove(&idx)
                                && !name.is_empty()
                            {
                                yield Ok(ProviderEvent::ToolCall(ToolCall {
                                    id,
                                    name,
                                    arguments: super::collect_tool_arguments(&arguments),
                                }));
                            }
                        }
                        "message_delta" => {
                            let reason = payload
                                .pointer("/delta/stop_reason")
                                .and_then(|r| r.as_str())
                                .unwrap_or("");
                            if !reason.is_empty() && !stop_emitted {
                                let reason = match reason {
                                    "end_turn" => StopReason::EndTurn,
                                    "tool_use" => StopReason::ToolUse,
                                    "max_tokens" => StopReason::MaxTokens,
                                    "stop_sequence" => StopReason::StopSequence,
                                    other => StopReason::Other(other.to_string()),
                                };
                                yield Ok(ProviderEvent::Stop(reason));
                                stop_emitted = true;
                            }
                        }
                        _ => {}
                    }
                }
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

enum Block {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
}
