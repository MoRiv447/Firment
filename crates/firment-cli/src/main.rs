mod install;

use async_trait::async_trait;
use clap::Parser;
use firment_core::config::{config_path, parse_size};
use firment_core::{
    Agent, AgentEvent, Config, EventSink, PermissionChecker, PermissionError, PlanModePermission,
    Session, SessionMode, SessionStore, ThinkingLevel, load_auth,
};
use std::collections::HashSet;
use std::env;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "firm",
    version,
    about = "Firment: firmware-first coding agent (beta)",
    long_about = None
)]
struct Cli {
    /// Install / update the `firm` binary.
    #[command(subcommand)]
    command: Option<Command>,

    /// Run a single prompt in non-interactive mode.
    #[arg(short = 'p', long = "prompt")]
    prompt: Option<String>,

    /// Resume a session by id, or the latest one when no id is given.
    #[arg(long = "continue", num_args = 0..=1, default_missing_value = "latest")]
    continue_session: Option<String>,

    /// Override the model id.
    #[arg(long)]
    model: Option<String>,

    /// Override the provider profile name from the config.
    #[arg(long)]
    provider: Option<String>,

    /// Session context budget in characters before auto-compaction kicks in
    /// (default 256k; accepts a k/m suffix, e.g. 256k, or a plain char count).
    #[arg(long = "context-length", value_parser = parse_size)]
    context_length: Option<usize>,

    /// Cap on output tokens per reply (default 32k; accepts a k/m suffix,
    /// e.g. 32k, or a plain token count).
    #[arg(long = "max-output-tokens", value_parser = parse_size)]
    max_output_tokens: Option<usize>,

    /// Thinking effort: off/low/medium/high/xhigh/max (Anthropic extended thinking, OpenAI reasoning).
    #[arg(long, value_parser = parse_thinking)]
    thinking: Option<ThinkingLevel>,

    /// Read-only planning mode: no write/edit/shell tools.
    #[arg(long)]
    plan: bool,

    /// Working directory for the session.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Auto-approve all risky tool calls (write/edit/shell).
    #[arg(short = 'y', long)]
    yes: bool,

    /// Allow destructive shell commands (rm/del/git clean, etc.) even with -y.
    /// Without this flag, the hard safety guard blocks them in one-shot mode.
    #[arg(long)]
    allow_dangerous: bool,

    /// List saved sessions.
    #[arg(long)]
    list: bool,

    /// Check configuration and provider connectivity.
    #[arg(long)]
    doctor: bool,

    /// Path to config.toml.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Persist an API key for a provider: --set-key provider=sk-xxx
    #[arg(long = "set-key")]
    set_key: Option<String>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Install firm to %USERPROFILE%\.firment\bin and add it to the user PATH.
    Install {
        /// Install directory (default: %USERPROFILE%\.firment\bin).
        #[arg(long)]
        to: Option<PathBuf>,
        /// Copy files only; do not modify PATH or the PowerShell profile.
        #[arg(long)]
        files_only: bool,
    },
    /// Replace the installed binary with a newer release.
    Update {
        /// Path to the new executable (default: the currently running one).
        source: Option<PathBuf>,
        /// Install directory override (default: %USERPROFILE%\.firment\bin).
        #[arg(long)]
        to: Option<PathBuf>,
    },
    /// Run the configured build command (config [tools] build_command).
    Build,
    /// Flash a firmware ELF via probe-rs.
    Flash {
        /// Path to the firmware ELF.
        file: PathBuf,
        /// Target chip (defaults to config [tools] default_chip).
        #[arg(long)]
        chip: Option<String>,
        /// Probe serial/id to use.
        #[arg(long)]
        probe: Option<String>,
    },
    /// Flash and run the target via probe-rs, streaming RTT logs.
    Run {
        /// Path to the firmware ELF.
        file: PathBuf,
        /// Target chip (defaults to config [tools] default_chip).
        #[arg(long)]
        chip: Option<String>,
        /// Probe serial/id to use.
        #[arg(long)]
        probe: Option<String>,
        /// Timeout in seconds (0 = wait until Ctrl-C, default).
        #[arg(long, default_value_t = 0)]
        timeout: u64,
    },
    /// Monitor a serial port with optional ELF symbol decoding.
    Monitor {
        /// Serial port, e.g. COM3 (defaults to config [tools] monitor_port).
        #[arg(long)]
        port: Option<String>,
        /// Baud rate (0 = config [tools] monitor_baud, default).
        #[arg(long, default_value_t = 0)]
        baud: u32,
        /// ELF file for decoding hex code addresses in log lines.
        #[arg(long)]
        elf: Option<PathBuf>,
        /// Timeout in seconds (0 = run until Ctrl-C).
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Print the tool registry specs as JSON — the single source of truth
    /// for tool names/descriptions/schemas (consumed by web/IDE surfaces).
    Tools,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(command) = &cli.command {
        match command {
            Command::Install { to, files_only } => install::install(to.clone(), *files_only)?,
            Command::Update { source, to } => install::update(source.clone(), to.clone())?,
            Command::Build => {
                let cwd = cli
                    .cwd
                    .clone()
                    .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                let config = load_config(&cli)?.merged_for(&cwd);
                run_direct_tool(&config, cli.cwd.clone(), "build", serde_json::json!({})).await?;
            }
            Command::Flash { file, chip, probe } => {
                let cwd = cli
                    .cwd
                    .clone()
                    .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                let config = load_config(&cli)?.merged_for(&cwd);
                let mut args = serde_json::Map::new();
                args.insert("file".to_string(), serde_json::json!(file));
                if let Some(chip) = chip {
                    args.insert("chip".to_string(), serde_json::json!(chip));
                }
                if let Some(probe) = probe {
                    args.insert("probe".to_string(), serde_json::json!(probe));
                }
                run_direct_tool(
                    &config,
                    cli.cwd.clone(),
                    "flash",
                    serde_json::Value::Object(args),
                )
                .await?;
            }
            Command::Run {
                file,
                chip,
                probe,
                timeout,
            } => {
                let cwd = cli
                    .cwd
                    .clone()
                    .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                let config = load_config(&cli)?.merged_for(&cwd);
                let mut args = serde_json::Map::new();
                args.insert("file".to_string(), serde_json::json!(file));
                if let Some(chip) = chip {
                    args.insert("chip".to_string(), serde_json::json!(chip));
                }
                if let Some(probe) = probe {
                    args.insert("probe".to_string(), serde_json::json!(probe));
                }
                args.insert(
                    "timeout_ms".to_string(),
                    serde_json::json!(timeout.saturating_mul(1000)),
                );
                run_direct_tool(
                    &config,
                    cli.cwd.clone(),
                    "run",
                    serde_json::Value::Object(args),
                )
                .await?;
            }
            Command::Monitor {
                port,
                baud,
                elf,
                timeout,
            } => {
                let cwd = cli
                    .cwd
                    .clone()
                    .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
                let config = load_config(&cli)?.merged_for(&cwd);
                let port = port
                    .clone()
                    .or(config.tools.monitor_port.clone())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "missing serial port: use --port COMx or set monitor_port in config.toml"
                        )
                    })?;
                let baud = if *baud > 0 {
                    *baud
                } else {
                    config.tools.monitor_baud
                };
                run_monitor(&port, baud, elf.clone(), *timeout)?;
            }
            Command::Tools => {
                let registry = firment_tools::default_registry();
                println!("{}", serde_json::to_string_pretty(&registry.specs())?);
            }
        }
        return Ok(());
    }

    let config_path = cli.config.clone().unwrap_or_else(config_path);
    let mut config = Config::load_or_create(&config_path)?;
    // CLI overrides win over config values (and apply to both TUI and
    // one-shot paths, since both build from this config).
    if let Some(length) = cli.context_length {
        config.context_budget_chars = length;
    }
    if let Some(tokens) = cli.max_output_tokens {
        // Clamp like the TUI's /output command: values above u32::MAX would
        // otherwise silently wrap around and shrink the budget.
        config.max_output_tokens = Some(tokens.min(u32::MAX as usize) as u32);
    }
    let _ = firment_core::kb::ensure_seed_kb();

    if let Some(kv) = &cli.set_key {
        let (name, key) = kv
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--set-key expects provider=key"))?;
        if name.trim().is_empty() || key.trim().is_empty() {
            anyhow::bail!("--set-key expects provider=key");
        }
        config.set_api_key(name.trim(), key.trim())?;
        println!(
            "API key saved for provider '{}' at {}",
            name.trim(),
            firment_core::auth_path().display()
        );
        return Ok(());
    }

    if cli.list {
        list_sessions()?;
        return Ok(());
    }
    if cli.doctor {
        doctor(&config, &config_path).await?;
        doctor_install();
        return Ok(());
    }

    let cwd = cli.cwd.clone().unwrap_or(env::current_dir()?);
    let store = SessionStore::default();
    let session = if let Some(id) = &cli.continue_session {
        let id = if id == "latest" {
            store
                .latest()?
                .map(|s| s.id)
                .ok_or_else(|| anyhow::anyhow!("no previous session found"))?
        } else {
            id.clone()
        };
        let mut session = store.load(&id)?;
        if cli.cwd.is_some() {
            session.cwd = cwd;
        }
        if cli.plan {
            session.mode = SessionMode::Plan;
        }
        if let Some(model) = &cli.model {
            session.model = model.clone();
        }
        if let Some(thinking) = cli.thinking {
            session.thinking = thinking;
        }
        session
    } else {
        let provider = cli
            .provider
            .clone()
            .unwrap_or_else(|| config.default_provider.clone());
        let model = cli.model.clone().unwrap_or_else(|| {
            config
                .provider(Some(&provider))
                .map(|p| p.model.clone())
                .unwrap_or_default()
        });
        let mut session = Session::new(cwd, provider, model);
        session.thinking = cli.thinking.unwrap_or(config.thinking);
        session.mode = if cli.plan {
            SessionMode::Plan
        } else {
            SessionMode::Agent
        };
        session
    };

    if let Some(prompt) = &cli.prompt {
        run_once(&config, session, prompt, cli.yes, cli.allow_dangerous).await?;
    } else {
        firment_tui::run(config, config_path, session).await?;
    }
    Ok(())
}

async fn run_once(
    config: &Config,
    session: Session,
    prompt: &str,
    yes: bool,
    allow_dangerous: bool,
) -> anyhow::Result<()> {
    let config = config.merged_for(&session.cwd);
    let provider = config.build_provider(Some(&session.provider), Some(&session.model))?;
    let registry = if session.mode == SessionMode::Plan {
        firment_tools::plan_registry()
    } else {
        firment_tools::default_registry()
    };
    let store = SessionStore::default();
    let work_dir = store.dir.join("work");
    // The verify tool runs the user-configured command from config.toml; in
    // one-shot mode it is part of the completion gate, so it is always
    // auto-approved (the dangerous-command guard still applies).
    let mut auto_approve = config.auto_approve.clone();
    if !auto_approve.iter().any(|t| t == "verify") {
        auto_approve.push("verify".to_string());
    }
    if !auto_approve.iter().any(|t| t == "build") {
        auto_approve.push("build".to_string());
    }
    let base_permission = Arc::new(CliPermission::new(yes, auto_approve));
    let permission: Arc<dyn PermissionChecker> = if session.mode == SessionMode::Plan {
        Arc::new(PlanModePermission::new(base_permission))
    } else {
        base_permission
    };
    let sink = Arc::new(CliSink);
    let mut agent = Agent::new(
        Some(provider),
        registry,
        session,
        store,
        permission.clone(),
        sink,
        config.max_iterations,
    );
    agent.set_allow_dangerous(allow_dangerous);
    agent.set_verify_command(config.tools.verify_command.clone());
    agent.set_context_budget_chars(config.context_budget_chars);
    agent.set_compaction_strategy(config.compaction_strategy);
    agent.set_symbols_backend(config.tools.symbols_backend.clone());
    agent.set_build_command(config.tools.build_command.clone());
    agent.set_default_chip(config.tools.default_chip.clone());
    agent.set_monitor_port(config.tools.monitor_port.clone());
    agent.set_monitor_baud(config.tools.monitor_baud);
    agent.set_elf_config(config.tools.elf.clone());
    agent.set_max_subagent_depth(config.tools.max_subagent_depth);
    agent.set_web_search(
        config.tools.web_search.clone(),
        config.tools.resolved_web_search_api_key(),
    );
    agent.set_session_dir(Some(work_dir));
    let subagent_factory: Arc<firment_core::SubagentRunner> =
        Arc::new(firment_core::SubagentRunner::new(
            Arc::new(config.clone()),
            firment_tools::plan_registry(),
            agent.session().provider.clone(),
            agent.session().model.clone(),
            None,
            permission.clone(),
        ));
    agent.set_subagent_factory(Some(subagent_factory));
    let text = agent.run_turn(prompt).await?;
    println!("{text}");
    Ok(())
}

struct CliSink;

#[async_trait]
impl EventSink for CliSink {
    async fn event(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolStart { name, .. } => eprintln!("▶ {name}"),
            AgentEvent::ToolEnd {
                name, ok, summary, ..
            } => {
                let mark = if ok { "✓" } else { "✗" };
                eprintln!("  {mark} {name}: {summary}");
            }
            AgentEvent::Error(message) => eprintln!("⚠ {message}"),
            _ => {}
        }
    }
}

struct CliPermission {
    yes: bool,
    auto: HashSet<String>,
    always: Arc<Mutex<HashSet<String>>>,
    interactive: bool,
}

impl CliPermission {
    fn new(yes: bool, auto: Vec<String>) -> Self {
        Self {
            yes,
            auto: auto.into_iter().collect(),
            always: Arc::new(Mutex::new(HashSet::new())),
            interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        }
    }
}

#[async_trait]
impl PermissionChecker for CliPermission {
    async fn confirm(
        &self,
        tool: &str,
        _args: &serde_json::Value,
        reason: &str,
    ) -> Result<(), PermissionError> {
        if self.yes || self.auto.contains(tool) || self.always.lock().unwrap().contains(tool) {
            return Ok(());
        }
        if !self.interactive {
            return Err(PermissionError::denied(format!(
                "tool '{tool}' requires approval; rerun with -y or add it to auto_approve"
            )));
        }
        eprintln!("\n⚠ {tool}: {reason}");
        eprint!("Approve? [y/N/a] ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        match line.trim().to_lowercase().as_str() {
            "y" => Ok(()),
            "a" => {
                self.always.lock().unwrap().insert(tool.to_string());
                Ok(())
            }
            _ => Err(PermissionError::denied("denied by user")),
        }
    }
}

fn list_sessions() -> anyhow::Result<()> {
    let store = SessionStore::default();
    let sessions = store.list()?;
    if sessions.is_empty() {
        println!("No sessions yet.");
        return Ok(());
    }
    for summary in sessions {
        let preview = store
            .load(&summary.id)
            .map(|s| s.title())
            .unwrap_or_default();
        println!(
            "{:<36} {}  {:<24} {}  {}",
            summary.id,
            format_ts(summary.updated_at),
            summary.model,
            summary.cwd.display(),
            preview
        );
    }
    Ok(())
}

async fn doctor(config: &Config, path: &Path) -> anyhow::Result<()> {
    println!("config file: {}", path.display());
    if config.providers.is_empty() {
        println!("no providers configured");
        return Ok(());
    }
    for (name, provider) in &config.providers {
        let key_status = if provider.api_key.is_some() {
            "configured (inline)".to_string()
        } else if load_auth().contains_key(name) {
            "configured (auth.json)".to_string()
        } else if let Some(env_name) = &provider.api_key_env {
            if env::var(env_name).is_ok() {
                format!("configured via ${env_name}")
            } else {
                format!("MISSING (${env_name} not set)")
            }
        } else {
            "MISSING (no api_key or api_key_env)".to_string()
        };
        println!(
            "provider {name}: type={} model={}",
            provider.r#type, provider.model
        );
        println!("  api key: {key_status}");

        let base = provider.base_url.clone().unwrap_or_else(|| {
            if provider.r#type == "anthropic" {
                "https://api.anthropic.com".to_string()
            } else {
                "https://api.openai.com/v1".to_string()
            }
        });
        let base = base.trim_end_matches('/');
        let probe_url = if provider.r#type == "anthropic" {
            format!("{base}/v1/models")
        } else {
            format!("{base}/models")
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let mut request = client.get(&probe_url);
        let key = provider
            .api_key
            .clone()
            .or_else(|| provider.api_key_env.as_ref().and_then(|e| env::var(e).ok()));
        if provider.r#type == "anthropic" {
            if let Some(key) = key {
                request = request.header("x-api-key", key);
            }
            request = request.header("anthropic-version", "2023-06-01");
        } else if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        match request.send().await {
            Ok(response) => println!("  probe {probe_url}: HTTP {}", response.status()),
            Err(e) => println!("  probe {probe_url}: unreachable ({e})"),
        }
    }
    Ok(())
}

fn doctor_install() {
    let dir = install::default_bin_dir();
    let target = dir.join(install::exe_name());
    let installed = target.is_file();
    let in_path = install::user_path_contains(&dir);
    println!("\ninstall:");
    println!("  bin dir       : {}", dir.display());
    println!(
        "  installed     : {}",
        if installed {
            "yes"
        } else {
            "no (run `firm install`)"
        }
    );
    println!(
        "  PATH includes : {}",
        if in_path {
            "yes"
        } else {
            "no (run `firm install`, then open a new terminal)"
        }
    );
    match std::env::current_exe() {
        Ok(current) => {
            let running_installed = installed
                && std::fs::canonicalize(&current).ok() == std::fs::canonicalize(&target).ok();
            println!(
                "  running from  : {} ({})",
                current.display(),
                if running_installed {
                    "installed copy"
                } else {
                    "other location"
                }
            );
        }
        Err(e) => println!("  running from  : unknown ({e})"),
    }
    println!(
        "  config dir    : {} ({})",
        firment_core::config_dir().display(),
        if firment_core::config_dir().is_dir() {
            "ok"
        } else {
            "not created yet"
        }
    );
}

fn load_config(cli: &Cli) -> anyhow::Result<Config> {
    let config_path = cli.config.clone().unwrap_or_else(config_path);
    Ok(Config::load_or_create(&config_path)?)
}

/// Run a tool directly with the user's explicit invocation (firm build/flash):
/// permission is granted, dangerous guard still applies inside the tools.
async fn run_direct_tool(
    config: &Config,
    cwd: Option<PathBuf>,
    tool: &str,
    args: serde_json::Value,
) -> anyhow::Result<()> {
    let cwd = cwd.unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let ctx = firment_core::ToolContext {
        cwd: cwd.clone(),
        permission: Arc::new(firment_core::AutoApprove::everything()),
        allow_dangerous: true,
        journal: Arc::new(Mutex::new(firment_core::EditJournal::new(
            env::temp_dir().join("firm-cli-journal"),
        ))),
        verify_command: config.tools.verify_command.clone(),
        symbols_backend: config.tools.symbols_backend.clone(),
        build_command: config.tools.build_command.clone(),
        default_chip: config.tools.default_chip.clone(),
        monitor_port: config.tools.monitor_port.clone(),
        monitor_baud: config.tools.monitor_baud,
        subagent: None,
        subagent_depth: 0,
        max_subagent_depth: 2,
        asker: None,
        web_search_provider: config.tools.web_search.clone(),
        web_search_api_key: config.tools.resolved_web_search_api_key(),
        session_dir: None,
        allowed_roots: Vec::new(),
        cancel: firment_core::Cancellable::new(),
    };
    let registry = firment_tools::default_registry();
    match registry.run(tool, args, &ctx).await {
        Ok(output) => {
            println!("{}", output.text);
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!("{}", e.message)),
    }
}

/// Read a serial port and print lines, optionally decoding hex code
/// addresses against an ELF symbol table. Blocking CLI helper.
fn run_monitor(
    port: &str,
    baud: u32,
    elf: Option<PathBuf>,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    use std::io::Read;
    use std::time::{Duration, Instant};
    let mut reader = serialport::new(port, baud)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| anyhow::anyhow!("failed to open serial port {port}: {e}"))?;
    let elf = elf.as_deref();
    let mut buf = [0u8; 4096];
    let mut line_buf = String::new();
    let deadline = if timeout_secs > 0 {
        Some(Instant::now() + Duration::from_secs(timeout_secs))
    } else {
        None
    };
    loop {
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            break;
        }
        match reader.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => {
                for ch in String::from_utf8_lossy(&buf[..n]).chars() {
                    if ch == '\n' {
                        println!("{}", firment_tools::decode::decode_line(&line_buf, elf));
                        line_buf.clear();
                    } else {
                        line_buf.push(ch);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(anyhow::anyhow!("serial read failed: {e}")),
        }
    }
    if !line_buf.is_empty() {
        println!("{}", firment_tools::decode::decode_line(&line_buf, elf));
    }
    Ok(())
}

fn parse_thinking(s: &str) -> Result<ThinkingLevel, std::io::Error> {
    s.parse()
}

fn format_ts(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| secs.to_string())
}
