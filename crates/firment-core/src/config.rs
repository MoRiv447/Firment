use crate::ThinkingLevel;
use crate::provider::{AnthropicProvider, OpenAIProvider, Provider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default = "default_provider_name")]
    pub default_provider: String,
    #[serde(default)]
    pub auto_approve: Vec<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default)]
    pub thinking: ThinkingLevel,
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Approximate character budget for session context; older messages are
    /// compacted into a digest when exceeded.
    #[serde(default = "default_context_budget")]
    pub context_budget_chars: usize,
    /// Auto-compaction strategy (see `CompactionStrategy`).
    #[serde(default)]
    pub compaction_strategy: CompactionStrategy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Command run by the `verify` tool (platform shell), e.g. `cargo check`.
    #[serde(default)]
    pub verify_command: Option<String>,
    /// Symbol index backend: `auto` (ctags if available, else regex) / `ctags` / `regex`.
    #[serde(default)]
    pub symbols_backend: Option<String>,
}

/// Auto-compaction strategy: `summarize` (default) summarizes all old rounds;
/// `drop` also discards the oldest rounds entirely; `off` disables auto-compaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactionStrategy {
    #[default]
    Summarize,
    Drop,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("provider '{0}' not found in config")]
    UnknownProvider(String),
    #[error("provider '{0}' has unsupported type '{1}' (expected \"openai\" or \"anthropic\")")]
    UnknownType(String, String),
    #[error("provider '{0}': API key missing; set {1}, add api_key to config, or run /apikey")]
    MissingApiKey(String, String),
    #[error("models endpoint returned HTTP {status}: {message}")]
    ListModels { status: u16, message: String },
}

impl Config {
    pub fn default_with_provider(name: &str, provider: ProviderConfig) -> Self {
        let mut providers = HashMap::new();
        providers.insert(name.to_string(), provider);
        Self {
            providers,
            default_provider: name.to_string(),
            auto_approve: Vec::new(),
            max_iterations: 30,
            thinking: ThinkingLevel::Off,
            tools: ToolsConfig::default(),
            context_budget_chars: default_context_budget(),
            compaction_strategy: CompactionStrategy::default(),
        }
    }

    pub fn default_config() -> Self {
        Self::default_with_provider(
            "default",
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: Some("https://api.deepseek.com/v1".to_string()),
                api_key_env: Some("DEEPSEEK_API_KEY".to_string()),
                api_key: None,
                model: "deepseek-v4-flash".to_string(),
                max_tokens: None,
                temperature: None,
            },
        )
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&text)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn save_default(&self) -> Result<(), ConfigError> {
        self.save(&config_path())
    }

    /// Load config, creating a commented template on first run.
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, default_config_text())?;
            eprintln!("Created default config at {}", path.display());
            return Ok(Self::default_config());
        }
        let mut config = Self::load(path)?;
        let mut migrated = false;
        for provider in config.providers.values_mut() {
            // deepseek-chat / deepseek-reasoner were deprecated on 2026-07-24;
            // both legacy aliases map to deepseek-v4-flash (thinking is
            // controlled separately via `thinking`).
            if provider.model == "deepseek-chat" || provider.model == "deepseek-reasoner" {
                provider.model = "deepseek-v4-flash".to_string();
                migrated = true;
            }
        }
        if migrated {
            fs::write(path, toml::to_string_pretty(&config)?)?;
            eprintln!(
                "Migrated deprecated DeepSeek model names in {}",
                path.display()
            );
        }
        Ok(config)
    }

    pub fn provider(&self, name: Option<&str>) -> Result<&ProviderConfig, ConfigError> {
        let name = name.unwrap_or(&self.default_provider);
        self.providers
            .get(name)
            .ok_or_else(|| ConfigError::UnknownProvider(name.to_string()))
    }

    /// Resolve the API key for a provider: inline `api_key`, then `auth.json`,
    /// then `api_key_env`.
    pub fn api_key_for(&self, provider: &ProviderConfig, name: &str) -> Option<String> {
        if let Some(key) = &provider.api_key {
            return Some(key.clone());
        }
        if let Some(key) = load_auth().get(name) {
            return Some(key.clone());
        }
        provider
            .api_key_env
            .as_ref()
            .and_then(|env_name| env::var(env_name).ok())
    }

    /// Persist an API key to `auth.json` (kept separate from config.toml so
    /// provider definitions stay readable).
    pub fn set_api_key(&self, provider: &str, key: &str) -> Result<(), ConfigError> {
        let mut auth = load_auth();
        auth.insert(provider.to_string(), key.to_string());
        save_auth(&auth)
    }

    /// Fetch the model list from the provider's `/models` endpoint
    /// (OpenAI-compatible) or `/v1/models` (Anthropic), like opencode.
    pub async fn list_models(&self, name: &str) -> Result<Vec<String>, ConfigError> {
        let provider = self.provider(Some(name))?.clone();
        let key = self.api_key_for(&provider, name).unwrap_or_default();
        let base = provider.base_url.clone().unwrap_or_else(|| {
            if provider.r#type == "anthropic" {
                "https://api.anthropic.com".to_string()
            } else {
                "https://api.openai.com/v1".to_string()
            }
        });
        let base = base.trim_end_matches('/');
        let url = if provider.r#type == "anthropic" {
            format!("{base}/v1/models")
        } else {
            format!("{base}/models")
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let mut request = client.get(&url);
        if provider.r#type == "anthropic" {
            if !key.is_empty() {
                request = request.header("x-api-key", key);
            }
            request = request.header("anthropic-version", "2023-06-01");
        } else if !key.is_empty() {
            request = request.bearer_auth(key);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(ConfigError::ListModels { status, message });
        }
        let payload: serde_json::Value = response.json().await?;
        let mut models = Vec::new();
        if let Some(items) = payload.get("data").and_then(|d| d.as_array()) {
            for item in items {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    models.push(id.to_string());
                }
            }
        }
        models.sort();
        models.dedup();
        Ok(models)
    }

    pub fn build_provider(
        &self,
        name: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<Box<dyn Provider>, ConfigError> {
        let name = name.unwrap_or(&self.default_provider).to_string();
        let provider = self.provider(Some(&name))?.clone();
        let api_key = self.api_key_for(&provider, &name).ok_or_else(|| {
            let env_name = provider
                .api_key_env
                .clone()
                .unwrap_or_else(|| "API_KEY".to_string());
            ConfigError::MissingApiKey(name.clone(), env_name)
        })?;
        let model = model_override.unwrap_or(&provider.model).to_string();
        match provider.r#type.as_str() {
            "openai" => Ok(Box::new(OpenAIProvider::new(
                provider
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                api_key,
                model,
                provider.max_tokens,
                provider.temperature,
            ))),
            "anthropic" => Ok(Box::new(AnthropicProvider::new(
                provider
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                api_key,
                model,
                provider.max_tokens,
                provider.temperature,
            ))),
            other => Err(ConfigError::UnknownType(name, other.to_string())),
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = env::var("FIRMENT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("firment")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Where per-provider API keys are stored (`auth.json`, separate from config).
pub fn auth_path() -> PathBuf {
    config_dir().join("auth.json")
}

pub type AuthMap = HashMap<String, String>;

pub fn load_auth() -> AuthMap {
    let path = auth_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_auth(auth: &AuthMap) -> Result<(), ConfigError> {
    if let Some(parent) = auth_path().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(auth_path(), serde_json::to_string_pretty(auth)?)?;
    Ok(())
}

pub fn default_config_text() -> &'static str {
    r#"# firment configuration

[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
# key 配置三种方式任选其一：
#   1. 环境变量 DEEPSEEK_API_KEY（推荐，避免明文落盘）
#   2. TUI 里执行 /apikey sk-xxx（写入 %APPDATA%\firment\auth.json，持久保存）
#   3. 直接填 api_key = "sk-..."（明文写在配置里，不推荐）
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"
# max_tokens = 8192
# temperature = 0.2

# 多 Provider 示例（TUI 里用 /provider claude 切换）
# [providers.claude]
# type = "anthropic"
# base_url = "https://api.anthropic.com"
# api_key_env = "ANTHROPIC_API_KEY"
# model = "claude-sonnet-4-5"

# Tools that skip confirmation prompts (write_file, edit_file, shell).
# auto_approve = []

# Max tool-calling rounds per turn.
# max_iterations = 30
# thinking = "medium"   # off / low / medium / high / xhigh / max（思考深度）
# context_budget_chars = 60000   # 会话上下文字符预算，超出后自动压缩早期对话
# compaction_strategy = "summarize"   # 默认 summarize；可选 drop（超预算直接丢弃旧轮）/ off（不自动压缩）

[tools]
# 代码改动后，agent 需先跑通 verify 再宣布完成；留空则 verify 工具不可用
# verify_command = "cargo check"
# 嵌入式示例：
# verify_command = "cmake --build build"
# symbols_backend = "auto"   # auto / ctags / regex（符号索引后端，auto=有 ctags 用 ctags）
"#
}

fn default_provider_name() -> String {
    "default".to_string()
}

fn default_max_iterations() -> usize {
    30
}

fn default_context_budget() -> usize {
    60_000
}
