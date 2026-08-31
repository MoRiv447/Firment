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
    /// Cap on output tokens per assistant reply (sent as the API's
    /// max_tokens). `None` falls back to the provider's own max_tokens, then
    /// to the built-in default (32k).
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Auto-compaction strategy (see `CompactionStrategy`).
    #[serde(default)]
    pub compaction_strategy: CompactionStrategy,
    /// MQTT broker for the SBC data plane (guard / device telemetry). Absent
    /// or empty `broker` = feature off (single-player without an SBC).
    #[serde(default)]
    pub mqtt: MqttConfig,
}

/// `[mqtt]` in config.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MqttConfig {
    /// "host:port" of the mosquitto broker, e.g. "192.168.1.6:1883".
    #[serde(default)]
    pub broker: String,
}

/// ELF binary-analysis gate policy. Written as a string (glob only) in
/// config.toml for backward compatibility, or as a table for full control.
#[derive(Debug, Clone, Serialize)]
pub struct ElfConfig {
    /// Glob pattern for the firmware ELF artifact (e.g. `build/fw.elf`).
    pub glob: String,
    /// Per-function stack-depth increase (bytes) that blocks completion
    /// until the user explicitly approves it.
    #[serde(default = "default_elf_stack_threshold")]
    pub stack_threshold: u32,
    /// Flash growth (KiB) that blocks completion until user approval.
    #[serde(default = "default_elf_flash_threshold")]
    pub flash_threshold_kib: u64,
    /// RAM growth (KiB) that blocks completion until user approval.
    #[serde(default = "default_elf_ram_threshold")]
    pub ram_threshold_kib: u64,
    /// Surface benign (below-threshold) diffs to the model as a review
    /// round. Default `false`: below-threshold changes are swallowed so
    /// the model is not trained to dismiss every diff as noise.
    #[serde(default)]
    pub report_benign: bool,
    /// Headless (no interactive approver) behavior: `true` blocks
    /// completion until the gate clears (CI); `false` downgrades an
    /// otherwise-blocking diff to a soft report.
    #[serde(default)]
    pub strict: bool,
}

fn default_elf_stack_threshold() -> u32 {
    32
}

fn default_elf_flash_threshold() -> u64 {
    1
}

fn default_elf_ram_threshold() -> u64 {
    1
}

impl Default for ElfConfig {
    fn default() -> Self {
        Self {
            glob: String::new(),
            stack_threshold: default_elf_stack_threshold(),
            flash_threshold_kib: default_elf_flash_threshold(),
            ram_threshold_kib: default_elf_ram_threshold(),
            report_benign: false,
            strict: false,
        }
    }
}

/// Accept both `elf = "build/fw.elf"` (glob string) and
/// `[tools.elf] glob = "..." stack_threshold = 64` (table) forms.
impl<'de> Deserialize<'de> for ElfConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Glob(String),
            Table {
                glob: String,
                #[serde(default = "default_elf_stack_threshold")]
                stack_threshold: u32,
                #[serde(default = "default_elf_flash_threshold")]
                flash_threshold_kib: u64,
                #[serde(default = "default_elf_ram_threshold")]
                ram_threshold_kib: u64,
                #[serde(default)]
                report_benign: bool,
                #[serde(default)]
                strict: bool,
            },
        }
        match Raw::deserialize(d)? {
            Raw::Glob(glob) => Ok(ElfConfig {
                glob,
                ..ElfConfig::default()
            }),
            Raw::Table {
                glob,
                stack_threshold,
                flash_threshold_kib,
                ram_threshold_kib,
                report_benign,
                strict,
            } => Ok(ElfConfig {
                glob,
                stack_threshold,
                flash_threshold_kib,
                ram_threshold_kib,
                report_benign,
                strict,
            }),
        }
    }
}

/// Logic-analyzer capture policy for the `la` tool. Written as a string
/// (driver name only) in config.toml for brevity, or as a table for full
/// control — the same dual form as [`ElfConfig`].
///
/// sigrok-cli is invoked as an EXTERNAL binary (argv array, no shell): the
/// GPL of sigrok/libsigrok does not reach this MIT program through a process
/// boundary, the same way probe-rs is used. Never link libsigrok.
#[derive(Debug, Clone, Serialize)]
pub struct LaConfig {
    /// sigrok driver name (e.g. `fx2lafw`, `saleae-logic`, `demo`).
    pub driver: String,
    /// Path to the sigrok-cli binary; defaults to `sigrok-cli` on PATH.
    /// A project-level config may NEVER set this (see `merged_for`): an
    /// executable path from an untrusted checkout is an arbitrary-program
    /// entry point.
    #[serde(default)]
    pub bin: Option<String>,
    /// Default samplerate token (sigrok spelling: `8m`, `100k`, …).
    #[serde(default)]
    pub samplerate: Option<String>,
    /// Default channel spec (e.g. `0,1,2-3`).
    #[serde(default)]
    pub channels: Option<String>,
    /// Upper bound on samples per capture: protects the disk from a
    /// runaway acquisition (10 M samples × 8 ch ≈ 1 MB of raw bits).
    #[serde(default = "default_la_max_samples")]
    pub max_samples: u64,
    /// Outer timeout for one sigrok-cli invocation. Unset derives it from
    /// the capture window plus headroom.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

fn default_la_max_samples() -> u64 {
    10_000_000
}

impl Default for LaConfig {
    fn default() -> Self {
        Self {
            driver: String::new(),
            bin: None,
            samplerate: None,
            channels: None,
            max_samples: default_la_max_samples(),
            timeout_ms: None,
        }
    }
}

/// Accept both `la = "fx2lafw"` (driver string) and
/// `[tools.la] driver = "..." samplerate = "8m"` (table) forms.
impl<'de> Deserialize<'de> for LaConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Driver(String),
            Table {
                driver: String,
                #[serde(default)]
                bin: Option<String>,
                #[serde(default)]
                samplerate: Option<String>,
                #[serde(default)]
                channels: Option<String>,
                #[serde(default = "default_la_max_samples")]
                max_samples: u64,
                #[serde(default)]
                timeout_ms: Option<u64>,
            },
        }
        match Raw::deserialize(d)? {
            Raw::Driver(driver) => Ok(LaConfig {
                driver,
                ..LaConfig::default()
            }),
            Raw::Table {
                driver,
                bin,
                samplerate,
                channels,
                max_samples,
                timeout_ms,
            } => Ok(LaConfig {
                driver,
                bin,
                samplerate,
                channels,
                max_samples,
                timeout_ms,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Command run by the `verify` tool (platform shell), e.g. `cargo check`.
    #[serde(default)]
    pub verify_command: Option<String>,
    /// Symbol index backend: `auto` (ctags if available, else regex) / `ctags` / `regex`.
    #[serde(default)]
    pub symbols_backend: Option<String>,
    /// Command run by the `build` tool (platform shell), e.g. `cmake --build build`.
    #[serde(default)]
    pub build_command: Option<String>,
    /// Default target chip for the `flash` tool (e.g. `stm32f407vetx`).
    #[serde(default)]
    pub default_chip: Option<String>,
    /// Serial port for `firm monitor` (e.g. `COM3`).
    #[serde(default)]
    pub monitor_port: Option<String>,
    /// Default baud rate for `firm monitor`.
    #[serde(default = "default_monitor_baud")]
    pub monitor_baud: u32,
    /// Web search provider: `bing` (no key, default — reachable from CN), or
    /// `duckduckgo` / `tavily` / `brave`.
    #[serde(default = "default_web_search")]
    pub web_search: Option<String>,
    /// Web search API key (tavily / brave).
    #[serde(default)]
    pub web_search_api_key: Option<String>,
    /// Environment variable holding the web search API key.
    #[serde(default)]
    pub web_search_api_key_env: Option<String>,
    /// Recursion limit for the `task` subagent tool.
    #[serde(default = "default_max_subagent_depth")]
    pub max_subagent_depth: usize,
    /// ELF binary-analysis gate: glob + thresholds. When set, the harness
    /// captures an ELF baseline and automatically runs `elf_analyze` against
    /// the newest match before each finished turn; changes above the
    /// thresholds block completion until the user approves (or, headless +
    /// `strict`, until fixed).
    ///
    /// TODO(relative scaling): thresholds are absolute (bytes/KiB). Scale them
    /// relative to target RAM/Flash size (e.g. 1% of available flash) so small
    /// MCUs are not drowned out and big ones are not overly strict.
    #[serde(default)]
    pub elf: Option<ElfConfig>,
    /// Logic analyzer for the `la` tool: sigrok driver + capture defaults.
    /// When unset, `la` action=capture/decode still works with every
    /// parameter passed explicitly; only the config-derived defaults
    /// (driver, samplerate, channels) are missing.
    #[serde(default)]
    pub la: Option<LaConfig>,
}

/// Hand-written so the in-memory default matches the serde defaults used
/// when a key is missing from config.toml. The derived `Default` silently
/// produced `monitor_baud = 0` and `max_subagent_depth = 0` (the latter then
/// clamped to 1 by the agent), diverging from the documented 115200 / 2.
impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            verify_command: None,
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: default_monitor_baud(),
            web_search: default_web_search(),
            web_search_api_key: None,
            web_search_api_key_env: None,
            max_subagent_depth: default_max_subagent_depth(),
            elf: None,
            la: None,
        }
    }
}

impl ToolsConfig {
    /// Resolved web search API key: the inline `web_search_api_key` wins,
    /// otherwise the variable named by `web_search_api_key_env` is read.
    pub fn resolved_web_search_api_key(&self) -> Option<String> {
        if let Some(key) = self.web_search_api_key.as_deref().filter(|k| !k.is_empty()) {
            return Some(key.to_string());
        }
        self.web_search_api_key_env
            .as_deref()
            .filter(|name| !name.is_empty())
            .and_then(|name| std::env::var(name).ok().filter(|k| !k.is_empty()))
    }
}

/// Resolved model endpoints for the `models` discovery tool: base_url
/// defaulted by provider type, API key resolved inline → env → auth.json.
pub fn provider_endpoints(config: &Config) -> Vec<crate::tool::ProviderEndpoint> {
    let auth = crate::load_auth();
    config
        .providers
        .iter()
        .map(|(name, p)| {
            let default_base = if p.r#type == "anthropic" {
                "https://api.anthropic.com"
            } else {
                "https://api.openai.com/v1"
            };
            crate::tool::ProviderEndpoint {
                name: name.clone(),
                base_url: p
                    .base_url
                    .clone()
                    .unwrap_or_else(|| default_base.to_string()),
                api_key: p
                    .api_key
                    .clone()
                    .or_else(|| p.api_key_env.as_ref().and_then(|e| std::env::var(e).ok()))
                    .or_else(|| auth.get(name).cloned()),
            }
        })
        .collect()
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

impl ProviderConfig {
    /// Base URL with the per-type default applied and trailing slashes
    /// trimmed — the single source both `list_models` and `firm doctor`
    /// probe against.
    pub fn effective_base(&self) -> String {
        let base = self.base_url.clone().unwrap_or_else(|| {
            if self.r#type == "anthropic" {
                "https://api.anthropic.com".to_string()
            } else {
                "https://api.openai.com/v1".to_string()
            }
        });
        base.trim_end_matches('/').to_string()
    }

    /// URL of the models endpoint. A base that already carries the
    /// anthropic `/v1` prefix (e.g. `https://openrouter.ai/api/v1`) must
    /// NOT get a second one — `/api/v1/v1/models` is a guaranteed 404.
    pub fn models_url(&self) -> String {
        let base = self.effective_base();
        if self.r#type == "anthropic" && !base.ends_with("/v1") {
            format!("{base}/v1/models")
        } else {
            format!("{base}/models")
        }
    }
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
            auto_approve: vec!["build".to_string()],
            max_iterations: 30,
            thinking: ThinkingLevel::Off,
            tools: ToolsConfig::default(),
            context_budget_chars: default_context_budget(),
            max_output_tokens: None,
            compaction_strategy: CompactionStrategy::default(),
            mqtt: MqttConfig::default(),
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

    /// Return a copy with project-local `.firment.toml` / `firment.toml`
    /// values merged over this config (project wins). Searches cwd and
    /// ancestors, like AGENTS.md.
    pub fn merged_for(&self, cwd: &Path) -> Config {
        let mut config = self.clone();
        let Some(project) = load_project_config(cwd) else {
            return config;
        };
        if let Some(value) = project.tools.verify_command {
            config.tools.verify_command = Some(value);
            // A command coming from the project (i.e. from an untrusted repo
            // checkout) must not run without an explicit human approval, even
            // if the user's own config auto-approves it.
            config.auto_approve.retain(|t| t != "verify");
        }
        if let Some(value) = project.tools.symbols_backend {
            config.tools.symbols_backend = Some(value);
        }
        if let Some(value) = project.tools.build_command {
            config.tools.build_command = Some(value);
            config.auto_approve.retain(|t| t != "build");
        }
        if let Some(value) = project.tools.default_chip {
            config.tools.default_chip = Some(value);
        }
        if let Some(value) = project.tools.monitor_port {
            config.tools.monitor_port = Some(value);
        }
        if project.tools.monitor_baud != default_monitor_baud() {
            config.tools.monitor_baud = project.tools.monitor_baud;
        }
        if let Some(value) = project.tools.web_search {
            config.tools.web_search = Some(value);
        }
        if let Some(value) = project.tools.web_search_api_key {
            config.tools.web_search_api_key = Some(value);
        }
        if let Some(value) = project.tools.web_search_api_key_env {
            config.tools.web_search_api_key_env = Some(value);
        }
        if project.tools.max_subagent_depth != default_max_subagent_depth() {
            config.tools.max_subagent_depth = project.tools.max_subagent_depth;
        }
        if let Some(elf) = &project.tools.elf {
            match config.tools.elf.as_mut() {
                Some(existing) => {
                    existing.glob = elf.glob.clone();
                    existing.stack_threshold = elf.stack_threshold;
                    existing.flash_threshold_kib = elf.flash_threshold_kib;
                    existing.ram_threshold_kib = elf.ram_threshold_kib;
                    existing.report_benign = elf.report_benign;
                    // strict is a tightening (blocking) flag, the opposite of
                    // auto_approve, so a checkout may enable it (e.g. CI).
                    existing.strict |= elf.strict;
                }
                None => config.tools.elf = Some(elf.clone()),
            }
        }
        if let Some(la) = &project.tools.la {
            match config.tools.la.as_mut() {
                Some(existing) => {
                    existing.driver = la.driver.clone();
                    // `bin` is deliberately NOT taken from the project:
                    // pointing the agent at an executable chosen by an
                    // untrusted checkout is an arbitrary-program entry
                    // point — the same reasoning that strips auto_approve
                    // from project build/verify commands above.
                    existing.samplerate = la.samplerate.clone();
                    existing.channels = la.channels.clone();
                    existing.max_samples = la.max_samples;
                    existing.timeout_ms = la.timeout_ms;
                }
                None => {
                    let mut v = la.clone();
                    v.bin = None;
                    config.tools.la = Some(v);
                }
            }
        }
        if project.compaction_strategy != CompactionStrategy::default() {
            config.compaction_strategy = project.compaction_strategy;
        }
        if project.max_output_tokens.is_some() {
            config.max_output_tokens = project.max_output_tokens;
        }
        // Per-run behavior knobs from the project config. auto_approve is
        // deliberately NOT merged: a project checkout must never grant itself
        // tool auto-approval (build/verify already opt out above).
        if project.max_iterations != default_max_iterations() {
            config.max_iterations = project.max_iterations;
        }
        if project.thinking != ThinkingLevel::default() {
            config.thinking = project.thinking;
        }
        if project.context_budget_chars != default_context_budget() {
            config.context_budget_chars = project.context_budget_chars;
        }
        config
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
    /// then `api_key_env`. A blank inline value is treated as unset so it
    /// falls through to auth.json / the environment instead of breaking every
    /// request with an empty key.
    pub fn api_key_for(&self, provider: &ProviderConfig, name: &str) -> Option<String> {
        if let Some(key) = provider.api_key.as_deref().filter(|k| !k.is_empty()) {
            return Some(key.to_string());
        }
        if let Some(key) = load_auth().get(name) {
            return Some(key.clone());
        }
        provider
            .api_key_env
            .as_ref()
            // An empty-but-set variable is treated as unset, matching the
            // inline-key rule above and resolved_web_search_api_key.
            .and_then(|env_name| env::var(env_name).ok())
            .filter(|k| !k.is_empty())
    }

    /// Persist an API key to `auth.json` (kept separate from config.toml so
    /// provider definitions stay readable).
    pub fn set_api_key(&self, provider: &str, key: &str) -> Result<(), ConfigError> {
        let mut auth = load_auth();
        auth.insert(provider.to_string(), key.to_string());
        save_auth(&auth)
    }

    /// Add or update a provider definition (type, base URL, model) and persist
    /// the whole config. When `name` equals `default_provider`, the updated
    /// values take effect for the next turn automatically.
    pub fn set_provider(
        &mut self,
        name: &str,
        provider_type: &str,
        base_url: Option<String>,
        model: &str,
    ) -> Result<(), ConfigError> {
        let entry = self
            .providers
            .entry(name.to_string())
            .or_insert_with(|| ProviderConfig {
                r#type: provider_type.to_string(),
                base_url: base_url.clone(),
                api_key_env: None,
                api_key: None,
                model: model.to_string(),
                max_tokens: None,
                temperature: None,
            });
        entry.r#type = provider_type.to_string();
        if base_url.is_some() {
            entry.base_url = base_url;
        }
        entry.model = model.to_string();
        self.save(&config_path())
    }

    /// Remove a provider definition. If the removed provider was the default,
    /// the default is repointed to the first remaining provider (deterministic:
    /// sorted by name). Deleting the last remaining provider is rejected.
    pub fn remove_provider(&mut self, name: &str) -> Result<(), ConfigError> {
        if !self.providers.contains_key(name) {
            return Ok(());
        }
        self.providers.remove(name);
        if self.default_provider == name {
            let mut remaining: Vec<&String> = self.providers.keys().collect();
            remaining.sort();
            let Some(next) = remaining.first() else {
                return Err(ConfigError::UnknownProvider(
                    "cannot delete the last provider — at least one must remain".to_string(),
                ));
            };
            self.default_provider = (*next).clone();
        }
        self.save(&config_path())
    }

    /// Fetch the model list from the provider's `/models` endpoint
    /// (OpenAI-compatible) or `/v1/models` (Anthropic).
    pub async fn list_models(&self, name: &str) -> Result<Vec<String>, ConfigError> {
        let provider = self.provider(Some(name))?.clone();
        let key = self.api_key_for(&provider, name).unwrap_or_default();
        let url = provider.models_url();
        let client = crate::http_builder()
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
        // Output cap precedence: explicit config max_output_tokens >
        // provider-level max_tokens > built-in 32k default.
        let max_output_tokens = self
            .max_output_tokens
            .or(provider.max_tokens)
            .unwrap_or_else(default_max_output_tokens);
        match provider.r#type.as_str() {
            "openai" => Ok(Box::new(OpenAIProvider::new(
                provider
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
                api_key,
                model,
                Some(max_output_tokens),
                provider.temperature,
            ))),
            "anthropic" => Ok(Box::new(AnthropicProvider::new(
                provider
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
                api_key,
                model,
                Some(max_output_tokens),
                provider.temperature,
            ))),
            other => Err(ConfigError::UnknownType(name, other.to_string())),
        }
    }
}

/// Parse a size with an optional binary suffix: plain chars/tokens, or a
/// trailing `k`/`m` (1024 / 1024^2), case-insensitive (e.g. 256k = 262144,
/// 32k = 32768). Shared by the CLI flags and TUI slash commands.
pub fn parse_size(s: &str) -> Result<usize, std::io::Error> {
    let s = s.trim();
    let invalid = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid size '{s}': use a plain number or a k/m suffix, e.g. 262144 or 256k"),
        )
    };
    if s.is_empty() {
        return Err(invalid());
    }
    let (digits, mult) = match s.as_bytes().last() {
        Some(b'k') | Some(b'K') => (&s[..s.len() - 1], 1024usize),
        Some(b'm') | Some(b'M') => (&s[..s.len() - 1], 1024usize * 1024),
        _ => (s, 1usize),
    };
    let n: usize = digits.parse().map_err(|_| invalid())?;
    n.checked_mul(mult).ok_or_else(invalid)
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
    let path = auth_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(auth)?)?;
    // API keys are secrets: never world-readable (Unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn default_config_text() -> &'static str {
    r#"# firment configuration

[providers.default]
type = "openai"
base_url = "https://api.deepseek.com/v1"
# Set the key in one of three ways:
#   1. Environment variable DEEPSEEK_API_KEY (recommended, no plaintext on disk)
#   2. Run /apikey sk-xxx in the TUI (writes %APPDATA%\firment\auth.json, persisted)
#   3. Set api_key = "sk-..." directly (plaintext in the config file, not recommended)
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-v4-flash"
# max_tokens = 8192
# temperature = 0.2

# Multiple-provider example (switch in the TUI with /provider example)
# [providers.example]
# type = "anthropic"
# base_url = "https://api.anthropic.com"
# api_key_env = "ANTHROPIC_API_KEY"
# model = "example-sonnet"

# Tools that skip confirmation prompts (write_file, edit_file, shell).
# build is auto-approved by default (it runs a user-configured command); flash always asks.
# auto_approve = ["build"]

# Max tool-calling rounds per turn.
# max_iterations = 30
# thinking = "medium"   # off / low / medium / high / xhigh / max (reasoning depth)
# context_budget_chars = 262144   # session context budget in chars (256k binary default); older messages are compacted past this
# max_output_tokens = 32768       # cap on output tokens per reply (32k default; overrides provider max_tokens)
# compaction_strategy = "summarize"   # default summarize; drop (discard old turns) / off (no auto-compaction)

[tools]
# After code changes, the agent must pass verify before declaring completion; empty disables the tool
# verify_command = "cargo check"
# Embedded example:
# verify_command = "cmake --build build"
# symbols_backend = "auto"   # auto / ctags / regex (symbol index backend; auto uses ctags when available)
# build_command = "cmake --build build"   # command run by the build tool (Keil: uv4 -j0 -b project.uvprojx)
# default_chip = "stm32f407vetx"          # default chip for the flash tool (probe-rs chip name)
# monitor_port = "COM3"                   # default serial port for firm monitor
# monitor_baud = 115200                   # default baud rate for firm monitor
# web_search = "bing"                     # default: bing (no key, CN-reachable); duckduckgo / tavily / brave also work
# web_search_api_key_env = "TAVILY_API_KEY"  # API key env for tavily / brave (or set web_search_api_key inline)
# elf = "build/fw.elf"                       # glob of the firmware ELF: harness seeds a binary baseline and auto-runs elf_analyze before finishing each edited turn (needs -fstack-usage for stack depth)
# Full gate policy (table form, all fields optional):
# [tools.elf]
# glob = "build/fw.elf"
# stack_threshold = 32        # stack-depth increase (bytes) that blocks completion until user approval
# flash_threshold_kib = 1     # flash growth (KiB) that blocks completion until user approval
# ram_threshold_kib = 1       # RAM growth (KiB) that blocks completion until user approval
# report_benign = false       # surface below-threshold diffs as a review round (default false: swallow noise)
# strict = false              # headless/CI: block completion until fixed instead of downgrading to a soft report
# max_subagent_depth = 2                  # recursion limit for the task subagent tool
# Logic analyzer (la tool): sigrok driver + capture defaults. sigrok-cli must be installed separately (external binary, never linked).
# la = "fx2lafw"                              # short form: driver name only
# [tools.la]
# driver = "fx2lafw"                      # sigrok driver (fx2lafw clones, saleae-logic, demo, …)
# bin = "sigrok-cli"                      # explicit path if not on PATH (user config only — a project .firment.toml cannot set this)
# samplerate = "8m"                       # default sample rate token (8m = 8 MHz)
# channels = "0,1,2-3"                    # default channel spec
# max_samples = 10000000                  # per-capture sample cap (disk protection)
# timeout_ms = 60000                      # outer timeout per sigrok-cli invocation
"#
}

fn load_project_config(cwd: &Path) -> Option<Config> {
    for dir in cwd.ancestors() {
        for name in [".firment.toml", "firment.toml"] {
            let path = dir.join(name);
            if path.is_file()
                && let Ok(text) = fs::read_to_string(&path)
                && let Ok(config) = toml::from_str::<Config>(&text)
            {
                return Some(config);
            }
        }
    }
    None
}

fn default_provider_name() -> String {
    "default".to_string()
}

fn default_max_iterations() -> usize {
    30
}

fn default_monitor_baud() -> u32 {
    115_200
}

/// Default web search provider: `bing` — no key, no cookie, reachable from
/// mainland China where DuckDuckGo is unreliable. International users can
/// set `web_search = "duckduckgo"` (or tavily/brave with an API key) to
/// prefer English-first results.
fn default_web_search() -> Option<String> {
    Some("bing".to_string())
}

fn default_max_subagent_depth() -> usize {
    2
}

fn default_context_budget() -> usize {
    256 * 1024 // 256k chars (binary)
}

/// Default cap on output tokens per reply when neither the config nor the
/// provider specifies one (32k starts generous for long tool-calling turns).
pub fn default_max_output_tokens() -> u32 {
    32_768
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tools_elf_string_backward_compatible() {
        let text = r#"
            [tools]
            elf = "build/*.elf"
        "#;
        let config: Config = toml::from_str(text).unwrap();
        let elf = config.tools.elf.expect("elf set");
        assert_eq!(elf.glob, "build/*.elf");
        assert_eq!(elf.stack_threshold, default_elf_stack_threshold());
        assert_eq!(elf.flash_threshold_kib, default_elf_flash_threshold());
        assert_eq!(elf.ram_threshold_kib, default_elf_ram_threshold());
        assert!(!elf.report_benign);
        assert!(!elf.strict);
    }

    #[test]
    fn parses_tools_elf_table_with_thresholds() {
        let text = r#"
            [tools.elf]
            glob = "build/*.elf"
            stack_threshold = 64
            flash_threshold_kib = 2
            ram_threshold_kib = 4
            report_benign = true
            strict = true
        "#;
        let config: Config = toml::from_str(text).unwrap();
        let elf = config.tools.elf.expect("elf set");
        assert_eq!(elf.glob, "build/*.elf");
        assert_eq!(elf.stack_threshold, 64);
        assert_eq!(elf.flash_threshold_kib, 2);
        assert_eq!(elf.ram_threshold_kib, 4);
        assert!(elf.report_benign);
        assert!(elf.strict);
    }

    #[test]
    fn tools_elf_defaults_to_none() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.tools.elf.is_none());
    }

    #[test]
    fn parses_tools_la_string_short_form() {
        let text = r#"
            [tools]
            la = "fx2lafw"
        "#;
        let config: Config = toml::from_str(text).unwrap();
        let la = config.tools.la.expect("la set");
        assert_eq!(la.driver, "fx2lafw");
        assert!(la.bin.is_none());
        assert_eq!(la.max_samples, default_la_max_samples());
    }

    #[test]
    fn parses_tools_la_table() {
        let text = r#"
            [tools.la]
            driver = "saleae-logic"
            bin = "D:/tools/sigrok-cli.exe"
            samplerate = "8m"
            channels = "0,1,2-3"
            max_samples = 500000
            timeout_ms = 30000
        "#;
        let config: Config = toml::from_str(text).unwrap();
        let la = config.tools.la.expect("la set");
        assert_eq!(la.driver, "saleae-logic");
        assert_eq!(la.bin.as_deref(), Some("D:/tools/sigrok-cli.exe"));
        assert_eq!(la.samplerate.as_deref(), Some("8m"));
        assert_eq!(la.channels.as_deref(), Some("0,1,2-3"));
        assert_eq!(la.max_samples, 500000);
        assert_eq!(la.timeout_ms, Some(30000));
    }

    #[test]
    fn project_la_config_never_supplies_a_binary_path() {
        // An executable path from an untrusted checkout is an
        // arbitrary-program entry point: the merge must drop `bin` in both
        // directions — overriding an existing config and arriving fresh.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".firment.toml"),
            r#"
            [tools.la]
            driver = "fx2lafw"
            bin = "evil.exe"
            "#,
        )
        .unwrap();
        let mut base = Config::default_config();
        base.tools.la = Some(LaConfig {
            driver: "saleae-logic".to_string(),
            bin: Some("C:/sigrok/sigrok-cli.exe".to_string()),
            ..LaConfig::default()
        });
        let merged = base.merged_for(dir.path());
        let la = merged.tools.la.expect("la merged");
        assert_eq!(la.driver, "fx2lafw", "project driver wins");
        assert_eq!(
            la.bin.as_deref(),
            Some("C:/sigrok/sigrok-cli.exe"),
            "the user's own bin survives"
        );

        let merged2 = Config::default_config().merged_for(dir.path());
        assert_eq!(
            merged2.tools.la.expect("la from project").bin,
            None,
            "a fresh project-level la must not carry a bin either"
        );
    }

    #[test]
    fn project_config_removes_build_verify_from_auto_approve() {
        let dir = tempfile::tempdir().unwrap();
        let project_toml = r#"
            [tools]
            build_command = "echo build"
            verify_command = "echo verify"
        "#;
        std::fs::write(dir.path().join(".firment.toml"), project_toml).unwrap();

        let mut base = Config::default_with_provider(
            "default",
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: None,
                api_key_env: None,
                api_key: None,
                model: "m".to_string(),
                max_tokens: None,
                temperature: None,
            },
        );
        base.auto_approve = vec!["build".to_string(), "verify".to_string()];

        let merged = base.merged_for(dir.path());
        assert_eq!(
            merged.tools.build_command.as_deref(),
            Some("echo build"),
            "project build_command must be merged"
        );
        assert_eq!(
            merged.tools.verify_command.as_deref(),
            Some("echo verify"),
            "project verify_command must be merged"
        );
        assert!(
            !merged.auto_approve.iter().any(|t| t == "build"),
            "project-provided build command must not be auto-approved: {:?}",
            merged.auto_approve
        );
        assert!(
            !merged.auto_approve.iter().any(|t| t == "verify"),
            "project-provided verify command must not be auto-approved: {:?}",
            merged.auto_approve
        );
    }

    #[test]
    fn no_project_config_keeps_auto_approve() {
        let dir = tempfile::tempdir().unwrap();
        let mut base = Config::default_with_provider(
            "default",
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: None,
                api_key_env: None,
                api_key: None,
                model: "m".to_string(),
                max_tokens: None,
                temperature: None,
            },
        );
        base.auto_approve = vec!["build".to_string()];
        let merged = base.merged_for(dir.path());
        assert!(merged.auto_approve.iter().any(|t| t == "build"));
        assert_eq!(merged.tools.build_command, None);
    }

    #[test]
    fn size_suffixes_parse_binary() {
        assert_eq!(parse_size("262144").unwrap(), 262144);
        assert_eq!(parse_size("256k").unwrap(), 262144);
        assert_eq!(parse_size("256K").unwrap(), 262144);
        assert_eq!(parse_size("32k").unwrap(), 32 * 1024);
        assert_eq!(parse_size("1m").unwrap(), 1024 * 1024);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1.5k").is_err());
        assert!(parse_size("-1").is_err());
    }

    #[test]
    fn defaults_are_256k_budget_32k_output() {
        let config = Config::default_with_provider(
            "default",
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: None,
                api_key_env: None,
                api_key: None,
                model: "m".to_string(),
                max_tokens: None,
                temperature: None,
            },
        );
        assert_eq!(config.context_budget_chars, 256 * 1024);
        assert_eq!(config.max_output_tokens, None);
        assert_eq!(default_max_output_tokens(), 32 * 1024);
    }
}

#[cfg(test)]
mod models_url_tests {
    use super::ProviderConfig;

    fn provider(t: &str, base: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            r#type: t.to_string(),
            base_url: base.map(String::from),
            api_key_env: None,
            api_key: None,
            model: "m".to_string(),
            max_tokens: None,
            temperature: None,
        }
    }

    #[test]
    fn anthropic_base_already_carrying_v1_is_not_doubled() {
        let p = provider("anthropic", Some("https://openrouter.ai/api/v1"));
        assert_eq!(p.models_url(), "https://openrouter.ai/api/v1/models");
    }

    #[test]
    fn anthropic_base_without_v1_gets_the_prefix() {
        let p = provider("anthropic", Some("https://api.deepseek.com"));
        assert_eq!(p.models_url(), "https://api.deepseek.com/v1/models");
    }

    #[test]
    fn openai_base_passes_through() {
        let p = provider("openai", Some("https://api.openai.com/v1"));
        assert_eq!(p.models_url(), "https://api.openai.com/v1/models");
    }

    #[test]
    fn trailing_slash_is_trimmed_before_appending() {
        let p = provider("openai", Some("http://localhost:11434/v1/"));
        assert_eq!(p.models_url(), "http://localhost:11434/v1/models");
    }

    #[test]
    fn defaults_apply_when_base_is_unset() {
        assert_eq!(
            provider("anthropic", None).models_url(),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            provider("openai", None).models_url(),
            "https://api.openai.com/v1/models"
        );
    }
}
