use crate::cancel::Cancellable;
use crate::config::{CompactionStrategy, ElfConfig};
use crate::journal::{EditJournal, Ledger};
use crate::provider::{Provider, ProviderError, ProviderEvent};
use crate::session::{SessionStore, SessionSummary};
use crate::tool::{ToolContext, ToolRegistry};
use crate::types::{ChatMessage, ChatRequest, SessionMode, ThinkingLevel, ToolCall};
use crate::{Asker, PermissionChecker, Session, SubagentFactory, system_prompt_for};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStart,
    TextDelta(String),
    /// Live extended-thinking snippet. UI-only: never persisted and never
    /// fed back into the transcript.
    Thinking(String),
    ToolStart {
        name: String,
        args: Value,
        /// Monotonic per-start id; ToolEnd carries the same id so concurrent
        /// same-name tool calls resolve to their own cards.
        seq: u64,
    },
    ToolEnd {
        name: String,
        ok: bool,
        summary: String,
        seq: u64,
    },
    TurnEnd {
        text: String,
    },
    /// Non-fatal status/info message shown in the UI (e.g. config changes).
    Info(String),
    /// Settings changed; UI should update its status bar.
    Settings {
        provider: Option<String>,
        model: Option<String>,
        thinking: Option<ThinkingLevel>,
        mode: Option<SessionMode>,
    },
    /// Model list fetched from the provider (for the model picker).
    Models(Vec<String>),
    /// Saved sessions fetched for the session picker.
    Sessions(Vec<SessionSummary>),
    /// A session was loaded/switched; the UI should repopulate its transcript.
    SessionLoaded(Session),
    Error(String),
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn event(&self, event: AgentEvent);
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("session error: {0}")]
    Session(#[from] crate::session::SessionError),
    #[error("reached max iterations ({0})")]
    MaxIterations(usize),
    #[error("provider not configured: run /apikey <key> or /provider <name> inside the TUI")]
    NoProvider,
    #[error("no final text produced")]
    NoOutput,
}

/// Inactivity timeout for a single provider stream call: the stream is
/// considered stalled when no event (text chunk or tool call) arrives for
/// this long. This catches dead sockets, a model that never finishes
/// thinking, and connections dropped mid-stream — the stream would never
/// return on its own, so without this the turn hangs and the IDE/TUI never
/// sees TurnEnd.
///
/// This is a "no-events" timeout, not an absolute wall-clock cap: a stream
/// that keeps delivering tokens (however slowly) keeps resetting it. That is
/// deliberate — slow-but-progressing output is legitimate for real LLM
/// providers, while a genuinely wedged stream emits nothing and is bounded
/// here.
const STREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// Hard upper bound on a single tool wave. Every tool carries its own
/// internal timeout (shell/build/run/flash/monitor), but a few cannot bound
/// themselves: the `task` subagent inherits its provider's stream behaviour
/// and `ask_user` waits on a human. This cap guarantees the wave always
/// makes progress even if one tool wedges.
const TOOL_WAVE_TIMEOUT: Duration = Duration::from_secs(600);

/// After the wave timeout fires, the shared cancel signal is signalled so
/// cooperating tools (shell/run/build kill their process trees) get a short
/// window to wind down before the wave futures are dropped.
const TOOL_CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Verdict of the binary-analysis gate for the latest edit batch.
pub enum ElfGateOutcome {
    /// No diff worth surfacing: below thresholds (and benign reports are
    /// off), or nothing changed. Completion proceeds normally.
    Silent,
    /// A below-threshold diff worth reviewing (`report_benign`); the model
    /// sees it as a soft review round.
    Report(String),
    /// The diff exceeds configured thresholds; completion is not accepted
    /// until the user approves it (or, headless + strict, until fixed).
    Blocked(String),
}

/// Classify a raw `elf_analyze` tool output (which carries a machine-readable
/// `[GATE:...]` marker when threshold args were passed) into a gate verdict.
fn classify_gate(text: &str, cfg: &ElfConfig) -> ElfGateOutcome {
    if let Some(rest) = text.strip_prefix("[GATE:BLOCK]") {
        return ElfGateOutcome::Blocked(rest.trim_start().to_string());
    }
    if let Some(rest) = text.strip_prefix("[GATE:OK]") {
        if cfg.report_benign {
            return ElfGateOutcome::Report(rest.trim_start().to_string());
        }
        return ElfGateOutcome::Silent;
    }
    if text.starts_with("[GATE:CLEAN]") {
        return ElfGateOutcome::Silent;
    }
    // Fail closed: run_elf_gate always passes threshold args, so elf_analyze
    // must return a [GATE:...] marker. A missing marker means the gate verdict
    // could not be determined (tool output drift / regression), so treat it as
    // blocking rather than silently downgrading to a soft review.
    ElfGateOutcome::Blocked(format!(
        "gate verdict missing ([GATE:...] marker not found); treating as blocked:\n{text}"
    ))
}

/// How a blocking elf-gate diff is resolved.
enum ElfGateDecision {
    /// The user explicitly approved the change; completion proceeds.
    Allow,
    /// Keep fixing: continue the loop (interactive "fix", or headless
    /// strict). Completion stays blocked until the gate clears.
    Fix,
    /// Headless non-strict: degrade to a soft report — the model sees the
    /// injected diff and decides; completion is allowed.
    SoftReview,
}

pub struct Agent {
    provider: Option<Box<dyn Provider>>,
    registry: Arc<ToolRegistry>,
    session: Session,
    store: SessionStore,
    permission: Arc<dyn PermissionChecker>,
    sink: Arc<dyn EventSink>,
    cancel_tx: watch::Sender<bool>,
    /// Persistent receiver that keeps the cancel watch channel open even
    /// between turns: `Sender::send` silently fails — leaving the stale value
    /// in place — when every receiver has been dropped, which would leave the
    /// next turn permanently marked cancelled.
    #[allow(dead_code)] // kept alive solely to hold the channel open for `cancel_tx`
    cancel_rx: watch::Receiver<bool>,
    /// Turn-level cancellation signal shared with tools (and child agents).
    /// `Agent::cancel` sets both this and the watch channel.
    cancel: Cancellable,
    max_iterations: usize,
    allow_dangerous: bool,
    verify_command: Option<String>,
    context_budget_chars: usize,
    /// Highest ledger sequence already merged into the conversation.
    ledger_seq_appended: u64,
    compaction_strategy: CompactionStrategy,
    symbols_backend: Option<String>,
    build_command: Option<String>,
    default_chip: Option<String>,
    monitor_port: Option<String>,
    monitor_baud: u32,
    /// Nested-agent runner exposed to the `task` tool.
    subagent: Option<Arc<dyn SubagentFactory>>,
    /// Current subagent nesting depth (0 = main agent).
    subagent_depth: usize,
    /// Recursion limit for the `task` tool.
    max_subagent_depth: usize,
    /// Interactive user front-end exposed to the `ask_user` tool.
    asker: Option<Arc<dyn Asker>>,
    /// Web search provider + resolved API key exposed to the web_search tool.
    web_search_provider: Option<String>,
    /// Monotonic counter for tool-start/end event pairing.
    tool_seq: u64,
    web_search_api_key: Option<String>,
    /// Per-session bookkeeping directory (todo list etc.).
    session_dir: Option<PathBuf>,
    /// ELF binary-analysis gate policy: glob + thresholds. When set, the
    /// harness seeds a baseline and auto-runs `elf_analyze` before a finished
    /// turn is accepted; diffs above the thresholds block completion.
    elf_config: Option<ElfConfig>,
    /// True once the auto elf gate has run for the current edit batch; reset
    /// whenever a new mutation lands. Independent of `mutations_since_verify`
    /// so the gate still runs after the model verifies via the verify tool.
    elf_gate_done: bool,
    /// True when a mutation landed since the last elf analysis; gates the
    /// binary-analysis diff without depending on verify-bookkeeping.
    elf_gate_dirty: bool,
    /// True while the elf gate has blocked completion (threshold exceeded,
    /// not yet approved by the user). Until a re-run of the gate clears it,
    /// finishing the turn is not accepted.
    elf_gate_required: bool,
    /// Hash of the last read result per path, for unchanged-read dedup.
    read_hashes: HashMap<PathBuf, String>,
    /// Recently read paths (most recent last), for post-compact re-injection.
    recent_read_paths: VecDeque<PathBuf>,
    /// No-events cap for a provider stream (creation and mid-stream). See
    /// `STREAM_TIMEOUT`. Configurable so tests can use tiny values.
    stream_timeout: Duration,
    /// Hard cap on a single tool wave. See `TOOL_WAVE_TIMEOUT`.
    tool_wave_timeout: Duration,
    /// Grace window after the wave timeout fires. See `TOOL_CANCEL_GRACE`.
    tool_cancel_grace: Duration,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Option<Box<dyn Provider>>,
        registry: Arc<ToolRegistry>,
        session: Session,
        store: SessionStore,
        permission: Arc<dyn PermissionChecker>,
        sink: Arc<dyn EventSink>,
        max_iterations: usize,
    ) -> Self {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        Self {
            provider,
            registry,
            session,
            store,
            permission,
            sink,
            cancel_tx,
            cancel_rx,
            cancel: Cancellable::new(),
            max_iterations,
            allow_dangerous: false,
            verify_command: None,
            context_budget_chars: 256 * 1024,
            ledger_seq_appended: 0,
            compaction_strategy: CompactionStrategy::default(),
            symbols_backend: None,
            build_command: None,
            default_chip: None,
            monitor_port: None,
            monitor_baud: 115_200,
            subagent: None,
            subagent_depth: 0,
            max_subagent_depth: 2,
            asker: None,
            web_search_provider: None,
            tool_seq: 0,
            web_search_api_key: None,
            session_dir: None,
            elf_config: None,
            elf_gate_done: false,
            elf_gate_dirty: false,
            elf_gate_required: false,
            read_hashes: HashMap::new(),
            recent_read_paths: VecDeque::new(),
            stream_timeout: STREAM_TIMEOUT,
            tool_wave_timeout: TOOL_WAVE_TIMEOUT,
            tool_cancel_grace: TOOL_CANCEL_GRACE,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_store(&self) -> &SessionStore {
        &self.store
    }

    /// The active tool registry (plan mode swaps in the read-only set).
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// The configured completion-gate command, if any.
    pub fn verify_command(&self) -> Option<&str> {
        self.verify_command.as_deref()
    }

    /// Maximum research-subagent nesting depth.
    pub fn max_subagent_depth(&self) -> usize {
        self.max_subagent_depth
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.session.model = model.into();
    }

    pub fn set_provider(&mut self, provider: Box<dyn Provider>) {
        self.provider = Some(provider);
    }

    pub fn set_provider_name(&mut self, name: impl Into<String>) {
        self.session.provider = name.into();
    }

    pub fn set_thinking(&mut self, level: ThinkingLevel) {
        self.session.thinking = level;
    }

    /// Request cancellation of the currently running turn. The agent stops at
    /// the next safe checkpoint (provider stream boundary / iteration start /
    /// tool-wave boundary) and long-running tools kill their child processes.
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
        self.cancel.cancel();
    }

    /// Clear a pending cancellation request before starting a new turn.
    pub fn reset_cancel(&self) {
        let _ = self.cancel_tx.send(false);
        self.cancel.reset();
    }

    /// Clone of the turn-level cancellation signal, used to propagate a
    /// parent agent's cancel into nested agents.
    pub fn cancel_signal(&self) -> Cancellable {
        self.cancel.clone()
    }

    /// Override the provider-stream no-events cap (default `STREAM_TIMEOUT`).
    /// Tests use tiny values to exercise the stall/timeout paths quickly.
    pub fn set_stream_timeout(&mut self, timeout: Duration) {
        self.stream_timeout = timeout;
    }

    /// Override the tool-wave hard cap (default `TOOL_WAVE_TIMEOUT`).
    pub fn set_tool_wave_timeout(&mut self, timeout: Duration) {
        self.tool_wave_timeout = timeout;
    }

    /// Override the tool-wave cancel grace window (default `TOOL_CANCEL_GRACE`).
    pub fn set_tool_cancel_grace(&mut self, grace: Duration) {
        self.tool_cancel_grace = grace;
    }

    /// Handles to cancel the currently running turn from outside (e.g. a test
    /// that spawned the agent task): `send(true)` on the watch channel arms
    /// the iteration/stream checkpoints and `cancel()` fires the tool-layer
    /// signal. Prefer `Agent::cancel` when the agent is still reachable.
    pub fn cancel_handle(&self) -> (watch::Sender<bool>, Cancellable) {
        (self.cancel_tx.clone(), self.cancel.clone())
    }

    pub fn set_allow_dangerous(&mut self, allow: bool) {
        self.allow_dangerous = allow;
    }

    /// Set the configured `[tools] verify_command` exposed to the verify tool.
    pub fn set_verify_command(&mut self, command: Option<String>) {
        self.verify_command = command;
    }

    /// Set the approximate character budget for session context. Older
    /// messages are compacted into a digest when the budget is exceeded.
    pub fn set_context_budget_chars(&mut self, budget: usize) {
        self.context_budget_chars = budget;
    }

    /// Rough per-part context usage (char counts) for the `/context`
    /// command: system prompt, tool schemas, and message history, against
    /// the current budget.
    pub fn context_usage(&self) -> String {
        let system_chars = crate::context::system_prompt_for(&self.session.cwd, self.session.mode)
            .chars()
            .count();
        let tools_chars = serde_json::to_string(&self.registry.specs())
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let messages_chars: usize = self.session.messages.iter().map(message_size).sum();
        let total = system_chars + tools_chars + messages_chars;
        let budget = self.context_budget_chars.max(1);
        let pct = total as f64 * 100.0 / budget as f64;
        format!(
            "context usage ({total} chars, {pct:.0}% of budget {budget}):\n  system prompt: {system_chars}\n  tool schemas:  {tools_chars}\n  messages:      {messages_chars}"
        )
    }

    /// Set the auto-compaction strategy (summarize / drop / off).
    pub fn set_compaction_strategy(&mut self, strategy: CompactionStrategy) {
        self.compaction_strategy = strategy;
    }

    /// Set the symbol index backend override (auto / ctags / regex).
    pub fn set_symbols_backend(&mut self, backend: Option<String>) {
        self.symbols_backend = backend;
    }

    /// Set the configured build command exposed to the build tool.
    pub fn set_build_command(&mut self, command: Option<String>) {
        self.build_command = command;
    }

    /// Set the default target chip for the flash tool.
    pub fn set_default_chip(&mut self, chip: Option<String>) {
        self.default_chip = chip;
    }

    /// Set the serial port for the monitor tool.
    pub fn set_monitor_port(&mut self, port: Option<String>) {
        self.monitor_port = port;
    }

    /// Set the baud rate for the monitor tool.
    pub fn set_monitor_baud(&mut self, baud: u32) {
        self.monitor_baud = baud;
    }

    /// Set the nested-agent runner exposed to the `task` tool.
    pub fn set_subagent_factory(&mut self, factory: Option<Arc<dyn SubagentFactory>>) {
        self.subagent = factory;
    }

    /// Set the current subagent nesting depth (used by nested agents).
    pub fn set_subagent_depth(&mut self, depth: usize) {
        self.subagent_depth = depth;
    }

    /// Set the `[tools] max_subagent_depth` recursion limit for the task tool.
    pub fn set_max_subagent_depth(&mut self, depth: usize) {
        self.max_subagent_depth = depth.max(1);
    }

    /// Set the interactive user front-end exposed to the `ask_user` tool.
    pub fn set_asker(&mut self, asker: Option<Arc<dyn Asker>>) {
        self.asker = asker;
    }

    /// Set the web search provider and resolved API key for the web_search tool.
    pub fn set_web_search(&mut self, provider: Option<String>, api_key: Option<String>) {
        self.web_search_provider = provider;
        self.web_search_api_key = api_key;
    }

    /// Set the per-session bookkeeping directory (todo list etc.).
    pub fn set_session_dir(&mut self, dir: Option<PathBuf>) {
        self.session_dir = dir;
    }

    /// Set the `[tools] elf` binary-analysis gate policy; when set, the
    /// harness seeds an ELF baseline and auto-runs `elf_analyze` before a
    /// finished turn is accepted, blocking when thresholds are exceeded.
    pub fn set_elf_config(&mut self, config: Option<ElfConfig>) {
        self.elf_config = config;
    }

    /// At turn start, refresh the ELF baseline so edits are diffed against the
    /// state the turn began with. Silent except on tool errors.
    async fn seed_elf_baseline(&mut self, ctx: &ToolContext) {
        let Some(cfg) = self.elf_config.clone() else {
            return;
        };
        let Some(tool) = self.registry.get("elf_analyze") else {
            return;
        };
        let Some(elf) = newest_elf_match(&self.session.cwd, &cfg.glob) else {
            return;
        };
        if let Err(e) = tool
            .run(json!({ "file": elf.to_string_lossy() }), ctx)
            .await
        {
            self.sink
                .event(AgentEvent::Info(format!(
                    "elf baseline skipped: {}",
                    e.message
                )))
                .await;
        }
    }

    /// Binary-analysis gate: run `elf_analyze` once per edit batch against the
    /// newest ELF matching the configured glob, with the configured
    /// thresholds. `Silent` means the diff is below thresholds; `Blocked`
    /// means a threshold is exceeded and completion must not be accepted
    /// without an explicit user approval (or, headless + strict, without the
    /// gate clearing).
    async fn run_elf_gate(&mut self, ctx: &ToolContext) -> Option<ElfGateOutcome> {
        let cfg = self.elf_config.clone()?;
        let tool = self.registry.get("elf_analyze")?;
        if self.elf_gate_done {
            return None;
        }
        let elf = newest_elf_match(&self.session.cwd, &cfg.glob)?;
        self.elf_gate_done = true;
        let args = json!({
            "file": elf.to_string_lossy(),
            "stack_threshold": cfg.stack_threshold,
            "flash_threshold_kib": cfg.flash_threshold_kib,
            "ram_threshold_kib": cfg.ram_threshold_kib,
        });
        match tool.run(args, ctx).await {
            Ok(out) => Some(classify_gate(&out.text, &cfg)),
            // Fail CLOSED: a crashing/misconfigured elf_analyze must not
            // silently convert the blocking gate into an advisory report —
            // surface the tool error as a block so the model (or user) fixes
            // the analyzer itself.
            Err(e) => Some(ElfGateOutcome::Blocked(format!(
                "[GATE:BLOCK] elf_analyze failed, gate result unknown: {}",
                e.message
            ))),
        }
    }

    /// Inject a gate report as a real tool round (assistant tool_use -> tool
    /// result) replacing the plain assistant message, so the transcript stays
    /// a valid provider message sequence. The call id is unique per
    /// injection: the gate can re-inject after a fix cycle, and providers
    /// expect tool call ids not to repeat within a conversation.
    async fn inject_elf_report(&mut self, content: &str, text: &str) {
        self.session.messages.pop();
        self.tool_seq += 1;
        let gate_call = ToolCall {
            id: format!("elf_gate_{}", self.tool_seq),
            name: "elf_analyze".to_string(),
            arguments: json!({}),
        };
        self.session.push(ChatMessage::Assistant {
            content: content.to_string(),
            tool_calls: vec![gate_call.clone()],
        });
        self.session.push(ChatMessage::Tool {
            tool_call_id: gate_call.id,
            name: "elf_analyze".to_string(),
            content: text.to_string(),
        });
    }

    /// Resolve a blocking elf-gate diff. Interactive front-ends (TUI, IDE)
    /// get an allow / fix / detail choice; headless runs degrade to `Fix`
    /// (strict: blocked until fixed; non-strict: soft report semantics, the
    /// model reviews the injected diff and decides).
    async fn ask_elf_gate(&mut self, text: &str) -> ElfGateDecision {
        let strict = self.elf_config.as_ref().is_some_and(|c| c.strict);
        let Some(asker) = &self.asker else {
            if strict {
                self.sink
                    .event(AgentEvent::Info(
                        "elf gate (strict): thresholds exceeded — completion is blocked until \
                         the gate clears"
                            .to_string(),
                    ))
                    .await;
                return ElfGateDecision::Fix;
            }
            return ElfGateDecision::SoftReview;
        };
        let mut detail_shown = false;
        loop {
            let answer = asker
                .ask(
                    "ELF gate: the firmware diff exceeds the configured thresholds (stack \
                     depth or flash size). Approve this change or keep fixing?",
                    &["allow".to_string(), "fix".to_string(), "detail".to_string()],
                )
                .await;
            match answer.as_deref() {
                Ok("allow") => return ElfGateDecision::Allow,
                Ok("fix") | Err(_) => return ElfGateDecision::Fix,
                Ok("detail") if !detail_shown => {
                    detail_shown = true;
                    self.sink
                        .event(AgentEvent::Info(format!("ELF gate diff:\n{text}")))
                        .await;
                }
                _ => return ElfGateDecision::Fix,
            }
        }
    }

    /// Pin a file so compaction always re-injects its full content. Warns when
    /// the file is large relative to the context budget.
    pub fn pin_path(&self, path: PathBuf) -> Result<String, String> {
        let id = self.session.id.clone();
        let mut pins = self.store.load_pins(&id);
        if !pins.contains(&path) {
            pins.push(path.clone());
            self.store
                .save_pins(&id, &pins)
                .map_err(|e| e.to_string())?;
        }
        let mut message = format!(
            "Pinned {} (full content kept during compaction)",
            path.display()
        );
        if let Ok(meta) = fs::metadata(&path) {
            let budget = self.context_budget_chars.max(1);
            if meta.len() as usize >= budget * 30 / 100 {
                message.push_str(&format!(
                    "\n⚠ File is about {} KB, over 30% of the context budget; pinning it may \
                     crowd out summarized history — prefer pinning only key source files",
                    meta.len() / 1024
                ));
            }
        }
        Ok(message)
    }

    /// Remove a pinned file.
    pub fn unpin_path(&self, path: PathBuf) -> Result<String, String> {
        let id = self.session.id.clone();
        let mut pins = self.store.load_pins(&id);
        let before = pins.len();
        pins.retain(|p| p != &path);
        self.store
            .save_pins(&id, &pins)
            .map_err(|e| e.to_string())?;
        if pins.len() == before {
            Ok(format!("{} is not in the pinned list", path.display()))
        } else {
            Ok(format!("Unpinned {}", path.display()))
        }
    }

    /// Tool outputs above the threshold are spilled to the session's spill
    /// directory; the message keeps a short excerpt plus a `read_file` pointer.
    fn spill_text(&self, text: &str) -> String {
        const THRESHOLD: usize = 8000;
        const EXCERPT: usize = 2000;
        if text.chars().count() <= THRESHOLD {
            return text.to_string();
        }
        let dir = self.store.spill_dir(&self.session.id);
        // Spill files live only as long as the messages referencing them; on
        // each spill, drop files older than a day so long sessions do not
        // accumulate unbounded disk usage. Files still referenced by the
        // current transcript are kept even past the cutoff: deleting them
        // would make the stored spill path unreadable after a session reload.
        let referenced: std::collections::HashSet<String> = self
            .session
            .messages
            .iter()
            .flat_map(|m| match m {
                ChatMessage::System { content }
                | ChatMessage::User { content }
                | ChatMessage::Assistant { content, .. }
                | ChatMessage::Tool { content, .. } => Some(content.as_str()),
            })
            .flat_map(|t| t.split_whitespace())
            .filter(|w| w.starts_with("spill") || w.ends_with(".txt"))
            .map(|w| w.rsplit(['/', '\\']).next().unwrap_or(w).to_string())
            .collect();
        let _ = fs::create_dir_all(&dir);
        if let Ok(read) = fs::read_dir(&dir) {
            let cutoff =
                std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(24 * 3600));
            for entry in read.flatten() {
                let Ok(meta) = entry.metadata() else { continue };
                let expired = cutoff.is_some_and(|c| meta.modified().ok().is_some_and(|t| t < c));
                let file_name = entry.file_name().to_string_lossy().into_owned();
                if expired && !referenced.contains(&file_name) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        {
            let name = format!("{}.txt", uuid::Uuid::new_v4());
            let path = dir.join(&name);
            if fs::write(&path, text).is_ok() {
                let excerpt: String = text.chars().take(EXCERPT).collect();
                return format!(
                    "[output too long ({} chars); full content spilled to {}; use read_file to view]\n{}",
                    text.chars().count(),
                    path.display(),
                    excerpt
                );
            }
        }
        text.to_string()
    }

    /// Formatted change-ledger summary for display (e.g. `/ledger`).
    pub fn ledger_summary(&self) -> String {
        Ledger::new(self.store.ledger_path(&self.session.id)).summary(30, 6000)
    }

    /// Switch between agent and read-only plan mode. The caller supplies the
    /// matching tool registry and permission checker for the new mode.
    pub fn set_mode(
        &mut self,
        mode: SessionMode,
        registry: Arc<ToolRegistry>,
        permission: Arc<dyn PermissionChecker>,
    ) {
        self.session.mode = mode;
        self.registry = registry;
        self.permission = permission;
    }

    /// Replace the whole session (used when switching to another saved session
    /// inside the TUI). The caller is responsible for rebuilding the provider.
    /// Per-turn bookkeeping that is scoped to the old session must be reset,
    /// otherwise change-ledger deltas, read hashes and the elf gate from the
    /// previous session leak into the new one.
    pub fn replace_session(&mut self, session: Session) {
        self.session = session;
        self.ledger_seq_appended = 0;
        self.read_hashes.clear();
        self.recent_read_paths.clear();
        self.elf_gate_done = false;
        self.set_elf_config(None);
        self.elf_gate_required = false;
    }

    pub fn save_session(&self) -> Result<(), crate::session::SessionError> {
        self.store.save(&self.session)
    }

    pub async fn emit(&self, event: AgentEvent) {
        self.sink.event(event).await;
    }

    fn build_request(&self) -> ChatRequest {
        // Keep the system prompt byte-stable so provider prefix caching keeps
        // hitting; dynamic state (change ledger) is merged into user messages.
        let mut messages = vec![ChatMessage::System {
            content: system_prompt_for(&self.session.cwd, self.session.mode),
        }];
        messages.extend(self.session.messages.clone());
        ChatRequest {
            model: self.session.model.clone(),
            messages,
            tools: self.registry.specs(),
            max_tokens: None,
            temperature: None,
            thinking: thinking_opt(self.session.thinking),
        }
    }

    pub async fn run_turn(&mut self, input: &str) -> Result<String, AgentError> {
        // `subscribe()` (not `clone()`): the returned receiver's version is
        // pinned to the current channel version, so `changed()` only fires on
        // future sends. Cloning the persistent `cancel_rx` field would inherit
        // its stale version and make every turn look pre-cancelled.
        let mut cancel_rx = self.cancel_tx.subscribe();
        if *cancel_rx.borrow() {
            self.sink
                .event(AgentEvent::Info(
                    "⏹ Interrupted (no work started yet)".to_string(),
                ))
                .await;
            // Persist BEFORE TurnEnd: clients that refresh the transcript on
            // turn_end must not read a stale store.
            let _ = self.store.save(&self.session);
            self.sink
                .event(AgentEvent::TurnEnd {
                    text: String::new(),
                })
                .await;
            return Ok(String::new());
        }
        let (delta, last_seq) = Ledger::new(self.store.ledger_path(&self.session.id))
            .delta_text(self.ledger_seq_appended, 5);
        let input = if delta.is_empty() {
            input.to_string()
        } else {
            self.ledger_seq_appended = last_seq;
            format!("[change ledger]\n{delta}\n\n{input}")
        };
        self.session.push(ChatMessage::User { content: input });
        self.sink.event(AgentEvent::TurnStart).await;

        let journal = Arc::new(Mutex::new(EditJournal::new(
            self.store.undo_dir(&self.session.id),
        )));
        let ledger = Ledger::new(self.store.ledger_path(&self.session.id));
        let mut mutations_since_verify = 0usize;

        let ctx = ToolContext {
            cwd: self.session.cwd.clone(),
            permission: self.permission.clone(),
            allow_dangerous: self.allow_dangerous,
            journal: journal.clone(),
            verify_command: self.verify_command.clone(),
            allowed_roots: vec![
                self.store.spill_dir(&self.session.id),
                crate::kb::seed_kb_dir(),
            ],
            symbols_backend: self.symbols_backend.clone(),
            build_command: self.build_command.clone(),
            default_chip: self.default_chip.clone(),
            monitor_port: self.monitor_port.clone(),
            monitor_baud: self.monitor_baud,
            subagent: self.subagent.clone(),
            subagent_depth: self.subagent_depth,
            max_subagent_depth: self.max_subagent_depth,
            asker: self.asker.clone(),
            web_search_provider: self.web_search_provider.clone(),
            web_search_api_key: self.web_search_api_key.clone(),
            session_dir: self.session_dir.clone(),
            cancel: self.cancel.clone(),
        };

        self.seed_elf_baseline(&ctx).await;

        for _ in 0..self.max_iterations {
            self.compact_if_needed().await;
            if *cancel_rx.borrow() {
                self.sink
                    .event(AgentEvent::Info(interrupted_note(&journal)))
                    .await;
                let _ = self.store.save(&self.session);
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: String::new(),
                    })
                    .await;
                return Ok(String::new());
            }
            let request = self.build_request();
            let provider = self.provider.as_ref().ok_or(AgentError::NoProvider)?;
            let mut stream = tokio::select! {
                result = provider.stream(request) => match result {
                    Ok(stream) => stream,
                    // Stream CREATION can also fail (HTTP 429/500, DNS, ...)
                    // on a later iteration after earlier waves already edited
                    // files. Mirror the mid-stream error path: roll back,
                    // inform, persist — otherwise the edits are stranded
                    // neither committed nor undoable.
                    Err(e) => {
                        let message = match rollback_journal(&journal) {
                            Some(summary) => {
                                format!("provider error; rolled back this turn's edits: {summary}")
                            }
                            None => format!("provider error: {e}"),
                        };
                        self.sink.event(AgentEvent::Error(message)).await;
                        let _ = self.store.save(&self.session);
                        self.sink
                            .event(AgentEvent::TurnEnd {
                                text: String::new(),
                            })
                            .await;
                        return Err(AgentError::Provider(e));
                    }
                },
                _ = cancel_rx.changed() => {
                    self.sink
                        .event(AgentEvent::Info(interrupted_note(&journal)))
                        .await;
                    let _ = self.store.save(&self.session);
                    self.sink
                        .event(AgentEvent::TurnEnd {
                            text: String::new(),
                        })
                        .await;
                    return Ok(String::new());
                }
                // Hard timeout: if the provider stream never returns (dead
                // socket, model stalled, network blackhole) we must NOT
                // hang the whole turn — surface the failure to the UI and
                // end the turn so the user can react / retry.
                _ = tokio::time::sleep(self.stream_timeout) => {
                    let rollback_note = match rollback_journal(&journal) {
                        Some(summary) => format!(" Partial edits rolled back: {summary}"),
                        None => String::new(),
                    };
                    self.sink
                        .event(AgentEvent::Info(format!(
                            "⚠ Provider stream timed out after {}s; ending turn.{rollback_note}",
                            self.stream_timeout.as_secs()
                        )))
                        .await;
                    let _ = self.store.save(&self.session);
                    self.sink
                        .event(AgentEvent::TurnEnd {
                            text: String::new(),
                        })
                        .await;
                    return Ok(String::new());
                }
            };
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut cancelled = false;
            let mut stalled = false;

            while let Some(event) = tokio::select! {
                next = stream.next() => next,
                _ = cancel_rx.changed() => {
                    cancelled = true;
                    None
                }
                // Mid-stream stall: the provider produced events and then went
                // silent (dead socket, connection dropped between chunks). The
                // outer select only bounds stream *creation*; this bounds the
                // iteration itself so a stall can never wedge the turn.
                _ = tokio::time::sleep(self.stream_timeout) => {
                    stalled = true;
                    None
                }
            } {
                let event = match event {
                    Ok(event) => event,
                    Err(e) => {
                        // An empty journal means nothing was rolled back —
                        // saying "rolled back: no file changes" reads like a
                        // second failure on top of the real one.
                        let message = match rollback_journal(&journal) {
                            Some(summary) => {
                                format!("provider error; rolled back this turn's edits: {summary}")
                            }
                            None => format!("provider error: {e}"),
                        };
                        self.sink.event(AgentEvent::Error(message)).await;
                        return Err(AgentError::Provider(e));
                    }
                };
                match event {
                    ProviderEvent::Text(text) => {
                        content.push_str(&text);
                        self.sink.event(AgentEvent::TextDelta(text.clone())).await;
                    }
                    ProviderEvent::Thinking(text) => {
                        // Reasoning-native models emit thinking blocks even
                        // when the request never enabled them; honour the
                        // knob so "off" means the UI stays quiet (the tokens
                        // themselves are upstream behaviour we cannot stop).
                        if self.session.thinking != ThinkingLevel::Off {
                            self.sink.event(AgentEvent::Thinking(text)).await;
                        }
                    }
                    ProviderEvent::ToolCall(call) => tool_calls.push(call),
                    ProviderEvent::Stop { .. } => {}
                }
            }

            // Never persist tool calls that were never executed: an assistant
            // message with dangling tool_calls would make the next request an
            // invalid provider sequence. On cancel/stall, keep only the text.
            let saved_calls = if cancelled || stalled {
                Vec::new()
            } else {
                tool_calls.clone()
            };
            self.session.push(ChatMessage::Assistant {
                content: content.clone(),
                tool_calls: saved_calls,
            });

            if cancelled {
                self.sink
                    .event(AgentEvent::Info(interrupted_note(&journal)))
                    .await;
                let _ = self.store.save(&self.session);
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                return Ok(content);
            }

            if stalled {
                let rollback_note = match rollback_journal(&journal) {
                    Some(summary) => format!(" Partial edits rolled back: {summary}"),
                    None => String::new(),
                };
                self.sink
                    .event(AgentEvent::Info(format!(
                        "⚠ Provider stream stalled (no events for {}s); ending turn.{rollback_note}",
                        self.stream_timeout.as_secs()
                    )))
                    .await;
                let _ = self.store.save(&self.session);
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                return Ok(content);
            }

            if tool_calls.is_empty() {
                let plain_assistant = self.session.messages.pop().expect("assistant message");
                if mutations_since_verify > 0
                    && self.verify_command.is_some()
                    && self.registry.get("verify").is_some()
                {
                    self.tool_seq += 1;
                    let seq = self.tool_seq;
                    let gate_call = ToolCall {
                        id: format!("verify_gate_{seq}"),
                        name: "verify".to_string(),
                        arguments: json!({}),
                    };
                    self.session.push(ChatMessage::Assistant {
                        content: content.clone(),
                        tool_calls: vec![gate_call.clone()],
                    });
                    self.sink
                        .event(AgentEvent::ToolStart {
                            name: "verify".to_string(),
                            args: json!({}),
                            seq,
                        })
                        .await;
                    let result = self
                        .registry
                        .get("verify")
                        .expect("checked above")
                        .run(json!({}), &ctx)
                        .await;
                    let (ok, text) = match &result {
                        Ok(output) => (true, output.text.clone()),
                        Err(e) => (false, e.message.clone()),
                    };
                    self.sink
                        .event(AgentEvent::ToolEnd {
                            name: "verify".to_string(),
                            ok,
                            summary: summarize(&text),
                            seq,
                        })
                        .await;
                    self.session.push(ChatMessage::Tool {
                        tool_call_id: gate_call.id,
                        name: "verify".to_string(),
                        content: text,
                    });
                    if ok {
                        // Restore the clean transcript and finish normally.
                        self.session.messages.pop();
                        self.session.messages.pop();
                        self.session.push(plain_assistant);
                        mutations_since_verify = 0;
                    } else {
                        self.sink
                            .event(AgentEvent::Info(
                                "verify gate failed: fix the errors and retry; completion is \
                                 not accepted until verify passes"
                                    .to_string(),
                            ))
                            .await;
                        continue;
                    }
                } else {
                    self.session.push(plain_assistant);
                }

                if self.elf_gate_dirty
                    && let Some(outcome) = self.run_elf_gate(&ctx).await
                {
                    self.elf_gate_dirty = false;
                    match outcome {
                        ElfGateOutcome::Silent => {
                            self.elf_gate_required = false;
                        }
                        ElfGateOutcome::Report(text) => {
                            self.elf_gate_required = false;
                            // Fresh binary diff: surface it to the model so it
                            // can decide whether to accept the state or keep
                            // fixing. The report is injected as a real tool
                            // round (assistant tool_use -> tool result) so the
                            // transcript stays a valid provider sequence.
                            self.inject_elf_report(&content, &text).await;
                            self.sink
                                .event(AgentEvent::Info(
                                    "binary analysis: the firmware changed vs its baseline — \
                                     review the diff above and decide whether to accept or keep fixing"
                                        .to_string(),
                                ))
                                .await;
                            continue;
                        }
                        ElfGateOutcome::Blocked(text) => {
                            self.inject_elf_report(&content, &text).await;
                            match self.ask_elf_gate(&text).await {
                                ElfGateDecision::Allow => {
                                    self.elf_gate_required = false;
                                    // Keep the transcript ending on an assistant
                                    // message; the change is approved, so fall
                                    // through to commit and finish the turn.
                                    self.session.push(ChatMessage::Assistant {
                                        content: content.clone(),
                                        tool_calls: vec![],
                                    });
                                    self.sink
                                        .event(AgentEvent::Info(
                                            "elf gate: change approved by the user — completing"
                                                .to_string(),
                                        ))
                                        .await;
                                }
                                ElfGateDecision::Fix => {
                                    self.elf_gate_required = true;
                                    self.sink
                                        .event(AgentEvent::Info(
                                            "elf gate: thresholds exceeded — completion is \
                                             blocked until the gate clears; keep fixing"
                                                .to_string(),
                                        ))
                                        .await;
                                    continue;
                                }
                                ElfGateDecision::SoftReview => {
                                    self.elf_gate_required = false;
                                    self.sink
                                        .event(AgentEvent::Info(
                                            "elf gate: thresholds exceeded (non-strict) — \
                                             review the diff above and decide"
                                                .to_string(),
                                        ))
                                        .await;
                                    continue;
                                }
                            }
                        }
                    }
                }
                // Hard gate: while a blocking diff is unresolved, the turn may
                // not finish even if the model stops editing and answers.
                if self.elf_gate_required {
                    self.sink
                        .event(AgentEvent::Info(
                            "elf gate: still blocked — fix the firmware before finishing"
                                .to_string(),
                        ))
                        .await;
                    continue;
                }

                let commit_result = lock_journal(&journal).commit();
                match commit_result {
                    Ok(changes) if !changes.is_empty() => {
                        if let Err(e) = ledger.append(&changes) {
                            self.sink
                                .event(AgentEvent::Info(format!(
                                    "failed to append change ledger: {e}"
                                )))
                                .await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        self.sink
                            .event(AgentEvent::Info(format!(
                                "failed to write edit journal: {e}"
                            )))
                            .await;
                    }
                }
                // The turn's real work (tools + journal commit) already
                // succeeded: a final save failure (e.g. Windows handle
                // contention on the session file) must not fail the whole
                // turn — surface it and still end cleanly.
                if let Err(e) = self.store.save(&self.session) {
                    self.sink
                        .event(AgentEvent::Error(format!(
                            "warning: could not persist the session transcript: {e}"
                        )))
                        .await;
                }
                // Persist BEFORE TurnEnd so transcript-refreshing clients see
                // the final message.
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                return Ok(content);
            }

            let stats = execute_tool_calls(self, &tool_calls, &ctx, &journal).await;
            if stats.timed_out {
                let rollback_note = match rollback_journal(&journal) {
                    Some(summary) => format!(" Partial edits rolled back: {summary}"),
                    None => String::new(),
                };
                self.sink
                    .event(AgentEvent::Info(format!(
                        "⚠ Tool wave timed out after {}s; ending turn.{rollback_note}",
                        self.tool_wave_timeout.as_secs()
                    )))
                    .await;
                let _ = self.store.save(&self.session);
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: String::new(),
                    })
                    .await;
                return Ok(String::new());
            }
            mutations_since_verify += stats.mutations;
            if stats.mutations > 0 {
                self.elf_gate_done = false;
                self.elf_gate_dirty = true;
            }
            // Only clear the counter when the wave verified AND mutated
            // nothing: a wave like [verify, edit_file] runs both concurrently
            // (no dependency edge between a broad tool and a later mutation),
            // so those edits landed unverified — or after the check ran.
            // Zeroing here would silently bypass the turn-end verify gate.
            if stats.verify_passed && stats.mutations == 0 {
                mutations_since_verify = 0;
            }
        }

        // Max iterations reached. Whether to keep or roll back the turn's
        // edits depends on WHY we got here:
        // - If a verify gate is configured and never passed
        //   (`mutations_since_verify > 0`), the workspace holds code that
        //   failed its checks — roll back rather than leave broken code.
        // - Otherwise every edit succeeded and the workspace is consistent;
        //   the task simply did not converge within the budget. Keep the
        //   edits (committed as an undo entry) so the user can keep typing or
        //   /undo instead of silently losing useful work.
        let unverified = self.verify_command.is_some() && mutations_since_verify > 0;
        let outcome = if unverified {
            match rollback_journal(&journal) {
                Some(summary) => {
                    format!("rolled back this turn's edits (verify never passed): {summary}")
                }
                None => {
                    "verify never passed, and no file changes were recorded this turn".to_string()
                }
            }
        } else {
            match lock_journal(&journal).commit() {
                Ok(changes) if !changes.is_empty() => {
                    if let Err(e) =
                        Ledger::new(self.store.ledger_path(&self.session.id)).append(&changes)
                    {
                        format!(
                            "kept edits to {} file(s), but failed to append the change ledger: {e}",
                            changes.len()
                        )
                    } else {
                        format!(
                            "kept edits to {} file(s); keep typing to continue the task, or /undo to revert this turn",
                            changes.len()
                        )
                    }
                }
                Ok(_) => "no file changes were recorded this turn".to_string(),
                Err(e) => format!("failed to finalize the edit journal: {e}"),
            }
        };
        self.sink
            .event(AgentEvent::Info(format!(
                "reached max iterations ({max}); {outcome}",
                max = self.max_iterations
            )))
            .await;
        let _ = self.store.save(&self.session);
        Err(AgentError::MaxIterations(self.max_iterations))
    }

    /// Approximate character budget for session context; older messages are
    /// compacted into a digest when exceeded.
    async fn compact_if_needed(&mut self) {
        if self.compaction_strategy == CompactionStrategy::Off {
            return;
        }
        const DIGEST_CHARS: usize = 6000;
        const ROUNDS_KEPT: usize = 3;
        const DROP_EXTRA_ROUNDS: usize = 5;
        let total: usize = self.session.messages.iter().map(message_size).sum();
        if total <= self.context_budget_chars {
            return;
        }
        let Some((cut, round_count)) = round_cut_index(&self.session.messages) else {
            return;
        };
        let mut drop_until = 0usize;
        if self.compaction_strategy == CompactionStrategy::Drop
            && round_count > ROUNDS_KEPT + DROP_EXTRA_ROUNDS
        {
            drop_until = round_starts_at(
                &self.session.messages,
                round_count - ROUNDS_KEPT - DROP_EXTRA_ROUNDS,
            );
        }
        let _dropped = self.session.messages.drain(..drop_until).count();
        let old = self
            .session
            .messages
            .drain(..(cut.saturating_sub(drop_until)))
            .collect::<Vec<_>>();
        let summary = match self.summarize_messages(&old).await {
            Some(summary) => summary,
            None => compact_summary(&old, DIGEST_CHARS),
        };
        // Merge the summary into the first surviving message instead of
        // prepending a separate User message: the original first message is
        // usually a User turn, and consecutive user roles break providers
        // that require strict role alternation (e.g. Anthropic returns 400).
        let mut content = format!("[compacted context] summary:\n{summary}");
        if drop_until > 0 {
            content.push_str(
                "\n\n(earlier conversation was dropped per the 'drop' strategy; no summary \
                 retained)",
            );
        }
        if let Some(files) = self.recent_read_files_text() {
            content.push_str(&format!("\n\n{files}"));
        }
        if let Some(pins) = self.pinned_files_text() {
            content.push_str(&format!("\n\n{pins}"));
        }
        let mut messages = Vec::with_capacity(self.session.messages.len() + 1);
        let mut inserted = false;
        for msg in self.session.messages.iter().cloned() {
            if !inserted && matches!(msg, ChatMessage::User { .. }) {
                let ChatMessage::User { content: old } = msg else {
                    unreachable!()
                };
                messages.push(ChatMessage::User {
                    content: format!("{}\n\n{old}", content),
                });
                inserted = true;
            } else {
                messages.push(msg);
            }
        }
        if !inserted {
            messages.insert(0, ChatMessage::User { content });
        }
        self.session.messages = messages;
    }

    /// Pinned files re-injected with full content after compaction.
    fn pinned_files_text(&self) -> Option<String> {
        const MAX_FILES: usize = 5;
        const MAX_CHARS: usize = 8000;
        let pins = self.store.load_pins(&self.session.id);
        if pins.is_empty() {
            return None;
        }
        let mut out = String::new();
        for path in pins.iter().take(MAX_FILES) {
            if let Ok(text) = fs::read_to_string(path) {
                let text: String = text.chars().take(MAX_CHARS).collect();
                out.push_str(&format!("### {}\n{text}\n\n", path.display()));
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(format!(
                "[pinned files (full content kept during compaction)]\n{out}"
            ))
        }
    }

    /// Ask the main provider to summarize the given messages (CC-style).
    async fn summarize_messages(&self, messages: &[ChatMessage]) -> Option<String> {
        let provider = self.provider.as_ref()?;
        let request = ChatRequest {
            model: self.session.model.clone(),
            messages: vec![
                ChatMessage::System {
                    content: "You are a helpful assistant tasked with summarizing conversations."
                        .to_string(),
                },
                ChatMessage::User {
                    content: "Summarize the conversation so far, in the language the user is \
                              using. Preserve: the user's goals and requirements; decisions and \
                              tradeoffs; files read with their key contents; files edited and \
                              exactly what changed; errors and fixes; commands run and outcomes; \
                              open questions and next steps. Be concrete and compact, prefer \
                              bullet points. The original messages will be removed from context, \
                              so keep enough detail to continue without re-reading everything."
                        .to_string(),
                },
            ]
            .into_iter()
            .chain(messages.iter().cloned())
            .collect(),
            tools: Vec::new(),
            max_tokens: Some(2048),
            temperature: None,
            thinking: None,
        };
        let mut stream = provider.stream(request).await.ok()?;
        let mut text = String::new();
        // The main provider stream is bounded by `stream_timeout` in run_turn,
        // but this auxiliary summarization request is not; bound it here so a
        // blackholed summary cannot hang the whole turn.
        while let Some(event) = tokio::time::timeout(self.stream_timeout, stream.next())
            .await
            .ok()
            .flatten()
        {
            if let Ok(ProviderEvent::Text(part)) = event {
                text.push_str(&part);
            }
        }
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Recently read files (paths only), re-read fresh and capped, to re-inject
    /// after compaction so the model does not lose file contents (CC-style).
    fn recent_read_files_text(&self) -> Option<String> {
        const MAX_FILES: usize = 3;
        const MAX_CHARS: usize = 4000;
        let mut out = String::new();
        let mut count = 0;
        for path in self.recent_read_paths.iter().rev() {
            if count >= MAX_FILES {
                break;
            }
            if let Ok(text) = fs::read_to_string(path) {
                let text: String = text.chars().take(MAX_CHARS).collect();
                out.push_str(&format!("### {}\n{text}\n\n", path.display()));
                count += 1;
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(format!(
                "[re-injected recently read files after compaction]\n{out}"
            ))
        }
    }

    /// Restore the most recently committed edit batch for this session.
    pub async fn undo_last(&mut self) -> Result<String, String> {
        let dir = self.store.undo_dir(&self.session.id);
        let summary = EditJournal::undo_latest(&dir)?;
        Ok(format!(
            "Restored {} file(s): {}",
            summary.files,
            summary.restored.join(", ")
        ))
    }
}

fn is_mutation_tool(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file")
}

/// Errors that mean "this call itself was rejected before touching the file":
/// the model mis-specified the anchor ([InvalidInput], e.g. `old_text`
/// matched 0 or several times, a no-change edit), the file is missing
/// ([NotFound]), or the call was hard-rejected before touching anything
/// ([Permission] — the path sandbox, not a user denial, which already carries
/// `denied=true` and is excluded above). They must not roll the whole batch
/// back — a previously succeeded mutation in the same wave is still valid and
/// the model can retry with corrected arguments. State failures
/// ([ConcurrentChange], [Io]) still roll back because the workspace may hold
/// edits based on stale content.
fn is_benign_mutation_error(message: &str) -> bool {
    message.starts_with("[InvalidInput]")
        || message.starts_with("[NotFound]")
        || message.starts_with("[Permission]")
}

fn is_broad_tool(name: &str) -> bool {
    matches!(name, "shell" | "verify" | "grep" | "glob" | "list_dir")
}

fn tool_path(call: &ToolCall) -> Option<PathBuf> {
    call.arguments
        .get("path")
        .and_then(|p| p.as_str())
        .map(PathBuf::from)
}

fn same_tool_path(a: &ToolCall, b: &ToolCall) -> bool {
    matches!((tool_path(a), tool_path(b)), (Some(x), Some(y)) if x == y)
}

/// Dependency edges between tool calls issued in the same turn:
/// - mutations to the same path run in order;
/// - reads of a path wait for mutations to that path;
/// - broad tools (shell/verify/grep/glob/list_dir) wait for all mutations
///   and for earlier broad tools, so they observe a stable workspace.
fn tool_call_dependencies(tool_calls: &[ToolCall]) -> Vec<Vec<usize>> {
    let n = tool_calls.len();
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in 0..i {
            let prev = &tool_calls[j];
            if !is_mutation_tool(&prev.name) {
                if is_broad_tool(&prev.name) && is_broad_tool(&tool_calls[i].name) {
                    deps[i].push(j);
                }
                continue;
            }
            if same_tool_path(&tool_calls[i], prev) || is_broad_tool(&tool_calls[i].name) {
                deps[i].push(j);
            }
        }
    }
    deps
}

struct ToolRunStats {
    mutations: usize,
    verify_passed: bool,
    timed_out: bool,
}

/// Execute the turn's tool calls in dependency waves: independent calls run
/// concurrently; a failed mutation rolls the whole turn's edit batch back.
async fn execute_tool_calls(
    agent: &mut Agent,
    tool_calls: &[ToolCall],
    ctx: &ToolContext,
    journal: &Arc<Mutex<EditJournal>>,
) -> ToolRunStats {
    let n = tool_calls.len();
    let deps = tool_call_dependencies(tool_calls);
    let mut done = vec![false; n];
    let mut remaining = n;
    let mut mutations = 0usize;
    let mut verify_passed = false;
    while remaining > 0 {
        let ready: Vec<usize> = (0..n)
            .filter(|&i| !done[i] && deps[i].iter().all(|&j| done[j]))
            .collect();
        let mut call_seqs: Vec<u64> = Vec::with_capacity(ready.len());
        for &i in &ready {
            let call = &tool_calls[i];
            agent.tool_seq += 1;
            call_seqs.push(agent.tool_seq);
            agent
                .sink
                .event(AgentEvent::ToolStart {
                    name: call.name.clone(),
                    args: call.arguments.clone(),
                    seq: agent.tool_seq,
                })
                .await;
        }
        let futures: Vec<_> = ready
            .iter()
            .map(|&i| {
                let call = &tool_calls[i];
                agent.registry.run(&call.name, call.arguments.clone(), ctx)
            })
            .collect();
        // Interruptible wave: on cancel, the running tool futures are dropped
        // (tools themselves kill their child processes via the shared cancel
        // signal), every pending tool_call_id gets a `[cancelled]` answer so
        // the transcript stays a valid provider message sequence, and the
        // iteration checkpoint in run_turn performs the rollback. A hard
        // wave timeout has the same shape but fires the cancel signal itself
        // (so cooperating tools wind down within a short grace window before
        // the futures are dropped), then fabricates `[timed out]` answers.
        let mut wave_cancelled = false;
        let mut wave_timed_out = false;
        let wave = futures::future::join_all(futures);
        tokio::pin!(wave);
        let results = tokio::select! {
            results = &mut wave => results,
            _ = agent.cancel.cancelled() => {
                wave_cancelled = true;
                Vec::new()
            }
            _ = tokio::time::sleep(agent.tool_wave_timeout) => {
                wave_timed_out = true;
                agent.cancel.cancel();
                let grace = tokio::time::sleep(agent.tool_cancel_grace);
                tokio::pin!(grace);
                let out = tokio::select! {
                    results = &mut wave => results,
                    _ = &mut grace => Vec::new(),
                };
                // The grace window is over. Reset the tool-layer signal we
                // just fired so the *next* wave is not instantly cancelled;
                // the user's own cancel rides the watch channel and is
                // unaffected by this reset.
                agent.cancel.reset();
                out
            }
        };
        if wave_cancelled {
            for (k, &i) in ready.iter().enumerate() {
                let call = &tool_calls[i];
                agent
                    .sink
                    .event(AgentEvent::ToolEnd {
                        name: call.name.clone(),
                        ok: false,
                        summary: "cancelled".to_string(),
                        seq: call_seqs[k],
                    })
                    .await;
                agent.session.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: "[cancelled: interrupted]".to_string(),
                });
                done[i] = true;
            }
            // Calls that never reached a wave still need their tool_call_id
            // answered for the assistant message to remain a valid sequence.
            let cancelled_text = "[cancelled: interrupted]".to_string();
            for i in 0..n {
                if !done[i] {
                    agent.session.push(ChatMessage::Tool {
                        tool_call_id: tool_calls[i].id.clone(),
                        name: tool_calls[i].name.clone(),
                        content: cancelled_text.clone(),
                    });
                    done[i] = true;
                }
            }
            return ToolRunStats {
                mutations,
                verify_passed,
                timed_out: false,
            };
        }

        // Hard wave timeout: the grace window expired and the tools never
        // returned. Fabricate `[timed out]` answers for every call so the
        // transcript stays a valid provider sequence, then end the turn in
        // run_turn (rollback + TurnEnd) like the cancel path does.
        if wave_timed_out && results.is_empty() {
            let note = format!(
                "[timed out: tool exceeded the {}s wave timeout]",
                agent.tool_wave_timeout.as_secs()
            );
            for (k, &i) in ready.iter().enumerate() {
                let call = &tool_calls[i];
                agent
                    .sink
                    .event(AgentEvent::ToolEnd {
                        name: call.name.clone(),
                        ok: false,
                        summary: format!("timed out after {}s", agent.tool_wave_timeout.as_secs()),
                        seq: call_seqs[k],
                    })
                    .await;
                agent.session.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: note.clone(),
                });
                done[i] = true;
            }
            // Calls that never reached a wave still need their tool_call_id
            // answered for the assistant message to remain a valid sequence.
            for i in 0..n {
                if !done[i] {
                    agent.session.push(ChatMessage::Tool {
                        tool_call_id: tool_calls[i].id.clone(),
                        name: tool_calls[i].name.clone(),
                        content: note.clone(),
                    });
                    done[i] = true;
                }
            }
            return ToolRunStats {
                mutations,
                verify_passed,
                timed_out: true,
            };
        }

        // A denied mutation call (user said no) executes nothing and must not
        // roll the batch back. Only real execution failures do — and only
        // *state* failures at that: [InvalidInput] (e.g. an anchor that does
        // not match, a no-change edit) and [NotFound] mean the call itself was
        // rejected before touching the file, so the rest of the wave (and any
        // earlier successful edits this turn) stays valid and the model can
        // retry with corrected arguments. Rolling back on those would silently
        // undo good work on every model mis-specification.
        let any_mutation_failed = ready.iter().enumerate().any(|(k, &i)| {
            is_mutation_tool(&tool_calls[i].name)
                && matches!(
                    &results[k],
                    Err(e) if !e.denied && !is_benign_mutation_error(&e.message)
                )
        });
        // On a real mutation failure the whole wave is rolled back wholesale.
        // The rollback must be surfaced to the model, but NOT as a standalone
        // assistant message: sandwiching an assistant message between the
        // turn's tool_calls message and its tool results yields an invalid
        // provider sequence (assistant -> assistant -> tool), which both
        // OpenAI and Anthropic reject with HTTP 400. Instead the note is
        // prefixed onto the first tool result below, keeping the transcript a
        // valid assistant(tool_calls) -> tool(...) -> tool(...) sequence.
        let mut rollback_note = if any_mutation_failed {
            let summary = match rollback_journal(journal) {
                Some(summary) => summary,
                // Even when there is nothing on disk to restore, the model
                // must learn that the whole wave was discarded — its earlier
                // "successful" edits are no longer in effect.
                None => "no file changes were recorded".to_string(),
            };
            let note = format!("edit batch failed; rolled back: {summary}");
            agent.sink.event(AgentEvent::Info(note.clone())).await;
            Some(note)
        } else {
            None
        };

        for (k, &i) in ready.iter().enumerate() {
            let call = &tool_calls[i];
            let (ok, text) = match &results[k] {
                Ok(output) => (true, output.text.clone()),
                Err(e) => (false, e.message.clone()),
            };
            if ok {
                if is_mutation_tool(&call.name) {
                    mutations += 1;
                }
                if call.name == "verify" {
                    verify_passed = true;
                }
            }
            let mut content = text.clone();
            if let Some(note) = rollback_note.take() {
                content = format!("{note}\n\n{content}");
            }
            let read_path = if ok && call.name == "read_file" && !content.is_empty() {
                resolved_tool_path(ctx, call)
            } else {
                None
            };
            if let Some(path) = read_path {
                let hash = simple_hash(&content);
                if agent.read_hashes.get(&path) == Some(&hash) {
                    content = format!(
                        "[file unchanged (same content as the previous read_file result): {}; \
                         re-read if you need the latest content]",
                        path.display()
                    );
                } else {
                    agent.read_hashes.insert(path.clone(), hash);
                    agent.recent_read_paths.push_back(path);
                    if agent.recent_read_paths.len() > 5 {
                        agent.recent_read_paths.pop_front();
                    }
                }
            }
            let summary = summarize(&content);
            agent
                .sink
                .event(AgentEvent::ToolEnd {
                    name: call.name.clone(),
                    ok,
                    summary: summary.clone(),
                    seq: call_seqs[k],
                })
                .await;
            agent.session.push(ChatMessage::Tool {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: agent.spill_text(&content),
            });
            done[i] = true;
            remaining -= 1;
        }
    }
    ToolRunStats {
        mutations,
        verify_passed,
        timed_out: false,
    }
}

fn lock_journal(journal: &Arc<Mutex<EditJournal>>) -> std::sync::MutexGuard<'_, EditJournal> {
    journal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Find the earliest message index to keep such that eviction never splits an
/// API round: cuts only at User-message round boundaries, keeping the last
/// `ROUNDS_TO_KEEP` rounds verbatim. Returns (cut_index, total_rounds).
fn round_cut_index(messages: &[ChatMessage]) -> Option<(usize, usize)> {
    const ROUNDS_TO_KEEP: usize = 3;
    let round_starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, ChatMessage::User { .. }))
        .map(|(i, _)| i)
        .collect();
    if round_starts.len() <= ROUNDS_TO_KEEP {
        return None;
    }
    let keep_from = round_starts[round_starts.len() - ROUNDS_TO_KEEP];
    if keep_from == 0 {
        None
    } else {
        Some((keep_from, round_starts.len()))
    }
}

/// Index of the `n`-th round start (0-based) among User messages.
fn round_starts_at(messages: &[ChatMessage], n: usize) -> usize {
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, ChatMessage::User { .. }))
        .map(|(i, _)| i)
        .nth(n)
        .unwrap_or(0)
}

fn resolved_tool_path(ctx: &ToolContext, call: &ToolCall) -> Option<PathBuf> {
    let path = tool_path(call)?;
    Some(if path.is_absolute() {
        path
    } else {
        ctx.cwd.join(path)
    })
}

fn simple_hash(text: &str) -> String {
    crate::hash::sha256_hex(text.as_bytes())
}

fn message_text(message: &ChatMessage) -> &str {
    match message {
        ChatMessage::System { content }
        | ChatMessage::User { content }
        | ChatMessage::Assistant { content, .. }
        | ChatMessage::Tool { content, .. } => content,
    }
}

/// Approximate on-wire size of a message: its text plus, for assistant
/// messages, the serialized tool-call arguments — write_file/edit_file carry
/// whole file contents in there, so counting only the prose made the budget
/// see a fraction of the real request and delayed auto-compaction until the
/// provider started rejecting oversized requests.
fn message_size(message: &ChatMessage) -> usize {
    let base = message_text(message).chars().count();
    match message {
        ChatMessage::Assistant { tool_calls, .. } => {
            base + tool_calls
                .iter()
                .map(|call| call.arguments.to_string().chars().count())
                .sum::<usize>()
        }
        _ => base,
    }
}

fn compact_summary(messages: &[ChatMessage], max_chars: usize) -> String {
    let mut parts = Vec::new();
    for message in messages {
        match message {
            ChatMessage::System { .. } => {}
            ChatMessage::User { content } => {
                parts.push(format!("User: {}", truncate_chars(content, 120)));
            }
            ChatMessage::Assistant { content, .. } => {
                parts.push(format!("Assistant: {}", truncate_chars(content, 120)));
            }
            ChatMessage::Tool { name, content, .. } => {
                parts.push(format!("{name}: {}", truncate_chars(content, 80)));
            }
        }
    }
    truncate_chars(&parts.join("\n"), max_chars)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    if chars.len() > max_chars {
        chars.truncate(max_chars);
        chars.push('…');
    }
    chars.into_iter().collect()
}

fn rollback_journal(journal: &Arc<Mutex<EditJournal>>) -> Option<String> {
    match lock_journal(journal).rollback() {
        Ok(files) if files.is_empty() => None,
        Ok(files) => Some(format!(
            "restored {} file(s): {}",
            files.len(),
            files.join(", ")
        )),
        Err(e) => Some(format!("rollback incomplete: {e}")),
    }
}

/// User-facing interrupt note; the rollback clause only appears when there
/// was actually something to roll back.
fn interrupted_note(journal: &Arc<Mutex<EditJournal>>) -> String {
    match rollback_journal(journal) {
        Some(summary) => format!("⏹ Interrupted; rolled back this turn's edits: {summary}"),
        None => "⏹ Interrupted".to_string(),
    }
}

fn summarize(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut chars: Vec<char> = first.chars().collect();
    if chars.len() > 120 {
        chars.truncate(120);
        chars.push('…');
    }
    chars.into_iter().collect()
}

/// Newest file under `cwd` matching `pattern` (glob; `**` crosses directories,
/// `*`/`?` match within a segment, `\\` is normalized to `/`). Hidden dirs,
/// `target/` and `node_modules/` are skipped.
pub fn newest_elf_match(cwd: &Path, pattern: &str) -> Option<PathBuf> {
    let pattern_norm = pattern.replace('\\', "/");
    fn walk(
        dir: &Path,
        cwd: &Path,
        pattern: &str,
        best: &mut Option<(PathBuf, std::time::SystemTime)>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let skip = name.starts_with('.')
                    || ((name == "target" || name == "node_modules")
                        && !allow_heavy_dir(pattern, name));
                if !skip {
                    walk(&path, cwd, pattern, best);
                }
            } else if file_type.is_file()
                && let Ok(rel) = path.strip_prefix(cwd)
            {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if glob_match(pattern, &rel_str) {
                    let mtime = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let newer = best.as_ref().is_none_or(|(_, t)| mtime > *t);
                    if newer {
                        *best = Some((path, mtime));
                    }
                }
            }
        }
    }
    let mut best = None;
    walk(cwd, cwd, &pattern_norm, &mut best);
    best.map(|(path, _)| path)
}

/// Whether a heavy artifact directory (e.g. `target`, `node_modules`) may be
/// entered. It is only allowed when the pattern explicitly names that segment,
/// e.g. `target/**/*.elf`; a broad `**/*.elf` still skips it for performance.
fn allow_heavy_dir(pattern: &str, dir: &str) -> bool {
    pattern.split('/').any(|seg| seg == dir)
}

/// Glob match of a relative path against a pattern. Both use `/` separators.
fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let path_segments: Vec<&str> = path.split('/').collect();
    glob_segments(&pattern_segments, &path_segments)
}

fn glob_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            glob_segments(rest, path) || (!path.is_empty() && glob_segments(pattern, &path[1..]))
        }
        Some((segment, rest)) => path.split_first().is_some_and(|(part, rest_path)| {
            glob_segment(segment, part) && glob_segments(rest, rest_path)
        }),
    }
}

fn glob_segment(pattern: &str, text: &str) -> bool {
    fn seg(pattern: &[char], text: &[char]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some(('*', rest)) => seg(rest, text) || (!text.is_empty() && seg(pattern, &text[1..])),
            Some(('?', rest)) => text
                .split_first()
                .is_some_and(|(_, rest_text)| seg(rest, rest_text)),
            Some((c, rest)) => text
                .split_first()
                .is_some_and(|(tc, rest_text)| c == tc && seg(rest, rest_text)),
        }
    }
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    seg(&pattern_chars, &text_chars)
}

fn thinking_opt(level: ThinkingLevel) -> Option<ThinkingLevel> {
    if level == ThinkingLevel::Off {
        None
    } else {
        Some(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn glob_matches_simple_patterns() {
        assert!(glob_match("build/fw.elf", "build/fw.elf"));
        assert!(glob_match("build/*.elf", "build/fw.elf"));
        assert!(!glob_match("build/*.elf", "build/out/fw.elf"));
        assert!(!glob_match("build/*.elf", "src/fw.elf"));
        assert!(glob_match("build/**/*.elf", "build/out/debug/fw.elf"));
        assert!(glob_match("**/*.elf", "build/fw.elf"));
        assert!(glob_match("build/**.elf", "build/fw.elf"));
        assert!(glob_match("build/fw?.elf", "build/fw2.elf"));
        assert!(!glob_match("build/fw?.elf", "build/fw.elf"));
        assert!(!glob_match("build/fw.elf", "build/fw2.elf"));
    }

    #[test]
    fn newest_match_picks_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("build");
        std::fs::create_dir_all(&sub).unwrap();
        let old = sub.join("fw.elf");
        let new = sub.join("fw2.elf");
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        let old_time = std::fs::metadata(&old).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&new)
            .unwrap()
            .set_modified(old_time + Duration::from_secs(10))
            .unwrap();
        let found = newest_elf_match(dir.path(), "build/*.elf").unwrap();
        assert_eq!(found, new);
    }

    #[test]
    fn newest_match_skips_none_matching() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build").join("fw.bin"), b"x").unwrap();
        assert_eq!(newest_elf_match(dir.path(), "build/*.elf"), None);
    }

    #[test]
    fn newest_match_enters_target_only_when_pattern_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir
            .path()
            .join("target")
            .join("thumbv7em-none-eabi")
            .join("debug");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("fw.elf"), b"x").unwrap();

        assert!(
            allow_heavy_dir("target/**/*.elf", "target"),
            "explicit target segment must be allowed"
        );
        assert!(
            !allow_heavy_dir("**/*.elf", "target"),
            "broad glob must keep skipping target for performance"
        );
        assert!(
            newest_elf_match(dir.path(), "target/**/*.elf").is_some(),
            "pattern naming target/ must find the artifact inside it"
        );
        assert_eq!(
            newest_elf_match(dir.path(), "**/*.elf"),
            None,
            "broad glob must not descend into target/"
        );
    }

    #[test]
    fn glob_crosses_separator_with_double_star() {
        assert!(glob_match("**", "anything/at/all.elf"));
        assert!(glob_match("**", "single"));
        assert!(!glob_match("**/*.elf", "fw.bin"));
    }

    // --- provider stream stall / tool wave timeout -------------------------

    use crate::AutoApprove;
    use crate::provider::{ProviderStream, StopReason};
    use crate::tool::{Tool, ToolError, ToolOutput};

    struct CollectingSink(Arc<Mutex<Vec<AgentEvent>>>);

    #[async_trait]
    impl EventSink for CollectingSink {
        async fn event(&self, event: AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn harness(
        provider: Box<dyn Provider>,
        registry: ToolRegistry,
        stream_timeout: Duration,
        wave_timeout: Duration,
    ) -> (Arc<Mutex<Vec<AgentEvent>>>, tokio::task::JoinHandle<()>) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let session = Session::new(dir.path().to_path_buf(), "mock", "mock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::new(
            Some(provider),
            Arc::new(registry),
            session,
            store,
            Arc::new(AutoApprove::everything()),
            Arc::new(CollectingSink(events.clone())),
            4,
        );
        agent.set_stream_timeout(stream_timeout);
        agent.set_tool_wave_timeout(wave_timeout);
        agent.set_tool_cancel_grace(Duration::from_millis(50));
        let handle = tokio::spawn(async move {
            let _ = agent.run_turn("go").await;
        });
        (events, handle)
    }

    /// Provider that yields one text chunk then goes silent forever, as if the
    /// connection died mid-stream.
    struct MidStreamStallProvider;

    #[async_trait]
    impl Provider for MidStreamStallProvider {
        async fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
            let stream = futures::stream::iter(vec![Ok(ProviderEvent::Text("hello".to_string()))])
                .chain(futures::stream::pending());
            Ok(Box::pin(stream))
        }

        fn model(&self) -> &str {
            "stall"
        }
    }

    #[tokio::test]
    async fn mid_stream_stall_ends_the_turn() {
        let (events, task) = harness(
            Box::new(MidStreamStallProvider),
            ToolRegistry::new(),
            Duration::from_millis(300),
            Duration::from_secs(600),
        );
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("turn must end promptly after a mid-stream stall")
            .unwrap();
        let events = events.lock().unwrap();
        let infos: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Info(m) => Some(m.clone()),
                _ => None,
            })
            .collect();
        assert!(
            infos.iter().any(|m| m.contains("stalled")),
            "expected a stall Info event, got: {infos:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::TurnEnd { .. })),
            "expected TurnEnd after the stall, got: {events:?}"
        );
    }

    /// Tool that never returns, like a wedged `task` subagent or an unanswered
    /// `ask_user`.
    struct HangingTool;

    #[async_trait]
    impl Tool for HangingTool {
        fn name(&self) -> &'static str {
            "hang"
        }
        fn description(&self) -> &'static str {
            "hangs forever"
        }
        fn input_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
            std::future::pending().await
        }
    }

    /// Provider that asks for one tool call then ends the round, so the tool
    /// wave runs the hanging tool.
    struct ToolWaveProvider;

    #[async_trait]
    impl Provider for ToolWaveProvider {
        async fn stream(&self, _request: ChatRequest) -> Result<ProviderStream, ProviderError> {
            let stream = futures::stream::iter(vec![
                Ok(ProviderEvent::ToolCall(ToolCall {
                    id: "t1".to_string(),
                    name: "hang".to_string(),
                    arguments: json!({}),
                })),
                Ok(ProviderEvent::Stop(StopReason::EndTurn)),
            ]);
            Ok(Box::pin(stream))
        }

        fn model(&self) -> &str {
            "toolwave"
        }
    }

    #[tokio::test]
    async fn hanging_tool_wave_is_bounded_by_the_timeout() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(HangingTool));
        let (events, task) = harness(
            Box::new(ToolWaveProvider),
            registry,
            Duration::from_secs(600),
            Duration::from_millis(300),
        );
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("turn must end promptly after the tool wave timeout")
            .unwrap();
        let events = events.lock().unwrap();
        let timed_out_ends: Vec<&AgentEvent> = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolEnd { ok: false, .. }))
            .collect();
        assert!(
            !timed_out_ends.is_empty(),
            "expected a failed ToolEnd for the hung tool, got: {events:?}"
        );
        let infos: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Info(m) => Some(m.clone()),
                _ => None,
            })
            .collect();
        assert!(
            infos.iter().any(|m| m.contains("Tool wave timed out")),
            "expected a wave-timeout Info event, got: {infos:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::TurnEnd { .. })),
            "expected TurnEnd after the wave timeout, got: {events:?}"
        );
    }

    /// Regression: a failed mutation in a wave must NOT push a standalone
    /// assistant message — that would yield `assistant -> assistant -> tool`
    /// and make the next provider request a 400. The rollback note must be
    /// surfaced inside a tool result instead.
    #[tokio::test]
    async fn mutation_failure_keeps_provider_sequence_valid() {
        struct OkWrite;
        #[async_trait]
        impl Tool for OkWrite {
            fn name(&self) -> &'static str {
                "write_file"
            }
            fn description(&self) -> &'static str {
                "ok write"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput {
                    text: "ok".to_string(),
                })
            }
        }
        struct FailEdit;
        #[async_trait]
        impl Tool for FailEdit {
            fn name(&self) -> &'static str {
                "edit_file"
            }
            fn description(&self) -> &'static str {
                "failing edit"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                Err(ToolError::new("[ConcurrentChange] changed during edit"))
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(OkWrite));
        registry.register(Arc::new(FailEdit));

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let session = Session::new(dir.path().to_path_buf(), "mock", "mock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::new(
            None,
            Arc::new(registry),
            session,
            store,
            Arc::new(AutoApprove::everything()),
            Arc::new(CollectingSink(events.clone())),
            4,
        );
        let journal = Arc::new(Mutex::new(EditJournal::new(dir.path().join("undo"))));
        let ctx = ToolContext::with_cwd(dir.path().to_path_buf());

        let calls = vec![
            ToolCall {
                id: "a".to_string(),
                name: "write_file".to_string(),
                arguments: json!({"path": "x.txt"}),
            },
            ToolCall {
                id: "b".to_string(),
                name: "edit_file".to_string(),
                arguments: json!({"path": "y.txt"}),
            },
        ];
        // Simulate what run_turn does before executing the wave.
        agent.session.push(ChatMessage::Assistant {
            content: "let me edit".to_string(),
            tool_calls: calls.clone(),
        });
        let stats = execute_tool_calls(&mut agent, &calls, &ctx, &journal).await;
        assert_eq!(
            stats.mutations, 1,
            "one mutation succeeded before the rollback"
        );

        // The transcript must remain a valid provider sequence: no assistant
        // message may be immediately followed by another assistant message.
        for w in agent.session.messages.windows(2) {
            assert!(
                !(matches!(w[0], ChatMessage::Assistant { .. })
                    && matches!(w[1], ChatMessage::Assistant { .. })),
                "consecutive assistant messages break the provider sequence: {:?}",
                agent.session.messages
            );
        }
        // The rollback note must be surfaced inside a tool result, not lost.
        let tool_contents: Vec<String> = agent
            .session
            .messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert!(
            tool_contents.iter().any(|c| c.contains("rolled back")),
            "rollback note should be present in a tool result, got: {tool_contents:?}"
        );
    }

    /// A mutation that fails with [InvalidInput] (anchor mismatch, no-change
    /// edit) did not touch the file: the rest of the wave must NOT be rolled
    /// back, and the successful mutation must still be counted/committed.
    #[tokio::test]
    async fn invalid_input_mutation_failure_does_not_roll_back_the_batch() {
        struct OkWrite;
        #[async_trait]
        impl Tool for OkWrite {
            fn name(&self) -> &'static str {
                "write_file"
            }
            fn description(&self) -> &'static str {
                "ok write"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput {
                    text: "ok".to_string(),
                })
            }
        }
        struct BadAnchorEdit;
        #[async_trait]
        impl Tool for BadAnchorEdit {
            fn name(&self) -> &'static str {
                "edit_file"
            }
            fn description(&self) -> &'static str {
                "failing edit"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                Err(ToolError::new(
                    "[InvalidInput] old_text matched 0 times in main.c; expected exactly 1",
                ))
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(OkWrite));
        registry.register(Arc::new(BadAnchorEdit));

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let session = Session::new(dir.path().to_path_buf(), "mock", "mock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::new(
            None,
            Arc::new(registry),
            session,
            store,
            Arc::new(AutoApprove::everything()),
            Arc::new(CollectingSink(events.clone())),
            4,
        );
        let journal = Arc::new(Mutex::new(EditJournal::new(dir.path().join("undo"))));
        let ctx = ToolContext::with_cwd(dir.path().to_path_buf());

        let calls = vec![
            ToolCall {
                id: "a".to_string(),
                name: "write_file".to_string(),
                arguments: json!({"path": "x.txt"}),
            },
            ToolCall {
                id: "b".to_string(),
                name: "edit_file".to_string(),
                arguments: json!({"path": "y.txt"}),
            },
        ];
        agent.session.push(ChatMessage::Assistant {
            content: "let me edit".to_string(),
            tool_calls: calls.clone(),
        });
        let stats = execute_tool_calls(&mut agent, &calls, &ctx, &journal).await;
        // The successful write is still counted; nothing was rolled back.
        assert_eq!(stats.mutations, 1);
        let tool_contents: Vec<String> = agent
            .session
            .messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !tool_contents.iter().any(|c| c.contains("rolled back")),
            "an [InvalidInput] failure must not roll back the batch, got: {tool_contents:?}"
        );
        // The failing edit's own error is still surfaced.
        assert!(
            tool_contents
                .iter()
                .any(|c| c.contains("old_text matched 0 times")),
            "the [InvalidInput] error must be visible, got: {tool_contents:?}"
        );
    }

    /// A mutation hard-rejected by the path sandbox ([Permission], with
    /// denied=false — this is not a user denial) must not roll the batch back
    /// either: the call never touched the file. This is the wave that kept
    /// reverting main.c in real sessions: the model edited a workspace file
    /// and tried to write a build script outside the workspace in the same
    /// wave, and the sandbox rejection rolled back the good edit.
    #[tokio::test]
    async fn permission_mutation_failure_does_not_roll_back_the_batch() {
        struct OkEdit;
        #[async_trait]
        impl Tool for OkEdit {
            fn name(&self) -> &'static str {
                "edit_file"
            }
            fn description(&self) -> &'static str {
                "ok edit"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput {
                    text: "Edited main.c".to_string(),
                })
            }
        }
        struct SandboxBlockedWrite;
        #[async_trait]
        impl Tool for SandboxBlockedWrite {
            fn name(&self) -> &'static str {
                "write_file"
            }
            fn description(&self) -> &'static str {
                "blocked write"
            }
            fn input_schema(&self) -> Value {
                json!({"type": "object", "properties": {}})
            }
            async fn run(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
                // Path-sandbox rejection: [Permission] tag but denied=false.
                Err(ToolError::new(
                    "[Permission] path is outside the workspace: C:\\build_fw.ps1",
                ))
            }
        }

        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(OkEdit));
        registry.register(Arc::new(SandboxBlockedWrite));

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf());
        let session = Session::new(dir.path().to_path_buf(), "mock", "mock");
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut agent = Agent::new(
            None,
            Arc::new(registry),
            session,
            store,
            Arc::new(AutoApprove::everything()),
            Arc::new(CollectingSink(events.clone())),
            4,
        );
        let journal = Arc::new(Mutex::new(EditJournal::new(dir.path().join("undo"))));
        let ctx = ToolContext::with_cwd(dir.path().to_path_buf());

        let calls = vec![
            ToolCall {
                id: "a".to_string(),
                name: "edit_file".to_string(),
                arguments: json!({"path": "main.c"}),
            },
            ToolCall {
                id: "b".to_string(),
                name: "write_file".to_string(),
                arguments: json!({"path": "C:\\build_fw.ps1"}),
            },
        ];
        agent.session.push(ChatMessage::Assistant {
            content: "edit and write".to_string(),
            tool_calls: calls.clone(),
        });
        let stats = execute_tool_calls(&mut agent, &calls, &ctx, &journal).await;
        assert_eq!(stats.mutations, 1, "the good edit must still count");
        let tool_contents: Vec<String> = agent
            .session
            .messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::Tool { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        assert!(
            !tool_contents.iter().any(|c| c.contains("rolled back")),
            "a sandbox [Permission] rejection must not roll back the batch, got: {tool_contents:?}"
        );
        // The blocked write's own error is still surfaced.
        assert!(
            tool_contents
                .iter()
                .any(|c| c.contains("outside the workspace")),
            "the [Permission] error must be visible, got: {tool_contents:?}"
        );
    }
}
