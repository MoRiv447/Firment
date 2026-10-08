use super::sse::SseLineBuffer;
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
            client: crate::provider_client(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: max_tokens.unwrap_or(8192),
            temperature,
        }
    }

    /// Messages endpoint. A base that already carries the `/v1` prefix (e.g.
    /// `https://openrouter.ai/api/v1`, the form `ProviderConfig::models_url`
    /// accepts) must NOT get a second one — `/api/v1/v1/messages` is a
    /// guaranteed 404.
    fn messages_url(&self) -> String {
        if self.base_url.ends_with("/v1") {
            format!("{}/messages", self.base_url)
        } else {
            format!("{}/v1/messages", self.base_url)
        }
    }

    fn convert(
        &self,
        messages: &[ChatMessage],
        include_thinking: bool,
    ) -> (Option<String>, Vec<Value>) {
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
                    thinking_blocks,
                } => {
                    let mut blocks = Vec::new();
                    // Replay captured thinking blocks FIRST — the API rejects
                    // a thinking-enabled turn whose assistant message starts
                    // with text/tool_use instead of its thinking block.
                    // Signature-less blocks (never completed) are dropped:
                    // they would be rejected as invalid.
                    if include_thinking {
                        for tb in thinking_blocks {
                            let complete = tb.get("type") == Some(&json!("redacted_thinking"))
                                || (tb.get("thinking").is_some()
                                    && tb
                                        .get("signature")
                                        .and_then(|s| s.as_str())
                                        .is_some_and(|s| !s.is_empty()));
                            if complete {
                                blocks.push(tb.clone());
                            }
                        }
                    }
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
        let (system, messages) = self.convert(&request.messages, request.thinking.is_some());
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
        // As in `openai.rs`: the configured temperature is the only one, because the
        // per-request field is gone (`types.rs:ChatRequest` says why). A request that wanted to
        // change it had no way to, and the read implied it could.
        if let Some(t) = self.temperature {
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
            // The API counts thinking tokens toward max_tokens, so the
            // budget must fit underneath it. A per-request cap (e.g. the
            // summarization limit) stays authoritative: shrink the budget to
            // fit rather than silently raising the cap the caller set. With
            // the session default, raise max_tokens to make room for the
            // requested thinking level (previous behavior).
            let budget = match request.max_tokens {
                Some(_) => budget.min(max_tokens.saturating_sub(2048)),
                None => budget,
            };
            body["max_tokens"] = json!(max_tokens.max(budget + 2048));
            // OpenRouter's compat endpoint (verified empirically against
            // stealth/ox-alpha): the anthropic-style thinking block
            // SUPPRESSES this model family's native reasoning, and sending
            // both params yields ZERO thinking. The unified `reasoning`
            // param is the only lever that actually controls reasoning
            // there — so on OpenRouter we send reasoning ONLY.
            if self.base_url.contains("openrouter.ai") {
                let effort = match level {
                    ThinkingLevel::Low => "low",
                    ThinkingLevel::Medium => "medium",
                    _ => "high",
                };
                body["reasoning"] = json!({"effort": effort});
            } else {
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            }
            body.as_object_mut().unwrap().remove("temperature");
        }
        body
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream(&self, request: ChatRequest) -> Result<super::ProviderStream, ProviderError> {
        let url = self.messages_url();
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
            let message = super::sanitize_error_body(&response.text().await.unwrap_or_default());
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message,
            });
        }

        let mut chunks = response.bytes_stream();
        let stream = async_stream::stream! {
            let mut buf = SseLineBuffer::default();
            let mut sse = AnthropicSse::default();

            while let Some(chunk) = chunks.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ProviderError::Http(e));
                        return;
                    }
                };
                // One heartbeat per network chunk, before parsing: bytes on the
                // wire mean the provider is alive even when no complete SSE
                // frame has arrived yet (a slow, giant tool payload streams as
                // many chunks with few parseable deltas).
                if !chunk.is_empty() {
                    yield Ok(ProviderEvent::Activity);
                }
                let lines = match buf.push(&chunk) {
                    Ok(lines) => lines,
                    // Refused at the ceiling, while the bytes are still
                    // arriving, rather than collected until the process cannot
                    // continue: this buffer used to have no ceiling at all and
                    // the heartbeat above re-armed the timer for every chunk of
                    // it.
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                };
                for line in lines {
                    let mut out = Vec::new();
                    let result = sse.on_line(&line, &mut out);
                    for event in out {
                        yield Ok(event);
                    }
                    if let Err(e) = result {
                        yield Err(e);
                        return;
                    }
                }
            }

            // The body ended with bytes still buffered, which means the last
            // frame had no trailing newline. Under Anthropic's dialect that
            // frame is most often `content_block_stop` (the only place a
            // `ToolCall` is emitted) or `message_delta` (the only place a stop
            // reason is), so dropping it lost a whole tool call and then
            // reported `EndTurn` as if the model had finished cleanly. Same
            // handler as a newline-terminated frame, by design: a second copy
            // of this branch is how the two would start disagreeing.
            for line in buf.finish() {
                let mut out = Vec::new();
                let result = sse.on_line(&line, &mut out);
                for event in out {
                    yield Ok(event);
                }
                if let Err(e) = result {
                    yield Err(e);
                    return;
                }
            }

            let mut out = Vec::new();
            sse.finish(&mut out);
            for event in out {
                yield Ok(event);
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
    Thinking {
        text: String,
        signature: String,
        redacted: bool,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
}

/// The per-line half of the Anthropic parser, kept out of the network loop so a
/// response body can be driven through it without a socket.
///
/// `on_line` returns what one frame produced; `Err` is fatal and, like the
/// `return` it replaces, abandons the rest of the body.
#[derive(Default)]
pub(crate) struct AnthropicSse {
    blocks: HashMap<usize, Block>,
    stop_emitted: bool,
}

impl AnthropicSse {
    pub(crate) fn on_line(
        &mut self,
        raw: &[u8],
        out: &mut Vec<ProviderEvent>,
    ) -> Result<(), ProviderError> {
        let line = String::from_utf8_lossy(raw).trim().to_string();
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.trim();
        if data.is_empty() {
            // SSE heartbeat / keep-alive frame; legal, ignore it.
            return Ok(());
        }
        if data == "[DONE]" {
            // OpenAI-style stream sentinel: Anthropic's official API
            // never sends it, but compatible gateways (OpenRouter,
            // ...) terminate their anthropic-flavored streams with
            // it. Skip instead of failing the whole turn.
            return Ok(());
        }
        if self.stop_emitted {
            // Anything after message_stop is trailer noise from
            // compatibility gateways (sentinels, keep-alives,
            // usage pings) — the turn is already complete.
            return Ok(());
        }
        let payload: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                return Err(ProviderError::InvalidResponse(format!(
                    "bad SSE payload: {e}"
                )));
            }
        };
        let event = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match event {
            "content_block_start" => {
                let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let block = payload.get("content_block").cloned().unwrap_or(Value::Null);
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let entry = self
                    .blocks
                    .entry(idx)
                    .or_insert_with(|| Block::Text(String::new()));
                if block_type == "tool_use" {
                    *entry = Block::ToolUse {
                        id: block
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        arguments: String::new(),
                    };
                } else if block_type == "thinking" {
                    *entry = Block::Thinking {
                        text: String::new(),
                        signature: String::new(),
                        redacted: false,
                    };
                } else if block_type == "redacted_thinking" {
                    // Redacted blocks arrive complete (data field,
                    // no deltas) — capture now, no UI deltas.
                    *entry = Block::Thinking {
                        text: block
                            .get("data")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        signature: String::new(),
                        redacted: true,
                    };
                }
            }
            "content_block_delta" => {
                let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let delta = payload.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            match self.blocks.get_mut(&idx) {
                                Some(Block::Text(buf)) => {
                                    buf.push_str(text);
                                }
                                // A protocol-conformant server never
                                // sends text deltas for a tool_use
                                // index; overwriting the accumulator
                                // here would destroy the tool call.
                                None => {
                                    self.blocks.insert(idx, Block::Text(text.to_string()));
                                }
                                Some(Block::ToolUse { .. }) | Some(Block::Thinking { .. }) => {}
                            }
                            out.push(ProviderEvent::Text(text.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(|t| t.as_str()) {
                            if let Some(Block::Thinking { text: buf, .. }) =
                                self.blocks.get_mut(&idx)
                            {
                                buf.push_str(text);
                            }
                            out.push(ProviderEvent::Thinking(text.to_string()));
                        }
                    }
                    Some("signature_delta") => {
                        if let Some(sig) = delta.get("signature").and_then(|t| t.as_str())
                            && let Some(Block::Thinking { signature, .. }) =
                                self.blocks.get_mut(&idx)
                        {
                            signature.push_str(sig);
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) = delta.get("partial_json").and_then(|p| p.as_str())
                            && let Some(Block::ToolUse { arguments, .. }) =
                                self.blocks.get_mut(&idx)
                        {
                            arguments.push_str(partial);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let idx = payload.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                // Remove ONCE and match: a second remove() here
                // would find None (the first already took the
                // value) and silently drop thinking blocks.
                match self.blocks.remove(&idx) {
                    Some(Block::ToolUse {
                        id,
                        name,
                        arguments,
                    }) if !name.is_empty() => {
                        // A gateway omitting the id would round-trip
                        // empty tool_use/tool_result ids, which strict
                        // APIs reject — synthesize a stable one.
                        let id = if id.is_empty() {
                            format!("toolu_synthesized_{idx}")
                        } else {
                            id
                        };
                        out.push(ProviderEvent::ToolCall(ToolCall {
                            id,
                            name,
                            arguments: super::collect_tool_arguments(&arguments),
                        }));
                    }
                    Some(Block::Thinking {
                        text,
                        signature,
                        redacted,
                    }) => {
                        // Complete block: persisted on the assistant
                        // message and replayed on the next request.
                        let block = if redacted {
                            json!({"type": "redacted_thinking", "data": text})
                        } else {
                            json!({
                                "type": "thinking",
                                "thinking": text,
                                "signature": signature,
                            })
                        };
                        out.push(ProviderEvent::ThinkingBlock(block));
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                let reason = payload
                    .pointer("/delta/stop_reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("");
                if !reason.is_empty() && !self.stop_emitted {
                    let reason = match reason {
                        "end_turn" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        "stop_sequence" => StopReason::StopSequence,
                        other => StopReason::Other(other.to_string()),
                    };
                    out.push(ProviderEvent::Stop(reason));
                    self.stop_emitted = true;
                }
            }
            "error" => {
                // Reported mid-stream over a 200 response (capacity,
                // content filter, gateway abort). Falling through the
                // catch-all here used to end the turn as if the model
                // had simply finished.
                let message = payload
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("provider reported an error")
                    .to_string();
                return Err(ProviderError::StreamEnded(message));
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn finish(&mut self, out: &mut Vec<ProviderEvent>) {
        if !self.stop_emitted {
            out.push(ProviderEvent::Stop(StopReason::EndTurn));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;

    #[test]
    fn messages_url_does_not_double_the_v1_prefix() {
        for (base, want) in [
            (
                "https://api.anthropic.com",
                "https://api.anthropic.com/v1/messages",
            ),
            (
                "https://api.anthropic.com/",
                "https://api.anthropic.com/v1/messages",
            ),
            (
                "https://openrouter.ai/api/v1",
                "https://openrouter.ai/api/v1/messages",
            ),
            (
                "https://gw.example.com/v1/",
                "https://gw.example.com/v1/messages",
            ),
        ] {
            let p = AnthropicProvider::new(base, "k", "m", None, None);
            assert_eq!(p.messages_url(), want, "base {base}");
        }
    }

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
                thinking_blocks: Vec::new(),
            },
            ChatMessage::Tool {
                tool_call_id: "call_00".to_string(),
                name: "list_dir".to_string(),
                content: String::new(),
            },
        ];
        let (_, out) = p.convert(&messages, false);
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
                thinking_blocks: Vec::new(),
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
        let (_, out) = p.convert(&messages, false);
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
        let (_, out) = p.convert(&messages, false);
        assert_eq!(
            out.len(),
            2,
            "user text and tool result stay separate: {out:?}"
        );
        assert_eq!(out[1]["content"][0]["type"], "tool_result");
    }
}
