use firment_core::{
    ChatMessage, ChatRequest, Config, OpenAIProvider, Provider, ProviderConfig, ProviderEvent,
    StopReason, ThinkingLevel, load_auth, save_auth,
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[test]
fn compaction_strategy_and_symbols_backend_roundtrip() {
    let text = r#"
compaction_strategy = "drop"

[providers.default]
type = "openai"
model = "x"

[tools]
symbols_backend = "ctags"
build_command = "cmake --build build"
default_chip = "stm32f407vetx"
"#;
    let config: firment_core::Config = toml::from_str(text).unwrap();
    assert_eq!(
        config.compaction_strategy,
        firment_core::CompactionStrategy::Drop
    );
    assert_eq!(config.tools.symbols_backend.as_deref(), Some("ctags"));
    assert_eq!(
        config.tools.build_command.as_deref(),
        Some("cmake --build build")
    );
    assert_eq!(config.tools.default_chip.as_deref(), Some("stm32f407vetx"));

    let empty: firment_core::Config =
        toml::from_str("[providers.default]\ntype=\"openai\"\nmodel=\"x\"\n").unwrap();
    assert_eq!(
        empty.compaction_strategy,
        firment_core::CompactionStrategy::Summarize
    );
    assert!(empty.tools.symbols_backend.is_none());
}

#[test]
fn project_config_merges_tools_over_global() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".firment.toml"),
        "[tools]\nbuild_command = \"make\"\ndefault_chip = \"stm32f103c8\"\n",
    )
    .unwrap();
    let global: firment_core::Config =
        toml::from_str("[providers.default]\ntype=\"openai\"\nmodel=\"x\"\n").unwrap();
    let merged = global.merged_for(dir.path());
    assert_eq!(merged.tools.build_command.as_deref(), Some("make"));
    assert_eq!(merged.tools.default_chip.as_deref(), Some("stm32f103c8"));
    assert!(merged.tools.monitor_port.is_none());

    let nested = dir.path().join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let merged2 = global.merged_for(&nested);
    assert_eq!(merged2.tools.build_command.as_deref(), Some("make"));
}

#[test]
fn default_config_auto_approves_build() {
    let config = firment_core::Config::default_config();
    assert!(config.auto_approve.iter().any(|t| t == "build"));
}

#[test]
fn config_save_load_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut config = Config::default_config();
    config.default_provider = "deepseek".to_string();
    config.thinking = ThinkingLevel::Max;
    config
        .providers
        .insert("deepseek".to_string(), config.providers["default"].clone());
    config.providers.remove("default");
    config.save(&path).unwrap();

    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.default_provider, "deepseek");
    assert_eq!(loaded.thinking, ThinkingLevel::Max);
    assert_eq!(loaded.providers["deepseek"].model, "deepseek-v4-flash");
}

#[test]
fn load_or_create_migrates_deprecated_deepseek_models() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"
[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-reasoner"
"#,
    )
    .unwrap();

    let config = Config::load_or_create(&path).unwrap();
    assert_eq!(config.providers["default"].model, "deepseek-v4-flash");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("deepseek-v4-flash"));
}

#[test]
fn auth_roundtrip_and_resolution_order() {
    let dir = tempdir().unwrap();
    unsafe { std::env::set_var("FIRMENT_CONFIG_DIR", dir.path()) };
    let config = Config::default_config();

    // 1. nothing set -> None
    assert!(
        config
            .api_key_for(&config.providers["default"], "default")
            .is_none()
    );

    // 2. auth.json fallback
    config.set_api_key("default", "sk-auth").unwrap();
    assert_eq!(
        config
            .api_key_for(&config.providers["default"], "default")
            .as_deref(),
        Some("sk-auth")
    );

    // 3. inline api_key wins over auth
    let mut inline = config.providers["default"].clone();
    inline.api_key = Some("sk-inline".to_string());
    assert_eq!(
        config.api_key_for(&inline, "default").as_deref(),
        Some("sk-inline")
    );

    // 4. env var is the last fallback
    let mut auth = load_auth();
    auth.remove("default");
    save_auth(&auth).unwrap();
    unsafe { std::env::set_var("DEEPSEEK_TEST_KEY", "sk-env") };
    let mut env_provider = config.providers["default"].clone();
    env_provider.api_key = None;
    env_provider.api_key_env = Some("DEEPSEEK_TEST_KEY".to_string());
    assert_eq!(
        config.api_key_for(&env_provider, "default").as_deref(),
        Some("sk-env")
    );

    unsafe {
        std::env::remove_var("FIRMENT_CONFIG_DIR");
        std::env::remove_var("DEEPSEEK_TEST_KEY");
    }
}

struct CaptureResponder {
    captured: Arc<Mutex<Vec<Value>>>,
}

impl Respond for CaptureResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        self.captured.lock().unwrap().push(body);
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream")
    }
}

async fn capture_deepseek_body(thinking: ThinkingLevel) -> Value {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CaptureResponder {
            captured: captured.clone(),
        })
        .mount(&server)
        .await;

    // vendor detection kicks in from the model name even with a generic base URL
    let provider = OpenAIProvider::new(server.uri(), "test-key", "deepseek-v4-flash", None, None);
    let request = ChatRequest {
        model: "deepseek-v4-flash".to_string(),
        messages: vec![ChatMessage::User {
            content: "hi".to_string(),
        }],
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        thinking: Some(thinking),
    };
    let mut stream = provider.stream(request).await.unwrap();
    while let Some(event) = stream.next().await {
        if let ProviderEvent::Stop(StopReason::EndTurn) = event.unwrap() {
            break;
        }
    }
    captured.lock().unwrap()[0].clone()
}

#[tokio::test]
async fn deepseek_thinking_uses_thinking_param_and_effort_mapping() {
    let high = capture_deepseek_body(ThinkingLevel::High).await;
    assert_eq!(high["thinking"], json!({"type": "enabled"}));
    assert_eq!(high["reasoning_effort"], json!("high"));

    let max = capture_deepseek_body(ThinkingLevel::Max).await;
    assert_eq!(max["thinking"], json!({"type": "enabled"}));
    assert_eq!(max["reasoning_effort"], json!("max"));

    let xhigh = capture_deepseek_body(ThinkingLevel::XHigh).await;
    assert_eq!(xhigh["reasoning_effort"], json!("max"));

    let low = capture_deepseek_body(ThinkingLevel::Low).await;
    assert_eq!(low["reasoning_effort"], json!("high"));
}

#[tokio::test]
async fn generic_openai_reasoning_effort_passes_through() {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CaptureResponder {
            captured: captured.clone(),
        })
        .mount(&server)
        .await;

    let provider = OpenAIProvider::new(server.uri(), "test-key", "gpt-test", None, None);
    let request = ChatRequest {
        model: "gpt-test".to_string(),
        messages: vec![ChatMessage::User {
            content: "hi".to_string(),
        }],
        tools: Vec::new(),
        max_tokens: None,
        temperature: None,
        thinking: Some(ThinkingLevel::XHigh),
    };
    let mut stream = provider.stream(request).await.unwrap();
    while let Some(event) = stream.next().await {
        if let ProviderEvent::Stop(StopReason::EndTurn) = event.unwrap() {
            break;
        }
    }
    let body = captured.lock().unwrap()[0].clone();
    assert_eq!(body["reasoning_effort"], json!("xhigh"));
    assert!(body.get("thinking").is_none());
}

#[tokio::test]
async fn list_models_fetches_from_models_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"id": "deepseek-v4-pro"},
                {"id": "deepseek-v4-flash"},
            ]
        })))
        .mount(&server)
        .await;

    let config = Config::default_with_provider(
        "default",
        ProviderConfig {
            r#type: "openai".to_string(),
            base_url: Some(server.uri()),
            api_key_env: None,
            api_key: Some("test-key".to_string()),
            model: "deepseek-v4-flash".to_string(),
            max_tokens: None,
            temperature: None,
        },
    );
    let models = config.list_models("default").await.unwrap();
    assert_eq!(models, vec!["deepseek-v4-flash", "deepseek-v4-pro"]);
    assert!(models.len() == 2);
}
