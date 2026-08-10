use crate::config::CompactionStrategy;
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
use tokio::sync::watch;

#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStart,
    TextDelta(String),
    ToolStart {
        name: String,
        args: Value,
    },
    ToolEnd {
        name: String,
        ok: bool,
        summary: String,
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

pub struct Agent {
    provider: Option<Box<dyn Provider>>,
    registry: Arc<ToolRegistry>,
    session: Session,
    store: SessionStore,
    permission: Arc<dyn PermissionChecker>,
    sink: Arc<dyn EventSink>,
    cancel_tx: watch::Sender<bool>,
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
    web_search_api_key: Option<String>,
    /// Per-session bookkeeping directory (todo list etc.).
    session_dir: Option<PathBuf>,
    /// Glob pattern for the firmware ELF artifact; enables the automatic
    /// binary-analysis gate (`elf_analyze`) before a finished turn is accepted.
    elf_glob: Option<String>,
    /// True once the auto elf gate has run for the current edit batch; reset
    /// whenever a new mutation lands. Independent of `mutations_since_verify`
    /// so the gate still runs after the model verifies via the verify tool.
    elf_gate_done: bool,
    /// True when a mutation landed since the last elf analysis; gates the
    /// binary-analysis diff without depending on verify-bookkeeping.
    elf_gate_dirty: bool,
    /// Hash of the last read result per path, for unchanged-read dedup.
    read_hashes: HashMap<PathBuf, String>,
    /// Recently read paths (most recent last), for post-compact re-injection.
    recent_read_paths: VecDeque<PathBuf>,
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
        Self {
            provider,
            registry,
            session,
            store,
            permission,
            sink,
            cancel_tx: watch::channel(false).0,
            max_iterations,
            allow_dangerous: false,
            verify_command: None,
            context_budget_chars: 60_000,
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
            web_search_api_key: None,
            session_dir: None,
            elf_glob: None,
            elf_gate_done: false,
            elf_gate_dirty: false,
            read_hashes: HashMap::new(),
            recent_read_paths: VecDeque::new(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
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
    /// the next safe checkpoint (provider stream boundary / iteration start).
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    /// Clear a pending cancellation request before starting a new turn.
    pub fn reset_cancel(&self) {
        let _ = self.cancel_tx.send(false);
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

    /// Set the `[tools] elf` glob pattern; when set, the harness seeds an ELF
    /// baseline and auto-runs `elf_analyze` before a finished turn is accepted.
    pub fn set_elf_glob(&mut self, glob: Option<String>) {
        self.elf_glob = glob;
    }

    /// At turn start, refresh the ELF baseline so edits are diffed against the
    /// state the turn began with. Silent except on tool errors.
    async fn seed_elf_baseline(&mut self, ctx: &ToolContext) {
        let Some(glob) = self.elf_glob.clone() else {
            return;
        };
        let Some(tool) = self.registry.get("elf_analyze") else {
            return;
        };
        let Some(elf) = newest_elf_match(&self.session.cwd, &glob) else {
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
    /// newest ELF matching `elf_glob`. Returns the report (with diff vs the
    /// recorded baseline) when the model should be asked to review it, and
    /// `None` when there is nothing new to surface. Never blocks completion.
    async fn run_elf_gate(&mut self, ctx: &ToolContext) -> Option<String> {
        let glob = self.elf_glob.clone()?;
        let tool = self.registry.get("elf_analyze")?;
        if self.elf_gate_done {
            return None;
        }
        let elf = newest_elf_match(&self.session.cwd, &glob)?;
        self.elf_gate_done = true;
        let args = json!({ "file": elf.to_string_lossy() });
        match tool.run(args, ctx).await {
            Ok(out) => Some(out.text),
            Err(e) => Some(e.message),
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
        if fs::create_dir_all(&dir).is_ok() {
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
    pub fn replace_session(&mut self, session: Session) {
        self.session = session;
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
        let mut cancel_rx = self.cancel_tx.subscribe();
        if *cancel_rx.borrow() {
            self.sink
                .event(AgentEvent::Info(
                    "⏹ Interrupted (no work started yet)".to_string(),
                ))
                .await;
            self.sink
                .event(AgentEvent::TurnEnd {
                    text: String::new(),
                })
                .await;
            let _ = self.store.save(&self.session);
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
        };

        self.seed_elf_baseline(&ctx).await;

        for _ in 0..self.max_iterations {
            self.compact_if_needed().await;
            if *cancel_rx.borrow() {
                let summary = rollback_journal(&journal);
                self.sink
                    .event(AgentEvent::Info(format!(
                        "⏹ Interrupted; rolled back this turn's edits: {summary}"
                    )))
                    .await;
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: String::new(),
                    })
                    .await;
                let _ = self.store.save(&self.session);
                return Ok(String::new());
            }
            let request = self.build_request();
            let provider = self.provider.as_ref().ok_or(AgentError::NoProvider)?;
            let mut stream = tokio::select! {
                result = provider.stream(request) => result?,
                _ = cancel_rx.changed() => {
                    let summary = rollback_journal(&journal);
                    self.sink
                        .event(AgentEvent::Info(format!(
                            "⏹ Interrupted; rolled back this turn's edits: {summary}"
                        )))
                        .await;
                    self.sink
                        .event(AgentEvent::TurnEnd {
                            text: String::new(),
                        })
                        .await;
                    let _ = self.store.save(&self.session);
                    return Ok(String::new());
                }
            };
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut cancelled = false;

            while let Some(event) = tokio::select! {
                next = stream.next() => next,
                _ = cancel_rx.changed() => {
                    cancelled = true;
                    None
                }
            } {
                let event = match event {
                    Ok(event) => event,
                    Err(e) => {
                        let summary = rollback_journal(&journal);
                        self.sink
                            .event(AgentEvent::Error(format!(
                                "provider error; rolled back this turn's edits: {summary}"
                            )))
                            .await;
                        return Err(AgentError::Provider(e));
                    }
                };
                match event {
                    ProviderEvent::Text(text) => {
                        content.push_str(&text);
                        self.sink.event(AgentEvent::TextDelta(text.clone())).await;
                    }
                    ProviderEvent::ToolCall(call) => tool_calls.push(call),
                    ProviderEvent::Stop { .. } => {}
                }
            }

            // Never persist tool calls that were never executed: an assistant
            // message with dangling tool_calls would make the next request an
            // invalid provider sequence. On cancel, keep only the text.
            let saved_calls = if cancelled {
                Vec::new()
            } else {
                tool_calls.clone()
            };
            self.session.push(ChatMessage::Assistant {
                content: content.clone(),
                tool_calls: saved_calls,
            });

            if cancelled {
                let summary = rollback_journal(&journal);
                self.sink
                    .event(AgentEvent::Info(format!(
                        "⏹ Interrupted; rolled back this turn's edits: {summary}"
                    )))
                    .await;
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                let _ = self.store.save(&self.session);
                return Ok(content);
            }

            if tool_calls.is_empty() {
                let plain_assistant = self.session.messages.pop().expect("assistant message");
                if mutations_since_verify > 0
                    && self.verify_command.is_some()
                    && self.registry.get("verify").is_some()
                {
                    let gate_call = ToolCall {
                        id: "verify_gate".to_string(),
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
                    && let Some(text) = self.run_elf_gate(&ctx).await
                {
                    // Fresh binary diff: surface it to the model so it can
                    // decide whether to accept the state or keep fixing. The
                    // report is injected as a real tool round (assistant
                    // tool_use -> tool result) so the transcript stays a valid
                    // provider message sequence.
                    self.elf_gate_dirty = false;
                    self.session.messages.pop();
                    let gate_call = ToolCall {
                        id: "elf_gate".to_string(),
                        name: "elf_analyze".to_string(),
                        arguments: json!({}),
                    };
                    self.session.push(ChatMessage::Assistant {
                        content: content.clone(),
                        tool_calls: vec![gate_call.clone()],
                    });
                    self.session.push(ChatMessage::Tool {
                        tool_call_id: gate_call.id,
                        name: "elf_analyze".to_string(),
                        content: text,
                    });
                    self.sink
                        .event(AgentEvent::Info(
                            "binary analysis: the firmware changed vs its baseline — review \
                             the diff above and decide whether to accept or keep fixing"
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
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                self.store.save(&self.session)?;
                return Ok(content);
            }

            let stats = execute_tool_calls(self, &tool_calls, &ctx, &journal).await;
            mutations_since_verify += stats.mutations;
            if stats.mutations > 0 {
                self.elf_gate_done = false;
                self.elf_gate_dirty = true;
            }
            if stats.verify_passed {
                mutations_since_verify = 0;
            }
        }

        let summary = rollback_journal(&journal);
        self.sink
            .event(AgentEvent::Info(format!(
                "reached max iterations; rolled back this turn's edits: {summary}"
            )))
            .await;
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
        let total: usize = self
            .session
            .messages
            .iter()
            .map(|m| message_text(m).chars().count())
            .sum();
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
        let mut messages = vec![ChatMessage::User { content }];
        messages.extend(self.session.messages.iter().cloned());
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
        while let Some(event) = stream.next().await {
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
        for &i in &ready {
            let call = &tool_calls[i];
            agent
                .sink
                .event(AgentEvent::ToolStart {
                    name: call.name.clone(),
                    args: call.arguments.clone(),
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
        let results = futures::future::join_all(futures).await;

        let any_mutation_failed = ready
            .iter()
            .enumerate()
            .any(|(k, &i)| is_mutation_tool(&tool_calls[i].name) && results[k].is_err());
        if any_mutation_failed {
            let summary = rollback_journal(journal);
            agent
                .sink
                .event(AgentEvent::Info(format!(
                    "edit batch failed; rolled back: {summary}"
                )))
                .await;
        }

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

fn rollback_journal(journal: &Arc<Mutex<EditJournal>>) -> String {
    match lock_journal(journal).rollback() {
        Ok(files) if files.is_empty() => "no file changes were recorded".to_string(),
        Ok(files) => format!("restored {} file(s): {}", files.len(), files.join(", ")),
        Err(e) => format!("rollback incomplete: {e}"),
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
fn newest_elf_match(cwd: &Path, pattern: &str) -> Option<PathBuf> {
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
}
