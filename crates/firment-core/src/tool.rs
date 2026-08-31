use crate::ask::Asker;
use crate::cancel::Cancellable;
use crate::journal::EditJournal;
use crate::subagent::SubagentFactory;
use crate::{PermissionChecker, PermissionError, ToolSpec};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub permission: Arc<dyn PermissionChecker>,
    /// Allow destructive shell commands (rm/del/git clean, etc.) without the
    /// hard guard. Interactive TUI enables this so the normal permission
    /// popup stays the decision point; one-shot `-y` keeps it disabled unless
    /// the user passes `--allow-dangerous`.
    pub allow_dangerous: bool,
    /// Per-turn edit journal: backups + rollback for write/edit tools.
    pub journal: Arc<Mutex<EditJournal>>,
    /// Configured verification command from `[tools] verify_command`.
    pub verify_command: Option<String>,
    /// Extra roots (besides cwd) that file tools may access, e.g. the
    /// session's spill directory. Paths outside cwd + these roots are rejected.
    pub allowed_roots: Vec<PathBuf>,
    /// Symbol index backend override: `auto` / `ctags` / `regex`.
    pub symbols_backend: Option<String>,
    /// Configured build command from `[tools] build_command`.
    pub build_command: Option<String>,
    /// Default target chip for the flash tool from `[tools] default_chip`.
    pub default_chip: Option<String>,
    /// Serial port for the monitor tool from `[tools] monitor_port`.
    pub monitor_port: Option<String>,
    /// Baud rate for the monitor tool from `[tools] monitor_baud`.
    pub monitor_baud: u32,
    /// Nested-agent runner for the `task` tool; `None` in direct tool runs.
    pub subagent: Option<Arc<dyn SubagentFactory>>,
    /// Attacker-profile runner for the `redteam` campaign: same plumbing as
    /// `subagent` but with the hardware-capable registry, so the campaign
    /// can clone it and swap in the target-locked permission. `None` in
    /// direct tool runs and plan mode.
    pub attacker: Option<Arc<crate::subagent::SubagentRunner>>,
    /// Current subagent nesting depth (0 = main agent).
    pub subagent_depth: usize,
    /// Recursion limit for nested agents from `[tools] max_subagent_depth`.
    pub max_subagent_depth: usize,
    /// Interactive user front-end for the `ask_user` tool.
    pub asker: Option<Arc<dyn Asker>>,
    /// Web search provider name from `[tools] web_search`.
    pub web_search_provider: Option<String>,
    /// Resolved web search API key (inline config or env var).
    pub web_search_api_key: Option<String>,
    /// Per-session directory for tool bookkeeping (e.g. the todo list).
    pub session_dir: Option<PathBuf>,
    /// This session's change ledger, when the embedder runs the tool inside
    /// a session. Fault forensics correlates the captured scene against
    /// recent changes through it; None for session-less direct invocations.
    pub ledger_path: Option<PathBuf>,
    /// Turn-level cooperative cancellation signal. Long-running tools poll
    /// `cancelled()` and terminate child processes when it fires.
    pub cancel: Cancellable,
    /// Directory holding the desktop MQTT link's device-log files
    /// (device-log-<date>.jsonl). `None` falls back to the global config
    /// dir; tests inject a temp dir.
    pub device_log_dir: Option<PathBuf>,
    /// OpenAI-compatible endpoints from config.toml [providers], with keys
    /// already resolved (inline → env → auth.json). Backs the `models`
    /// discovery tool so the agent can see what each backend serves.
    pub providers: Vec<ProviderEndpoint>,
    /// Logic-analyzer defaults from config.toml [tools.la] (sigrok driver,
    /// samplerate, channel spec, sample cap). `None` = not configured; the
    /// `la` tool then requires every parameter explicitly.
    pub la: Option<crate::config::LaConfig>,
}

/// One callable model endpoint for the `models` discovery tool.
#[derive(Clone, Debug)]
pub struct ProviderEndpoint {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ToolContext {
    /// Convenience constructor with safe defaults; tests and direct tool runs
    /// override fields afterwards. Permission defaults to deny-all (fail
    /// closed) so a caller that forgets to set a permission checker cannot
    /// accidentally auto-approve mutating tools.
    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self {
            cwd,
            permission: Arc::new(crate::AutoApprove::nothing()),
            allow_dangerous: false,
            journal: Arc::new(Mutex::new(EditJournal::new(
                std::env::temp_dir().join("firment-journal"),
            ))),
            verify_command: None,
            allowed_roots: Vec::new(),
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            subagent: None,
            attacker: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            web_search_provider: None,
            web_search_api_key: None,
            session_dir: None,
            ledger_path: None,
            cancel: Cancellable::new(),
            device_log_dir: None,
            providers: Vec::new(),
            la: None,
        }
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::with_cwd(PathBuf::from("."))
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ToolError {
    pub message: String,
    /// Whether the call was rejected before execution (user denied approval
    /// or the permission check failed). A denied call mutates nothing, so the
    /// harness must not count it as a mutation or roll back on it.
    pub denied: bool,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ToolError {}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: false,
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denied: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;

    /// Return a human-readable reason if this invocation needs explicit approval.
    fn approval(&self, _args: &Value) -> Option<String> {
        None
    }

    /// Optional unified-diff preview appended to the approval prompt (used by
    /// write/edit tools so the user sees exactly what will change).
    fn preview(&self, _args: &Value, _ctx: &ToolContext) -> Option<String> {
        None
    }

    async fn run(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError>;
}

pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self
            .tools
            .values()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub async fn run(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::new(format!("unknown tool: {name}")))?;
        crate::schema::validate_args(tool.name(), &tool.input_schema(), &args)
            .map_err(ToolError::new)?;
        let mut reason = tool.approval(&args);
        if let Some(preview) = tool.preview(&args, ctx)
            && let Some(reason) = reason.as_mut()
        {
            reason.push_str(&format!("\n{preview}"));
        }
        if let Some(reason) = reason {
            match ctx.permission.confirm(tool.name(), &args, &reason).await {
                Ok(()) => {}
                Err(PermissionError::Denied(message)) => {
                    return Err(ToolError::denied(format!("Permission denied: {message}")));
                }
                Err(PermissionError::Io(e)) => {
                    return Err(ToolError::denied(format!("Permission check failed: {e}")));
                }
            }
        }
        tool.run(args, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
