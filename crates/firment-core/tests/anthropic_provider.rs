use firment_core::{
    AnthropicProvider, ChatMessage, ChatRequest, Provider, ProviderEvent, StopReason,
};
use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(payloads: &[&str]) -> String {
    payloads.iter().map(|p| format!("data: {p}\n\n")).collect()
}

#[tokio::test]
async fn anthropic_stream_parses_text_and_tool_use() {
    let server = MockServer::start().await;
    let body = sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"echo","input":{}}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"message\":\"hi\"}"}}"#,
        r#"{"type":"content_block_stop","index":1}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(server.uri(), "test-key", "anthropic-test", None, None);
    let request = ChatRequest {
        model: "anthropic-test".to_string(),
        messages: vec![ChatMessage::User {
            content: "hi".to_string(),
        }],
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        thinking: None,
    };
    let mut stream = provider.stream(request).await.unwrap();
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    let mut stop = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ProviderEvent::Text(t) => texts.push(t),
            ProviderEvent::ToolCall(call) => calls.push(call),
            ProviderEvent::Stop(reason) => stop = Some(reason),
        }
    }
    assert_eq!(texts, vec!["Hello"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "tu_1");
    assert_eq!(calls[0].name, "echo");
    assert_eq!(calls[0].arguments, serde_json::json!({"message": "hi"}));
    assert_eq!(stop, Some(StopReason::ToolUse));
}

#[tokio::test]
async fn anthropic_stream_tolerates_openrouter_trailers() {
    // OpenRouter's anthropic-compatible endpoint terminates the stream with
    // the OpenAI-style `data: [DONE]` sentinel (the official API never sends
    // it) — the parser must skip it instead of failing the whole turn.
    let server = MockServer::start().await;
    let body = format!(
        "{}data: [DONE]\n\ndata: {{\"garbage\":true}}\n\n",
        sse(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ])
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(server.uri(), "test-key", "anthropic-test", None, None);
    let request = ChatRequest {
        model: "anthropic-test".to_string(),
        messages: vec![ChatMessage::User {
            content: "hi".to_string(),
        }],
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        thinking: None,
    };
    let mut stream = provider.stream(request).await.unwrap();
    let mut texts = Vec::new();
    let mut stop = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            ProviderEvent::Text(t) => texts.push(t),
            ProviderEvent::ToolCall(_) => panic!("no tool calls expected"),
            ProviderEvent::Stop(reason) => stop = Some(reason),
        }
    }
    assert_eq!(texts, vec!["Hi"]);
    assert_eq!(stop, Some(StopReason::EndTurn));
}
