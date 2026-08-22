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
                    let text = if content.trim().is_empty() {
                        "…".to_string()
                    } else {
                        content.clone()
                    };
                    out.push(json!({"role": "user", "content": [{"type": "text", "text": text}]}));
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
                            "input": super::normalize_tool_arguments(&tc.arguments),
                        }));
                    }
                    // Anthropic rejects an assistant message whose content
                    // block list is empty (messages.203) — a stalled or
                    // cancelled turn can have neither text nor tool calls.
                    if blocks.is_empty() {
                        blocks.push(json!({"type": "text", "text": "…"}));
                    }
                    out.push(json!({"role": "assistant", "content": blocks}));
                }
                ChatMessage::Tool {
                    tool_call_id,
                    content,
                    ..
                } => {
                    let result = if content.trim().is_empty() {
                        "(no output)".to_string()
                    } else {
                        content.clone()
                    };
                    let block = json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": result,
                    });
                    // Anthropic requires every tool_use block of an assistant
                    // message to be answered by tool_result blocks inside the
                    // single message that immediately follows it. A wave of
                    // parallel tool calls pushes one result per
                    // ChatMessage::Tool, so consecutive results must be merged
                    // into ONE user message — otherwise the API rejects the
                    // request (400: "tool_use ids ... without tool_result").
                    let last_is_tool_results = matches!(
                        out.last(),
                        Some(Value::Object(last))
                            if last.get("role").and_then(|r| r.as_str()) == Some("user")
                                && last.get("content").and_then(|c| c.as_array()).is_some_and(
                                    |blocks| {
                                        !blocks.is_empty()
                                            && blocks.iter().all(|b| {
                                                b.get("type").and_then(|t| t.as_str())
                                                    == Some("tool_result")
                                            })
                                    }
                                )
                    );
                    if last_is_tool_results {
                        if let Some(Value::Object(last)) = out.last_mut()
                            && let Some(blocks) =
                                last.get_mut("content").and_then(|c| c.as_array_mut())
                        {
                            blocks.push(block);
                        }
                    } else {
                        out.push(json!({
                            "role": "user",
                            "content": [block]
                        }));
                    }
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
        // Per-request max_tokens (e.g. the summarization cap) wins over the
        // session default so callers can bound token output independently.
        let max_tokens = request.max_tokens.unwrap_or(self.max_tokens);
        let mut body = json!({
            "model": request.model,
            "max_tokens": max_tokens,
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
                    if data == "[DONE]" {
                        // OpenAI-style stream sentinel: Anthropic's official API
                        // never sends it, but compatible gateways (OpenRouter,
                        // ...) terminate their anthropic-flavored streams with
                        // it. Skip instead of failing the whole turn.
                        continue;
                    }
                    if stop_emitted {
                        // Anything after message_stop is trailer noise from
                        // compatibility gateways (sentinels, keep-alives,
                        // usage pings) — the turn is already complete.
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
                                        match blocks.get_mut(&idx) {
                                            Some(Block::Text(buf)) => {
                                                buf.push_str(text);
                                            }
                                            // A protocol-conformant server never
                                            // sends text deltas for a tool_use
                                            // index; overwriting the accumulator
                                            // here would destroy the tool call.
                                            None => {
                                                blocks.insert(idx, Block::Text(text.to_string()));
                                            }
                                            Some(Block::ToolUse { .. }) => {}
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
                                // A gateway omitting the id would round-trip
                                // empty tool_use/tool_result ids, which strict
                                // APIs reject — synthesize a stable one.
                                let id = if id.is_empty() {
                                    format!("toolu_synthesized_{idx}")
                                } else {
                                    id
                                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    #[test]
    fn convert_guarantees_non_empty_content_blocks() {
        let p = AnthropicProvider::new("http://localhost", "k", "m", None, None);
        let messages = vec![
            ChatMessage::User {
                content: String::new(),
            },
            // Stalled turn: assistant with neither text nor tool calls.
            ChatMessage::Assistant {
                content: String::new(),
                tool_calls: vec![],
            },
            ChatMessage::Tool {
                tool_call_id: "call_00".to_string(),
                name: "list_dir".to_string(),
                content: String::new(),
            },
        ];
        let (_, out) = p.convert(&messages);
        assert!(
            out.iter().all(|m| {
                m["content"]
                    .as_array()
                    .map(|blocks| !blocks.is_empty())
                    .unwrap_or(false)
            }),
            "no message may have an empty content block list: {out:?}"
        );
        assert_eq!(out[0]["content"][0]["text"], "…");
        assert_eq!(out[1]["content"][0]["text"], "…");
        assert_eq!(out[2]["content"][0]["content"], "(no output)");
    }

    #[test]
    fn parallel_tool_results_merge_into_one_user_message() {
        let p = AnthropicProvider::new("http://localhost", "k", "m", None, None);
        let messages = vec![
            ChatMessage::User {
                content: "go".to_string(),
            },
            ChatMessage::Assistant {
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "call_00".to_string(),
                        name: "list_dir".to_string(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "call_01".to_string(),
                        name: "ask_user".to_string(),
                        arguments: json!({}),
                    },
                ],
            },
            ChatMessage::Tool {
                tool_call_id: "call_00".to_string(),
                name: "list_dir".to_string(),
                content: String::new(),
            },
            ChatMessage::Tool {
                tool_call_id: "call_01".to_string(),
                name: "ask_user".to_string(),
                content: "answer".to_string(),
            },
        ];
        let (_, out) = p.convert(&messages);
        assert_eq!(
            out.len(),
            3,
            "expected user/assistant/merged-user, got: {out:?}"
        );
        let merged = &out[2];
        assert_eq!(merged["role"], "user");
        let blocks = merged["content"].as_array().expect("content array");
        assert_eq!(blocks.len(), 2);
        assert!(
            blocks.iter().all(|b| b["type"] == "tool_result"),
            "all blocks must be tool_result: {blocks:?}"
        );
        assert_eq!(blocks[0]["tool_use_id"], "call_00");
        assert_eq!(blocks[1]["tool_use_id"], "call_01");
    }

    #[test]
    fn tool_result_after_user_text_stays_its_own_message() {
        let p = AnthropicProvider::new("http://localhost", "k", "m", None, None);
        let messages = vec![
            ChatMessage::User {
                content: "hi".to_string(),
            },
            ChatMessage::Tool {
                tool_call_id: "call_00".to_string(),
                name: "list_dir".to_string(),
                content: "[]".to_string(),
            },
        ];
        let (_, out) = p.convert(&messages);
        assert_eq!(
            out.len(),
            2,
            "user text and tool result stay separate: {out:?}"
        );
        assert_eq!(out[1]["content"][0]["type"], "tool_result");
    }
}
