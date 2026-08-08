use async_trait::async_trait;
use firment_core::{
    Agent, AgentError, AgentEvent, AutoApprove, ChatMessage, ChatRequest, EventSink,
    PlanModePermission, Provider, ProviderError, ProviderEvent, ProviderStream, Session,
    SessionMode, SessionStore, StopReason, Tool, ToolContext, ToolError, ToolOutput, ToolRegistry,
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Clone)]
struct FakeProvider {
    queue: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    model: String,
}

#[async_trait]
impl Provider for FakeProvider {
    async fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        let events = self.queue.lock().unwrap().pop_front().unwrap_or_default();
        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }

    fn model(&self) -> &str {
        &self.model
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        "echo"
    }

    fn description(&self) -> &'static str {
        "echo a message back"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {"message": {"type": "string"}}})
    }

    async fn run(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            text: format!(
                "echo: {}",
                args.get("message").and_then(|m| m.as_str()).unwrap_or("")
            ),
        })
    }
}

struct GuardedTool;

#[async_trait]
impl Tool for GuardedTool {
    fn name(&self) -> &'static str {
        "guarded"
    }

    fn description(&self) -> &'static str {
        "tool that requires approval"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("guarded operation".to_string())
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            text: "guarded ran".to_string(),
        })
    }
}

struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "fake write tool"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("write file".to_string())
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            text: "wrote".to_string(),
        })
    }
}

struct JournalingWriteTool;

#[async_trait]
impl Tool for JournalingWriteTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "fake write tool that journals"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("write file".to_string())
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("out.txt");
        let content = args.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let resolved = ctx.cwd.join(path);
        ctx.journal
            .lock()
            .unwrap()
            .begin(&resolved)
            .map_err(ToolError::new)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError::new(e.to_string()))?;
        }
        std::fs::write(&resolved, content).map_err(|e| ToolError::new(e.to_string()))?;
        Ok(ToolOutput {
            text: "wrote".to_string(),
        })
    }
}

struct FailingEditTool;

#[async_trait]
impl Tool for FailingEditTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "fake edit tool that always fails"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    fn approval(&self, _args: &Value) -> Option<String> {
        Some("edit file".to_string())
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Err(ToolError::new("anchor mismatch (fake failure)"))
    }
}

struct LongOutputTool;

#[async_trait]
impl Tool for LongOutputTool {
    fn name(&self) -> &'static str {
        "long"
    }

    fn description(&self) -> &'static str {
        "returns very long output"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            text: "x".repeat(10_000),
        })
    }
}

struct FlagTool {
    ran: Arc<AtomicBool>,
}

#[async_trait]
impl Tool for FlagTool {
    fn name(&self) -> &'static str {
        "flag"
    }

    fn description(&self) -> &'static str {
        "sets a flag when actually run"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"need": {"type": "string"}},
            "required": ["need"]
        })
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        self.ran.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            text: "ran".to_string(),
        })
    }
}

struct FakeVerifyTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for FakeVerifyTool {
    fn name(&self) -> &'static str {
        "verify"
    }

    fn description(&self) -> &'static str {
        "fake verify tool"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput {
            text: "verify passed (exit 0)".to_string(),
        })
    }
}

struct FailingVerifyTool;

#[async_trait]
impl Tool for FailingVerifyTool {
    fn name(&self) -> &'static str {
        "verify"
    }

    fn description(&self) -> &'static str {
        "fake verify tool that always fails"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        Err(ToolError::new("[CompileError] verify failed (exit 3)"))
    }
}

struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "fake read tool"
    }

    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("out.txt");
        let resolved = ctx.cwd.join(path);
        match std::fs::read_to_string(&resolved) {
            Ok(text) => Ok(ToolOutput { text }),
            Err(e) => Err(ToolError::new(format!("[NotFound] {e}"))),
        }
    }
}

struct RecordingProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl Provider for RecordingProvider {
    async fn stream(&self, request: ChatRequest) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(futures::stream::iter(vec![Ok(
            ProviderEvent::Stop(StopReason::EndTurn),
        )])))
    }

    fn model(&self) -> &str {
        "fake"
    }
}

struct CollectSink(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait]
impl EventSink for CollectSink {
    async fn event(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn registry_with(tools: Vec<Arc<dyn Tool>>) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    Arc::new(registry)
}

#[tokio::test]
async fn session_roundtrip() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "test-model");
    session.push(ChatMessage::User {
        content: "hello".to_string(),
    });
    session.push(ChatMessage::Assistant {
        content: "hi".to_string(),
        tool_calls: vec![firment_core::ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: json!({"message": "hi"}),
        }],
    });
    store.save(&session).unwrap();

    let loaded = store.load(&session.id).unwrap();
    assert_eq!(loaded.id, session.id);
    assert_eq!(loaded.messages, session.messages);
    assert_eq!(loaded.model, "test-model");

    let list = store.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, session.id);
}

#[test]
fn session_load_migrates_deprecated_deepseek_model() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let id = "legacy-session";
    let path = store.path_for(id);
    std::fs::write(
        &path,
        format!(
            "{{\"type\":\"meta\",\"id\":\"{id}\",\"cwd\":\".\",\"provider\":\"default\",\"model\":\"deepseek-chat\",\"thinking\":\"off\",\"created_at\":0,\"updated_at\":0}}\n"
        ),
    )
    .unwrap();

    let session = store.load(id).unwrap();
    assert_eq!(session.model, "deepseek-v4-flash");
    assert_eq!(session.mode, SessionMode::Agent);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("deepseek-v4-flash"));
    assert!(!text.contains("deepseek-chat"));
}

#[test]
fn session_mode_roundtrip_with_plan() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    session.mode = SessionMode::Plan;
    session.push(ChatMessage::User {
        content: "hi".to_string(),
    });
    store.save(&session).unwrap();

    let loaded = store.load(&session.id).unwrap();
    assert_eq!(loaded.mode, SessionMode::Plan);
    assert_eq!(loaded.messages, session.messages);
}

#[test]
fn system_prompt_covers_core_guidance_and_plan_contract() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Use this board's HAL layer for all GPIO access.\n",
    )
    .unwrap();
    let prompt = firment_core::default_system_prompt(dir.path());
    for needle in [
        "Firment",
        "Working directory",
        "read_file",
        "edit_file",
        "shell",
        "path:line",
        "AGENTS.md",
        "verify",
        "Report outcomes faithfully",
        "Use this board's HAL layer",
    ] {
        assert!(prompt.contains(needle), "default prompt missing: {needle}");
    }

    let plan_prompt = firment_core::system_prompt_for(dir.path(), SessionMode::Plan);
    assert!(plan_prompt.contains("PLAN mode (read-only)"));
    assert!(plan_prompt.contains("decision-complete"));
    assert!(plan_prompt.contains("MUST NOT write"));
}

#[tokio::test]
async fn agent_loop_runs_tools() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::Text("Checking…".to_string()),
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: json!({"message": "hi"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("Done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(EchoTool)]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(events.clone())),
        10,
    );

    let text = agent.run_turn("echo hi").await.unwrap();
    assert_eq!(text, "Done");

    let collected = events.lock().unwrap();
    assert!(
        collected
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolStart { name, .. } if name == "echo"))
    );
    assert!(
        collected
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolEnd { ok: true, .. }))
    );

    let roles: Vec<&str> = agent
        .session()
        .messages
        .iter()
        .map(|m| match m {
            ChatMessage::User { .. } => "user",
            ChatMessage::Assistant { .. } => "assistant",
            ChatMessage::Tool { .. } => "tool",
            ChatMessage::System { .. } => "system",
        })
        .collect();
    assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
}

#[tokio::test]
async fn permission_denied_is_reported_to_model() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_1".to_string(),
                    name: "guarded".to_string(),
                    arguments: json!({}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("ok".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(GuardedTool)]),
        session,
        store,
        Arc::new(AutoApprove::nothing()),
        Arc::new(CollectSink(events)),
        10,
    );

    let text = agent.run_turn("go").await.unwrap();
    assert_eq!(text, "ok");
    let tool_message = agent
        .session()
        .messages
        .iter()
        .find_map(|m| match m {
            ChatMessage::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert!(tool_message.starts_with("Permission denied"));
}

#[tokio::test]
async fn plan_mode_permission_hard_denies_mutating_tools() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_1".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "x"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("ok".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    session.mode = SessionMode::Plan;
    let permission = Arc::new(PlanModePermission::new(Arc::new(AutoApprove::everything())));
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(WriteTool)]),
        session,
        store,
        permission,
        Arc::new(CollectSink(events)),
        10,
    );

    let text = agent.run_turn("write").await.unwrap();
    assert_eq!(text, "ok");
    let tool_message = agent
        .session()
        .messages
        .iter()
        .find_map(|m| match m {
            ChatMessage::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert!(tool_message.contains("plan mode"));
    assert!(tool_message.starts_with("Permission denied"));
}

#[tokio::test]
async fn plan_mode_injects_read_only_system_prompt() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = RecordingProvider {
        requests: requests.clone(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    session.mode = SessionMode::Plan;
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(Vec::new()),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("plan something").await;
    let requests = requests.lock().unwrap();
    let first = &requests[0].messages[0];
    match first {
        ChatMessage::System { content } => {
            assert!(content.contains("PLAN mode"));
            assert!(content.contains("read-only"));
        }
        _ => panic!("expected a system message"),
    }
}

#[tokio::test]
async fn switching_back_to_agent_mode_restores_mutating_tools_and_prompt() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = RecordingProvider {
        requests: requests.clone(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    session.mode = SessionMode::Plan;
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(EchoTool)]),
        session,
        store,
        Arc::new(PlanModePermission::new(Arc::new(AutoApprove::everything()))),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    agent.set_mode(
        SessionMode::Agent,
        registry_with(vec![Arc::new(EchoTool), Arc::new(WriteTool)]),
        Arc::new(AutoApprove::everything()),
    );
    let _ = agent.run_turn("back to normal").await;

    let requests = requests.lock().unwrap();
    let request = &requests[0];
    assert!(request.tools.iter().any(|t| t.name == "write_file"));
    match &request.messages[0] {
        ChatMessage::System { content } => {
            assert!(!content.contains("PLAN mode"));
        }
        _ => panic!("expected a system message"),
    }
}

#[tokio::test]
async fn max_iterations_stops() {
    let tool_call = vec![
        ProviderEvent::ToolCall(firment_core::ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: json!({"message": "loop"}),
        }),
        ProviderEvent::Stop(StopReason::ToolUse),
    ];
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            tool_call.clone(),
            tool_call.clone(),
        ]))),
        model: "fake".to_string(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(EchoTool)]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(events)),
        2,
    );

    let err = agent.run_turn("loop").await.unwrap_err();
    assert!(matches!(err, AgentError::MaxIterations(2)));
}

#[tokio::test]
async fn agent_without_provider_reports_clear_error() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        None,
        registry_with(vec![Arc::new(EchoTool)]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let err = agent.run_turn("hi").await.unwrap_err();
    assert!(matches!(err, AgentError::NoProvider));
}

#[tokio::test]
async fn long_tool_output_is_spilled_to_disk() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_long".to_string(),
                    name: "long".to_string(),
                    arguments: json!({}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(LongOutputTool)]),
        session,
        store.clone(),
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("long").await.unwrap();
    let tool_message = agent
        .session()
        .messages
        .iter()
        .find_map(|m| match m {
            ChatMessage::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert!(tool_message.contains("已外溢到"), "got: {tool_message}");
    let spill_dir = store.spill_dir(&agent.session().id);
    let spilled = std::fs::read_dir(&spill_dir).unwrap().count();
    assert_eq!(spilled, 1);
    let file = std::fs::read_dir(&spill_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(file.path()).unwrap().len(), 10_000);
}

#[tokio::test]
async fn ledger_records_changes_and_is_injected_into_prompt() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "c.txt", "content": "v1"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(JournalingWriteTool)]),
        session,
        store.clone(),
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("write c").await.unwrap();
    let ledger_text = std::fs::read_to_string(store.ledger_path(&agent.session().id)).unwrap();
    assert!(ledger_text.contains("c.txt"), "got: {ledger_text}");

    let requests = Arc::new(Mutex::new(Vec::new()));
    agent.set_provider(Box::new(RecordingProvider {
        requests: requests.clone(),
    }));
    let _ = agent.run_turn("second").await.unwrap();
    let requests = requests.lock().unwrap();
    // System prompt stays byte-stable (cache-friendly): the ledger delta must
    // be merged into the turn's user message instead.
    match &requests[0].messages[0] {
        ChatMessage::System { content } => {
            assert!(
                !content.contains("本会话改动台账"),
                "ledger leaked into system prompt"
            );
        }
        _ => panic!("expected a system message"),
    }
    let user_texts: Vec<&str> = requests[0]
        .messages
        .iter()
        .filter_map(|m| match m {
            ChatMessage::User { content } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        user_texts
            .iter()
            .any(|c| c.contains("[最近改动台账]") && c.contains("c.txt")),
        "ledger delta missing from user message, got: {user_texts:?}"
    );
}

#[tokio::test]
async fn model_based_compaction_uses_provider_summary() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::Text("MODEL SUMMARY CONTENT".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
            vec![
                ProviderEvent::Text("done2".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    for i in 0..14 {
        session.push(ChatMessage::User {
            content: format!("message {i} {}", "x".repeat(200)),
        });
    }
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(Vec::new()),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );
    agent.set_context_budget_chars(1000);

    let _ = agent.run_turn("final").await.unwrap();
    let first = &agent.session().messages[0];
    match first {
        ChatMessage::User { content } => {
            assert!(content.contains("MODEL SUMMARY CONTENT"), "got: {content}");
            assert!(content.contains("[对话已压缩]"), "got: {content}");
        }
        _ => panic!("expected a compaction summary message"),
    }
}

#[tokio::test]
async fn duplicate_read_results_are_stubbed() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_r1".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "a.txt"}),
                }),
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_r2".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "a.txt"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(ReadFileTool)]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("read twice").await.unwrap();
    let tool_texts: Vec<&str> = agent
        .session()
        .messages
        .iter()
        .filter_map(|m| match m {
            ChatMessage::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tool_texts.len(), 2);
    assert!(
        tool_texts[0].contains("hello world"),
        "got: {}",
        tool_texts[0]
    );
    assert!(
        tool_texts[1].contains("[文件未变化"),
        "second read should be stubbed, got: {}",
        tool_texts[1]
    );
}

#[tokio::test]
async fn verify_hard_gate_runs_after_mutations_and_allows_completion() {
    let verify_calls = Arc::new(AtomicUsize::new(0));
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "a.txt", "content": "x"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![
            Arc::new(JournalingWriteTool),
            Arc::new(FakeVerifyTool {
                calls: verify_calls.clone(),
            }),
        ]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );
    agent.set_verify_command(Some("fake-verify".to_string()));

    let text = agent.run_turn("write then finish").await.unwrap();
    assert_eq!(text, "done");
    assert_eq!(
        verify_calls.load(Ordering::SeqCst),
        1,
        "hard gate must run verify once"
    );
    assert!(
        dir.path().join("a.txt").exists(),
        "mutation must be committed"
    );
    assert!(
        matches!(agent.session().messages.last(), Some(ChatMessage::Assistant { content, .. }) if content == "done")
    );
}

#[tokio::test]
async fn verify_hard_gate_blocks_completion_on_failure() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "b.txt", "content": "x"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![
            Arc::new(JournalingWriteTool),
            Arc::new(FailingVerifyTool),
        ]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        3,
    );
    agent.set_verify_command(Some("fake-verify".to_string()));

    let err = agent.run_turn("write then finish").await.unwrap_err();
    assert!(matches!(err, AgentError::MaxIterations(3)));
    assert!(
        agent.session().messages.iter().any(|m| matches!(
            m,
            ChatMessage::Tool { content, .. } if content.contains("[CompileError]")
        )),
        "verify failure must be visible to the model"
    );
    assert!(
        !dir.path().join("b.txt").exists(),
        "unverified mutation must be rolled back"
    );
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_tool_runs() {
    let ran = Arc::new(AtomicBool::new(false));
    let registry = registry_with(vec![Arc::new(FlagTool { ran: ran.clone() })]);
    let dir = tempdir().unwrap();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        permission: Arc::new(AutoApprove::everything()),
        allow_dangerous: false,
        journal: Arc::new(Mutex::new(firment_core::EditJournal::new(
            dir.path().join("undo"),
        ))),
        verify_command: None,
        allowed_roots: Vec::new(),
    };
    let err = registry.run("flag", json!({}), &ctx).await.unwrap_err();
    assert!(err.message.contains("参数校验失败"), "got: {}", err.message);
    assert!(
        !ran.load(Ordering::SeqCst),
        "tool must not run on invalid args"
    );
}

#[tokio::test]
async fn parallel_calls_are_ordered_by_file_dependency() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_write".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "a.txt", "content": "hello"}),
                }),
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_read".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "a.txt"}),
                }),
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_echo".to_string(),
                    name: "echo".to_string(),
                    arguments: json!({"message": "parallel"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![
            Arc::new(JournalingWriteTool),
            Arc::new(ReadFileTool),
            Arc::new(EchoTool),
        ]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("write then read").await.unwrap();
    let tool_messages: Vec<(&str, &str)> = agent
        .session()
        .messages
        .iter()
        .filter_map(|m| match m {
            ChatMessage::Tool { name, content, .. } => Some((name.as_str(), content.as_str())),
            _ => None,
        })
        .collect();
    let names: Vec<&str> = tool_messages.iter().map(|(n, _)| *n).collect();
    // read depends on the write to the same file; echo is independent and
    // runs in the same wave as the write.
    assert_eq!(
        names,
        vec!["write_file", "echo", "read_file"],
        "got: {names:?}"
    );
    assert!(tool_messages[2].1.contains("hello"));
}

#[tokio::test]
async fn context_compaction_replaces_old_messages_with_digest() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = RecordingProvider {
        requests: requests.clone(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let mut session = Session::new(dir.path().to_path_buf(), "default", "fake");
    for i in 0..14 {
        session.push(ChatMessage::User {
            content: format!("message {i} {}", "x".repeat(200)),
        });
    }
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(Vec::new()),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );
    agent.set_context_budget_chars(1000);

    let _ = agent.run_turn("hi").await.unwrap();
    let requests = requests.lock().unwrap();
    // requests[0] is the summarization call; the main request follows it.
    assert!(requests.len() >= 2, "expected summary + main requests");
    let messages = &requests[1].messages;
    assert!(
        messages.iter().any(
            |m| matches!(m, ChatMessage::User { content } if content.contains("[对话已压缩]"))
        ),
        "expected a compaction marker"
    );
    assert!(messages.len() <= 12, "got {} messages", messages.len());
    assert!(matches!(messages.last(), Some(ChatMessage::User { content }) if content == "hi"));
}

#[tokio::test]
async fn mutation_batch_rolls_back_when_a_later_edit_fails() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_1".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "a.txt", "content": "hello"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_2".to_string(),
                    name: "edit_file".to_string(),
                    arguments: json!({}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![
            Arc::new(JournalingWriteTool),
            Arc::new(FailingEditTool),
        ]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(events.clone())),
        10,
    );

    let text = agent.run_turn("write then edit").await.unwrap();
    assert_eq!(text, "done");
    assert!(
        !dir.path().join("a.txt").exists(),
        "created file must be rolled back"
    );
    let collected = events.lock().unwrap();
    assert!(
        collected
            .iter()
            .any(|e| matches!(e, AgentEvent::Info(m) if m.contains("已回滚"))),
        "expected a rollback info event"
    );
}

#[tokio::test]
async fn max_iterations_rolls_back_writes() {
    let call = vec![
        ProviderEvent::ToolCall(firment_core::ToolCall {
            id: "call_1".to_string(),
            name: "write_file".to_string(),
            arguments: json!({"path": "b.txt", "content": "x"}),
        }),
        ProviderEvent::Stop(StopReason::ToolUse),
    ];
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([call.clone(), call.clone()]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(JournalingWriteTool)]),
        session,
        store,
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        2,
    );

    let err = agent.run_turn("loop write").await.unwrap_err();
    assert!(matches!(err, AgentError::MaxIterations(2)));
    assert!(!dir.path().join("b.txt").exists());
}

#[tokio::test]
async fn committed_turn_can_be_undone() {
    let provider = FakeProvider {
        queue: Arc::new(Mutex::new(VecDeque::from([
            vec![
                ProviderEvent::ToolCall(firment_core::ToolCall {
                    id: "call_1".to_string(),
                    name: "write_file".to_string(),
                    arguments: json!({"path": "c.txt", "content": "v1"}),
                }),
                ProviderEvent::Stop(StopReason::ToolUse),
            ],
            vec![
                ProviderEvent::Text("done".to_string()),
                ProviderEvent::Stop(StopReason::EndTurn),
            ],
        ]))),
        model: "fake".to_string(),
    };
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf());
    let session = Session::new(dir.path().to_path_buf(), "default", "fake");
    let mut agent = Agent::new(
        Some(Box::new(provider)),
        registry_with(vec![Arc::new(JournalingWriteTool)]),
        session,
        store.clone(),
        Arc::new(AutoApprove::everything()),
        Arc::new(CollectSink(Arc::new(Mutex::new(Vec::new())))),
        10,
    );

    let _ = agent.run_turn("write c").await.unwrap();
    let file = dir.path().join("c.txt");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1");

    let summary = agent.undo_last().await.unwrap();
    assert!(summary.contains("已恢复 1 个文件"));
    assert!(!file.exists());
}
