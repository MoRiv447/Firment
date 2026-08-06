use async_trait::async_trait;
use firment_core::{
    Agent, AgentEvent, AutoApprove, Config, EventSink, PlanModePermission, ProviderConfig, Session,
    SessionMode, SessionStore,
};
use firment_tools::{default_registry, plan_registry};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct SequenceResponder {
    queue: Arc<Mutex<VecDeque<ResponseTemplate>>>,
}

impl Respond for SequenceResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let mut queue = self.queue.lock().unwrap();
        if queue.len() == 1 {
            queue.front().unwrap().clone()
        } else {
            queue.pop_front().unwrap()
        }
    }
}

struct Sink;

#[async_trait]
impl EventSink for Sink {
    async fn event(&self, _event: AgentEvent) {}
}

fn sse(payloads: &[Value]) -> String {
    payloads.iter().map(|p| format!("data: {p}\n\n")).collect()
}

fn openai_chunk(delta: Value, finish: Option<&str>) -> Value {
    let mut chunk = json!({"choices": [{"delta": delta, "index": 0}]});
    if let Some(finish) = finish {
        chunk["choices"][0]["finish_reason"] = json!(finish);
    }
    chunk
}

#[tokio::test]
async fn openai_compatible_end_to_end_read_edit_verify() {
    let server = MockServer::start().await;
    let first_turn = vec![
        openai_chunk(json!({"content": "I'll read the file first.\n"}), None),
        openai_chunk(
            json!({"tool_calls": [{
                "index": 0,
                "id": "call_read",
                "function": {"name": "read_file", "arguments": "{\"path\":\"sample.txt\"}"}
            }]}),
            Some("tool_calls"),
        ),
    ];
    let second_turn = vec![openai_chunk(
        json!({"tool_calls": [{
            "index": 0,
            "id": "call_edit",
            "function": {"name": "edit_file", "arguments": "{\"path\":\"sample.txt\",\"old_text\":\"hello\",\"new_text\":\"hello base\"}"}
        }]}),
        Some("tool_calls"),
    )];
    let third_turn = vec![
        openai_chunk(json!({"content": "Done, file edited."}), None),
        openai_chunk(json!({}), Some("stop")),
    ];
    let queue = Arc::new(Mutex::new(VecDeque::from([
        ResponseTemplate::new(200).set_body_raw(sse(&first_turn), "text/event-stream"),
        ResponseTemplate::new(200).set_body_raw(sse(&second_turn), "text/event-stream"),
        ResponseTemplate::new(200).set_body_raw(sse(&third_turn), "text/event-stream"),
    ])));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(SequenceResponder { queue })
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let file = dir.path().join("sample.txt");
    std::fs::write(&file, "hello\nworld\n").unwrap();

    let config = Config::default_with_provider(
        "default",
        ProviderConfig {
            r#type: "openai".to_string(),
            base_url: Some(server.uri()),
            api_key_env: None,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            max_tokens: None,
            temperature: None,
        },
    );
    let provider = config
        .build_provider(Some("default"), Some("test-model"))
        .unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "test-model");
    let mut agent = Agent::new(
        Some(provider),
        default_registry(),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(Sink),
        10,
    );

    let text = agent
        .run_turn("read sample.txt and replace hello with hello base")
        .await
        .unwrap();
    assert_eq!(text, "Done, file edited.");
    let content = std::fs::read_to_string(&file).unwrap();
    assert!(content.starts_with("hello base\n"));
    assert!(content.contains("world"));
}

#[tokio::test]
async fn plan_mode_end_to_end_only_runs_read_tools() {
    let server = MockServer::start().await;
    let read_turn = vec![openai_chunk(
        json!({"tool_calls": [{
            "index": 0,
            "id": "call_read",
            "function": {"name": "read_file", "arguments": "{\"path\":\"sample.txt\"}"}
        }]}),
        Some("tool_calls"),
    )];
    let write_turn = vec![openai_chunk(
        json!({"tool_calls": [{
            "index": 0,
            "id": "call_write",
            "function": {"name": "write_file", "arguments": "{\"path\":\"sample.txt\",\"content\":\"x\"}"}
        }]}),
        Some("tool_calls"),
    )];
    let final_turn = vec![
        openai_chunk(json!({"content": "Plan ready."}), None),
        openai_chunk(json!({}), Some("stop")),
    ];
    let queue = Arc::new(Mutex::new(VecDeque::from([
        ResponseTemplate::new(200).set_body_raw(sse(&read_turn), "text/event-stream"),
        ResponseTemplate::new(200).set_body_raw(sse(&write_turn), "text/event-stream"),
        ResponseTemplate::new(200).set_body_raw(sse(&final_turn), "text/event-stream"),
    ])));
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(SequenceResponder { queue })
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let file = dir.path().join("sample.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let config = Config::default_with_provider(
        "default",
        ProviderConfig {
            r#type: "openai".to_string(),
            base_url: Some(server.uri()),
            api_key_env: None,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            max_tokens: None,
            temperature: None,
        },
    );
    let provider = config
        .build_provider(Some("default"), Some("test-model"))
        .unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let mut session = Session::new(dir.path().to_path_buf(), "default", "test-model");
    session.mode = SessionMode::Plan;
    let mut agent = Agent::new(
        Some(provider),
        plan_registry(),
        session,
        store,
        Arc::new(PlanModePermission::new(Arc::new(AutoApprove::everything()))),
        Arc::new(Sink),
        10,
    );

    let text = agent
        .run_turn("调研 sample.txt 并给出修改计划")
        .await
        .unwrap();
    assert_eq!(text, "Plan ready.");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello\n");

    let write_result = agent
        .session()
        .messages
        .iter()
        .find_map(|m| match m {
            firment_core::ChatMessage::Tool { name, content, .. } if name == "write_file" => {
                Some(content.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(
        write_result.contains("unknown tool: write_file") || write_result.contains("plan mode"),
        "unexpected write result: {write_result}"
    );
}
