use async_trait::async_trait;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use firment_core::{
    Agent, AgentEvent, Asker, ChatMessage, Config, EventSink, PermissionChecker, PermissionError,
    PlanModePermission, ProviderConfig, QuestionRequest, Session, SessionMode, SessionStore,
    SubagentRunner, ThinkingLevel,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::{HashSet, VecDeque};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub async fn run(
    config: Config,
    config_path: std::path::PathBuf,
    session: Session,
) -> anyhow::Result<()> {
    let config = config.merged_for(&session.cwd);
    // The TUI must start even without an API key, so the user can configure
    // it from inside with /apikey or /provider.
    let (provider, startup_hint) =
        match config.build_provider(Some(&session.provider), Some(&session.model)) {
            Ok(provider) => (Some(provider), None),
            Err(e) => (
                None,
                Some(format!(
                    "⚠ {e} (run /apikey sk-xxx inside the TUI to configure it without exiting)"
                )),
            ),
        };
    let store = SessionStore::default();
    let default_registry = firment_tools::default_registry();
    let plan_registry = firment_tools::plan_registry();

    let (event_tx, event_rx) = mpsc::channel(256);
    let (perm_tx, perm_rx) = mpsc::channel(16);
    let (ask_tx, ask_rx) = mpsc::channel(16);
    let (cmd_tx, mut cmd_rx) = mpsc::channel(32);
    let always: Arc<Mutex<HashSet<String>>> =
        Arc::new(Mutex::new(config.auto_approve.iter().cloned().collect()));

    let sink = Arc::new(ChannelSink { tx: event_tx });
    let tui_permission: Arc<dyn PermissionChecker> = Arc::new(TuiPermission {
        req_tx: perm_tx,
        always: always.clone(),
    });
    let plan_permission: Arc<dyn PermissionChecker> =
        Arc::new(PlanModePermission::new(tui_permission.clone()));

    let session_mode = session.mode;
    let initial_registry = if session_mode == SessionMode::Plan {
        plan_registry.clone()
    } else {
        default_registry.clone()
    };
    let initial_permission = if session_mode == SessionMode::Plan {
        plan_permission.clone()
    } else {
        tui_permission.clone()
    };
    let mut agent = Agent::new(
        provider,
        initial_registry,
        session,
        store.clone(),
        initial_permission,
        sink.clone(),
        config.max_iterations,
    );
    // Interactive TUI: the permission popup is the decision point, so
    // dangerous shell commands are allowed to reach it (and are labeled ⚠).
    agent.set_allow_dangerous(true);
    agent.set_context_budget_chars(config.context_budget_chars);
    agent.set_compaction_strategy(config.compaction_strategy);
    agent.set_symbols_backend(config.tools.symbols_backend.clone());
    agent.set_build_command(config.tools.build_command.clone());
    agent.set_default_chip(config.tools.default_chip.clone());
    agent.set_monitor_port(config.tools.monitor_port.clone());
    agent.set_monitor_baud(config.tools.monitor_baud);
    agent.set_elf_glob(config.tools.elf.clone());
    let asker: Arc<dyn Asker> = Arc::new(TuiAsker { req_tx: ask_tx });
    agent.set_asker(Some(asker.clone()));
    agent.set_web_search(
        config.tools.web_search.clone(),
        config.tools.resolved_web_search_api_key(),
    );
    agent.set_session_dir(Some(store.dir.join("work")));
    let subagent_factory: Arc<SubagentRunner> = Arc::new(SubagentRunner::new(
        Arc::new(config.clone()),
        plan_registry.clone(),
        agent.session().provider.clone(),
        agent.session().model.clone(),
        Some(asker.clone()),
    ));
    agent.set_subagent_factory(Some(subagent_factory));
    let initial_messages = agent.session().messages.clone();
    let model = agent.session().model.clone();
    let cwd = agent.session().cwd.clone();
    let provider_name = agent.session().provider.clone();
    let thinking = agent.session().thinking;
    let mut task_config = config.clone();
    let task_config_path = config_path.clone();
    let agent_task = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                AgentCmd::User(text) => {
                    agent.reset_cancel();
                    if let Err(e) = agent.run_turn(&text).await {
                        agent.emit(AgentEvent::Error(e.to_string())).await;
                    }
                }
                AgentCmd::Cancel => {
                    agent.cancel();
                }
                AgentCmd::SetModel(model) => {
                    agent.set_model(model.clone());
                    if let Some(provider) = task_config.providers.get_mut(&agent.session().provider)
                    {
                        provider.model = model.clone();
                    }
                    let _ = task_config.save(&task_config_path);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!("model -> {model} (saved)")))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: Some(model),
                            thinking: None,
                            mode: None,
                        })
                        .await;
                }
                AgentCmd::SetThinking(level) => {
                    agent.set_thinking(level);
                    task_config.thinking = level;
                    let _ = task_config.save(&task_config_path);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!(
                            "thinking -> {} (saved)",
                            level.label()
                        )))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: Some(level),
                            mode: None,
                        })
                        .await;
                }
                AgentCmd::SetMode(mode) => {
                    let registry = if mode == SessionMode::Plan {
                        plan_registry.clone()
                    } else {
                        default_registry.clone()
                    };
                    let permission: Arc<dyn PermissionChecker> = if mode == SessionMode::Plan {
                        plan_permission.clone()
                    } else {
                        tui_permission.clone()
                    };
                    agent.set_mode(mode, registry, permission);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!(
                            "mode -> {} (takes effect from the next message)",
                            mode.label()
                        )))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: None,
                            mode: Some(mode),
                        })
                        .await;
                }
                AgentCmd::OpenModelPicker => {
                    let provider_name = agent.session().provider.clone();
                    match task_config.list_models(&provider_name).await {
                        Ok(models) => agent.emit(AgentEvent::Models(models)).await,
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!(
                                    "failed to fetch the model list: {e}"
                                )))
                                .await;
                            agent.emit(AgentEvent::Models(Vec::new())).await;
                        }
                    }
                }
                AgentCmd::OpenSessionPicker => match store.list() {
                    Ok(mut sessions) => {
                        for summary in &mut sessions {
                            if summary.preview.is_empty() {
                                summary.preview = store
                                    .load(&summary.id)
                                    .map(|s| s.title())
                                    .unwrap_or_default();
                            }
                        }
                        agent.emit(AgentEvent::Sessions(sessions)).await;
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("failed to list sessions: {e}")))
                            .await;
                        agent.emit(AgentEvent::Sessions(Vec::new())).await;
                    }
                },
                AgentCmd::NewSession => {
                    let fresh = Session::new(
                        agent.session().cwd.clone(),
                        agent.session().provider.clone(),
                        agent.session().model.clone(),
                    );
                    let registry = default_registry.clone();
                    let permission: Arc<dyn PermissionChecker> = tui_permission.clone();
                    agent.replace_session(fresh.clone());
                    agent.set_mode(SessionMode::Agent, registry, permission);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(
                            "Started a new conversation (current provider/model kept)".to_string(),
                        ))
                        .await;
                    agent.emit(AgentEvent::SessionLoaded(fresh)).await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: None,
                            mode: Some(SessionMode::Agent),
                        })
                        .await;
                }
                AgentCmd::LoadSession(id) => match store.load(&id) {
                    Ok(loaded) => {
                        let mode = loaded.mode;
                        agent.replace_session(loaded.clone());
                        match task_config
                            .build_provider(Some(&loaded.provider), Some(&loaded.model))
                        {
                            Ok(provider) => agent.set_provider(provider),
                            Err(e) => {
                                agent
                                    .emit(AgentEvent::Error(format!(
                                        "session switched, but rebuilding the provider failed: {e}"
                                    )))
                                    .await;
                            }
                        }
                        let registry = if mode == SessionMode::Plan {
                            plan_registry.clone()
                        } else {
                            default_registry.clone()
                        };
                        let permission: Arc<dyn PermissionChecker> = if mode == SessionMode::Plan {
                            plan_permission.clone()
                        } else {
                            tui_permission.clone()
                        };
                        agent.set_mode(mode, registry, permission);
                        let _ = agent.save_session();
                        agent
                            .emit(AgentEvent::Info(format!(
                                "Switched to session {} ({} · {} · {})",
                                loaded.id,
                                loaded.provider,
                                loaded.model,
                                mode.label()
                            )))
                            .await;
                        agent.emit(AgentEvent::SessionLoaded(loaded.clone())).await;
                        agent
                            .emit(AgentEvent::Settings {
                                provider: Some(loaded.provider.clone()),
                                model: Some(loaded.model.clone()),
                                thinking: Some(loaded.thinking),
                                mode: Some(mode),
                            })
                            .await;
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("failed to load session: {e}")))
                            .await;
                    }
                },
                AgentCmd::Undo => match agent.undo_last().await {
                    Ok(summary) => {
                        agent.emit(AgentEvent::Info(summary)).await;
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("undo failed: {e}")))
                            .await;
                    }
                },
                AgentCmd::Ledger => {
                    let summary = agent.ledger_summary();
                    if summary.is_empty() {
                        agent
                            .emit(AgentEvent::Info(
                                "No committed edits in this session yet".to_string(),
                            ))
                            .await;
                    } else {
                        agent
                            .emit(AgentEvent::Info(format!(
                                "Recent change ledger:\n{summary}"
                            )))
                            .await;
                    }
                }
                AgentCmd::Pin { path } => match agent.pin_path(std::path::PathBuf::from(&path)) {
                    Ok(message) => {
                        agent.emit(AgentEvent::Info(message)).await;
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("pin failed: {e}")))
                            .await;
                    }
                },
                AgentCmd::Unpin { path } => {
                    match agent.unpin_path(std::path::PathBuf::from(&path)) {
                        Ok(message) => {
                            agent.emit(AgentEvent::Info(message)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("unpin failed: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::SetProvider(name) => {
                    match task_config.build_provider(Some(&name), None) {
                        Ok(new_provider) => {
                            let configured_model = new_provider.model().to_string();
                            agent.set_provider_name(&name);
                            agent.set_provider(new_provider);
                            agent.set_model(configured_model.clone());
                            task_config.default_provider = name.clone();
                            let _ = task_config.save(&task_config_path);
                            let _ = agent.save_session();
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "provider -> {name} · model -> {configured_model} (saved)"
                                )))
                                .await;
                            agent
                                .emit(AgentEvent::Settings {
                                    provider: Some(name),
                                    model: Some(configured_model),
                                    thinking: None,
                                    mode: None,
                                })
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to switch provider: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::SetApiKey { provider, key } => {
                    let provider_name =
                        provider.unwrap_or_else(|| agent.session().provider.clone());
                    match task_config.set_api_key(&provider_name, &key) {
                        Ok(()) => {
                            let model = agent.session().model.clone();
                            match task_config.build_provider(Some(&provider_name), Some(&model)) {
                                Ok(new_provider) => {
                                    agent.set_provider(new_provider);
                                    agent
                                        .emit(AgentEvent::Info(format!(
                                            "API key for {provider_name} saved to {} (no \
                                             further setup needed)",
                                            firment_core::auth_path().display()
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    agent
                                        .emit(AgentEvent::Error(format!(
                                            "rebuilding the provider after saving failed: {e}"
                                        )))
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to save API key: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::ListModels => {
                    let provider_name = agent.session().provider.clone();
                    match task_config.list_models(&provider_name).await {
                        Ok(models) => {
                            let providers: Vec<String> =
                                task_config.providers.keys().cloned().collect();
                            let mut msg = format!(
                                "Configured providers: {} (current: {})\nAvailable models:",
                                providers.join(", "),
                                provider_name
                            );
                            if models.is_empty() {
                                msg.push_str("\n  (the API returned no models; set one manually with /model <id>)");
                            } else {
                                for model in models {
                                    msg.push_str(&format!("\n  {model}"));
                                }
                            }
                            msg.push_str("\nSwitch: /model <id> or /provider <name>");
                            agent.emit(AgentEvent::Info(msg)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!(
                                    "failed to fetch the model list: {e}"
                                )))
                                .await;
                        }
                    }
                }
                AgentCmd::AddProvider {
                    name,
                    r#type,
                    base_url,
                    model,
                } => {
                    let entry = task_config
                        .providers
                        .entry(name.clone())
                        .or_insert_with(|| ProviderConfig {
                            r#type: r#type.clone(),
                            base_url: Some(base_url.clone()),
                            api_key_env: None,
                            api_key: None,
                            model: model.clone(),
                            max_tokens: None,
                            temperature: None,
                        });
                    entry.r#type = r#type.clone();
                    entry.base_url = Some(base_url.clone());
                    entry.model = model.clone();
                    match task_config.save(&task_config_path) {
                        Ok(()) => {
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "provider {name} saved; next run /apikey {name} sk-xxx to set \
                                     the key"
                                )))
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to save provider: {e}")))
                                .await;
                        }
                    }
                }
            }
        }
    });

    let (ui_tx, ui_rx) = mpsc::channel(128);
    thread::spawn(move || {
        while let Ok(event) = read() {
            if ui_tx.blocking_send(event).is_err() {
                break;
            }
        }
    });

    let mut terminal = init_terminal()?;
    let mut app = App::new(
        cmd_tx,
        always,
        model,
        cwd,
        provider_name,
        thinking,
        session_mode,
        config_path,
        startup_hint,
        initial_messages,
    );
    let result = run_loop(&mut terminal, &mut app, event_rx, perm_rx, ask_rx, ui_rx).await;
    restore_terminal(&mut terminal)?;
    agent_task.abort();
    result
}

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Maximum input box height (borders included): 2 border rows + up to 5 text rows.
const MAX_INPUT_HEIGHT: usize = 7;

fn init_terminal() -> anyhow::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Tui) -> anyhow::Result<()> {
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    disable_raw_mode()?;
    Ok(())
}

enum AgentCmd {
    User(String),
    Cancel,
    SetModel(String),
    SetThinking(ThinkingLevel),
    SetMode(SessionMode),
    OpenModelPicker,
    OpenSessionPicker,
    NewSession,
    LoadSession(String),
    Undo,
    Ledger,
    Pin {
        path: String,
    },
    Unpin {
        path: String,
    },
    SetProvider(String),
    SetApiKey {
        provider: Option<String>,
        key: String,
    },
    ListModels,
    AddProvider {
        name: String,
        r#type: String,
        base_url: String,
        model: String,
    },
}

struct ChannelSink {
    tx: mpsc::Sender<AgentEvent>,
}

#[async_trait]
impl EventSink for ChannelSink {
    async fn event(&self, event: AgentEvent) {
        let _ = self.tx.send(event).await;
    }
}

struct PermissionRequest {
    tool: String,
    reason: String,
    reply: oneshot::Sender<bool>,
}

struct TuiPermission {
    req_tx: mpsc::Sender<PermissionRequest>,
    always: Arc<Mutex<HashSet<String>>>,
}

#[async_trait]
impl PermissionChecker for TuiPermission {
    async fn confirm(
        &self,
        tool: &str,
        _args: &serde_json::Value,
        reason: &str,
    ) -> Result<(), PermissionError> {
        if self.already_approved(tool) {
            return Ok(());
        }
        let (reply, rx) = oneshot::channel();
        self.req_tx
            .send(PermissionRequest {
                tool: tool.to_string(),
                reason: reason.to_string(),
                reply,
            })
            .await
            .map_err(|_| PermissionError::denied("TUI closed while asking for approval"))?;
        match rx.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(PermissionError::denied("denied by user")),
            Err(_) => Err(PermissionError::denied(
                "TUI closed while waiting for approval",
            )),
        }
    }
}

impl TuiPermission {
    fn already_approved(&self, tool: &str) -> bool {
        self.always.lock().unwrap().contains(tool)
    }
}

/// Forwards `ask_user` questions to the UI thread, which shows a modal; the
/// agent blocks until the user answers or dismisses it.
struct TuiAsker {
    req_tx: mpsc::Sender<QuestionRequest>,
}

#[async_trait]
impl Asker for TuiAsker {
    async fn ask(&self, question: &str, options: &[String]) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.req_tx
            .send(QuestionRequest {
                question: question.to_string(),
                options: options.to_vec(),
                reply,
            })
            .await
            .map_err(|_| "TUI closed while asking a question".to_string())?;
        rx.await
            .map_err(|_| "TUI closed while waiting for an answer".to_string())?
            .ok_or_else(|| "user declined the question".to_string())
    }
}

struct App {
    items: Vec<Item>,
    input: Vec<char>,
    cursor: usize,
    /// Selection inside the input box (anchor char index, current char index)
    input_sel: Option<(usize, usize)>,
    /// Collapsed paste blocks: placeholder text + original text
    paste_blocks: Vec<PasteBlock>,
    paste_burst: PasteBurst,
    history: Vec<String>,
    history_pos: Option<usize>,
    busy: bool,
    ai_thinking: bool,
    /// Tools currently running (raw name, activity label) for status hints.
    active_tools: Vec<(String, String)>,
    permission: Option<PermissionRequest>,
    /// Pending `ask_user` question shown as a modal; the agent is blocked until
    /// the user answers or dismisses it.
    question: Option<QuestionRequest>,
    /// Free-form answer being typed into the question modal.
    question_input: Vec<char>,
    interrupting: bool,
    scroll: usize,
    max_offset: usize,
    follow: bool,
    input_scroll: usize,
    quit: bool,
    model: String,
    provider: String,
    thinking: ThinkingLevel,
    mode: SessionMode,
    /// Set by `/new`: the transcript was cleared locally; events from the old
    /// turn are suppressed until `SessionLoaded` for the fresh session arrives.
    pending_new_session: bool,
    /// Items index captured by `/new`; messages added after it (e.g. a message
    /// typed and sent while the fresh session is still loading) survive the
    /// transcript clear in `SessionLoaded`.
    pending_new_baseline: usize,
    model_picker: Option<ModelPicker>,
    session_picker: Option<SessionPicker>,
    transcript_rect: Rect,
    input_rect: Rect,
    content_width: usize,
    input_width: usize,
    selection: Option<Selection>,
    cwd: PathBuf,
    config_path: PathBuf,
    cmd_tx: mpsc::Sender<AgentCmd>,
    always: Arc<Mutex<HashSet<String>>>,
    frame: u64,
}

struct PasteBlock {
    placeholder: String,
    text: String,
}

/// Paste-burst detection.
///
/// When bracketed paste is not enabled, Windows terminals inject pasted text
/// as a rapid stream of keystrokes (often ending with Enter). `PasteBurst`
/// recognizes plain-text keys arriving within 35ms as one paste: Enter counts
/// as a newline instead of submit during the burst, and the whole buffer is
/// collapsed into one paste after it goes quiet.
#[derive(Debug, Default)]
struct PasteBurst {
    /// Arrival time of the previous plain-text char, used to detect a burst.
    last_char_time: Option<Instant>,
    /// Most recent char inserted directly into the input, with its position
    /// (used for retro-capture).
    last_inserted: Option<(usize, char)>,
    /// First ASCII char being held while waiting for a second char to confirm
    /// a burst.
    held: Option<(char, Instant)>,
    /// Confirmed paste buffer.
    buffer: Option<String>,
    /// Last write time of the buffer.
    buffer_last_update: Option<Instant>,
    /// Enters arriving before this time are treated as newlines (protection
    /// window after a burst flushes).
    suppress_enter_until: Option<Instant>,
    /// Outputs waiting to be applied by the App.
    out: VecDeque<PasteOut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PasteOut {
    /// Insert into the input as a normal char.
    InsertChar(char),
    /// Remove the char at this position (reclaim the retro-captured prefix).
    RemoveAt(usize, char),
    /// Insert as one paste (auto-collapsed).
    HandlePaste(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnterAction {
    Submit,
    Newline,
    BufferNewline,
}

impl PasteBurst {
    const BURST_INTERVAL: Duration = Duration::from_millis(35);
    const HOLD_DELAY: Duration = Duration::from_millis(30);
    const FLUSH_DELAY: Duration = Duration::from_millis(80);
    const SUPPRESS_WINDOW: Duration = Duration::from_millis(120);

    /// Handle a plain char with no modifiers. `cursor` is where it will be
    /// inserted.
    fn on_plain_char(&mut self, c: char, now: Instant, cursor: usize) {
        let prev_time = self.last_char_time;
        self.last_char_time = Some(now);

        // Burst confirmed: keep appending to the buffer.
        if let Some(buf) = &mut self.buffer {
            buf.push(c);
            self.buffer_last_update = Some(now);
            return;
        }

        // A first char is held: a second char arriving quickly confirms a burst.
        if let Some((held, at)) = self.held.take() {
            if now.duration_since(at) <= Self::HOLD_DELAY {
                let mut buf = String::with_capacity(2);
                buf.push(held);
                buf.push(c);
                self.buffer = Some(buf);
                self.buffer_last_update = Some(now);
                self.last_inserted = None;
                return;
            }
            // Hold timed out: emit the old char as normal input.
            self.out.push_back(PasteOut::InsertChar(held));
        }

        // Retro-capture: a non-ASCII first char was inserted immediately; when
        // a second char arrives quickly, reclaim that prefix into the buffer
        // (so pasted CJK text does not leave a stray first char).
        if let (Some(at), Some((pos, prev))) = (prev_time, self.last_inserted)
            && now.duration_since(at) <= Self::BURST_INTERVAL
        {
            let mut buf = String::with_capacity(2);
            buf.push(prev);
            buf.push(c);
            self.buffer = Some(buf);
            self.buffer_last_update = Some(now);
            self.last_inserted = None;
            self.out.push_back(PasteOut::RemoveAt(pos, prev));
            return;
        }

        if c.is_ascii() {
            // Hold ASCII briefly: detects bursts without flicker on single keys.
            self.held = Some((c, now));
            self.last_inserted = None;
        } else {
            // Non-ASCII (IME/CJK) is not held; insert immediately and record
            // the position.
            self.last_inserted = Some((cursor, c));
            self.out.push_back(PasteOut::InsertChar(c));
        }
    }

    /// Enter: returns Newline while a burst is active or within the protection
    /// window, otherwise Submit.
    fn on_enter(&mut self, now: Instant) -> EnterAction {
        if let Some((held, _)) = self.held.take() {
            // The first char is still held: release it into the input and treat
            // Enter as a newline, so a trailing Enter in a single-char paste
            // cannot submit.
            self.out.push_back(PasteOut::InsertChar(held));
            return EnterAction::Newline;
        }
        if self.buffer.is_some() || self.suppress_enter_until.is_some_and(|t| now <= t) {
            if let Some(buf) = &mut self.buffer {
                buf.push('\n');
                self.buffer_last_update = Some(now);
                EnterAction::BufferNewline
            } else {
                EnterAction::Newline
            }
        } else {
            EnterAction::Submit
        }
    }

    /// Shift+Enter: merged into the buffer during a burst, otherwise normal
    /// newline behavior.
    fn on_shift_enter(&mut self, now: Instant) -> EnterAction {
        if let Some(buf) = &mut self.buffer {
            buf.push('\n');
            self.buffer_last_update = Some(now);
            EnterAction::BufferNewline
        } else {
            EnterAction::Newline
        }
    }

    /// Due-time handling: a held char past its timeout is emitted as normal
    /// input; a buffer idle past its timeout is flushed as one paste.
    fn flush_if_due(&mut self, now: Instant) {
        if let Some((c, at)) = self.held
            && now.duration_since(at) >= Self::HOLD_DELAY
        {
            self.held = None;
            self.out.push_back(PasteOut::InsertChar(c));
        }
        let ready = self
            .buffer_last_update
            .is_some_and(|at| now.duration_since(at) >= Self::FLUSH_DELAY);
        if ready && let Some(text) = self.buffer.take() {
            self.buffer_last_update = None;
            self.last_inserted = None;
            self.out.push_back(PasteOut::HandlePaste(text));
            self.suppress_enter_until = Some(now + Self::SUPPRESS_WINDOW);
        }
    }

    /// Clear burst state; a held char must not be lost, so queue it as output
    /// first.
    fn clear(&mut self) {
        if let Some((c, _)) = self.held.take() {
            self.out.push_back(PasteOut::InsertChar(c));
        }
        self.last_char_time = None;
        self.last_inserted = None;
        self.buffer = None;
        self.buffer_last_update = None;
        self.suppress_enter_until = None;
    }

    fn drain_outputs(&mut self) -> Vec<PasteOut> {
        self.out.drain(..).collect()
    }
}

struct ModelPicker {
    query: Vec<char>,
    models: Vec<String>,
    selected: usize,
}

impl ModelPicker {
    fn new(models: Vec<String>) -> Self {
        Self {
            query: Vec::new(),
            models,
            selected: 0,
        }
    }

    fn filtered(&self) -> Vec<&str> {
        let query: String = self.query.iter().collect();
        let query = query.to_lowercase();
        if query.is_empty() {
            return self.models.iter().map(|m| m.as_str()).collect();
        }
        self.models
            .iter()
            .filter(|m| m.to_lowercase().contains(&query))
            .map(|m| m.as_str())
            .collect()
    }

    fn clamp(&mut self) {
        let count = self.filtered().len();
        self.selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
    }

    fn selected_model(&self) -> Option<String> {
        self.filtered().get(self.selected).map(|m| m.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor_row: usize,
    anchor_col: usize,
    row: usize,
    col: usize,
}

impl Selection {
    fn normalized(self) -> ((usize, usize), (usize, usize)) {
        if (self.anchor_row, self.anchor_col) <= (self.row, self.col) {
            ((self.anchor_row, self.anchor_col), (self.row, self.col))
        } else {
            ((self.row, self.col), (self.anchor_row, self.anchor_col))
        }
    }
}

struct SessionPicker {
    query: Vec<char>,
    sessions: Vec<firment_core::SessionSummary>,
    selected: usize,
}

impl SessionPicker {
    fn new(sessions: Vec<firment_core::SessionSummary>) -> Self {
        Self {
            query: Vec::new(),
            sessions,
            selected: 0,
        }
    }

    fn filtered(&self) -> Vec<&firment_core::SessionSummary> {
        let query: String = self.query.iter().collect();
        let query = query.to_lowercase();
        if query.is_empty() {
            return self.sessions.iter().collect();
        }
        self.sessions
            .iter()
            .filter(|s| {
                s.id.to_lowercase().contains(&query)
                    || s.model.to_lowercase().contains(&query)
                    || s.preview.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn clamp(&mut self) {
        let count = self.filtered().len();
        self.selected = if count == 0 {
            0
        } else {
            self.selected.min(count - 1)
        };
    }
}

enum Item {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        running: bool,
        ok: bool,
        summary: String,
    },
    /// Permission confirmations render as inline cards in the transcript
    /// instead of popups covering the context.
    Permission {
        tool: String,
        reason: String,
    },
    System(String),
    Error(String),
}

impl App {
    #[allow(clippy::too_many_arguments)]
    fn new(
        cmd_tx: mpsc::Sender<AgentCmd>,
        always: Arc<Mutex<HashSet<String>>>,
        model: String,
        cwd: PathBuf,
        provider: String,
        thinking: ThinkingLevel,
        mode: SessionMode,
        config_path: PathBuf,
        startup_hint: Option<String>,
        initial_messages: Vec<ChatMessage>,
    ) -> Self {
        let mut app = Self {
            items: Vec::new(),
            input: Vec::new(),
            cursor: 0,
            input_sel: None,
            paste_blocks: Vec::new(),
            paste_burst: PasteBurst::default(),
            history: Vec::new(),
            history_pos: None,
            busy: false,
            ai_thinking: false,
            active_tools: Vec::new(),
            permission: None,
            question: None,
            question_input: Vec::new(),
            interrupting: false,
            scroll: 0,
            max_offset: 0,
            follow: true,
            input_scroll: 0,
            quit: false,
            model,
            provider,
            thinking,
            mode,
            pending_new_session: false,
            pending_new_baseline: 0,
            model_picker: None,
            session_picker: None,
            transcript_rect: Rect::default(),
            input_rect: Rect::default(),
            content_width: 0,
            input_width: 80,
            selection: None,
            cwd,
            config_path,
            cmd_tx,
            always,
            frame: 0,
        };
        if let Some(hint) = startup_hint {
            app.items.push(Item::System(hint));
        }
        app.push_messages(&initial_messages);
        app
    }

    fn push_messages(&mut self, messages: &[ChatMessage]) {
        for message in messages {
            match message {
                ChatMessage::User { content } => {
                    self.items.push(Item::User(content.clone()));
                }
                ChatMessage::Assistant { content, .. } => {
                    self.items.push(Item::Assistant(content.clone()));
                }
                ChatMessage::Tool { name, content, .. } => {
                    let ok = !content.starts_with("Permission denied")
                        && !content.starts_with("unknown tool")
                        && !content.starts_with("[Permission] Dangerous command");
                    self.items.push(Item::Tool {
                        name: name.clone(),
                        running: false,
                        ok,
                        summary: content.clone(),
                    });
                }
                ChatMessage::System { content } => {
                    self.items.push(Item::System(content.clone()));
                }
            }
        }
    }

    fn on_agent(&mut self, event: AgentEvent) {
        // While `/new` is in flight, ignore events from the old turn (stream
        // deltas, tool cards, interrupt/rollback messages) so they cannot leak
        // into the fresh conversation.
        if self.pending_new_session && !matches!(&event, AgentEvent::SessionLoaded(_)) {
            return;
        }
        match event {
            AgentEvent::TurnStart => {
                self.busy = true;
                self.ai_thinking = true;
            }
            AgentEvent::TextDelta(text) => match self.items.last_mut() {
                Some(Item::Assistant(buffer)) => {
                    self.ai_thinking = false;
                    buffer.push_str(&text);
                }
                _ => {
                    self.ai_thinking = false;
                    self.items.push(Item::Assistant(text));
                }
            },
            AgentEvent::ToolStart { name, args } => {
                self.ai_thinking = false;
                self.active_tools
                    .push((name.clone(), tool_activity(&name, &args)));
                self.items.push(Item::Tool {
                    name,
                    running: true,
                    ok: false,
                    summary: args.to_string(),
                });
            }
            AgentEvent::ToolEnd { name, ok, summary } => {
                if let Some(pos) = self.active_tools.iter().rposition(|(n, _)| n == &name) {
                    self.active_tools.remove(pos);
                }
                for item in self.items.iter_mut().rev() {
                    if let Item::Tool {
                        name: n,
                        running,
                        ok: current_ok,
                        summary: current_summary,
                    } = item
                        && n == &name
                        && *running
                    {
                        *running = false;
                        *current_ok = ok;
                        *current_summary = summary;
                        break;
                    }
                }
            }
            AgentEvent::TurnEnd { .. } => {
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
            }
            AgentEvent::Info(message) => self.items.push(Item::System(message)),
            AgentEvent::Settings {
                provider,
                model,
                thinking,
                mode,
            } => {
                if let Some(provider) = provider {
                    self.provider = provider;
                }
                if let Some(model) = model {
                    self.model = model;
                }
                if let Some(thinking) = thinking {
                    self.thinking = thinking;
                }
                if let Some(mode) = mode {
                    self.mode = mode;
                }
            }
            AgentEvent::Models(models) => match &mut self.model_picker {
                Some(picker) => {
                    picker.models = models;
                    picker.clamp();
                }
                None => {
                    self.model_picker = Some(ModelPicker::new(models));
                }
            },
            AgentEvent::Sessions(sessions) => match &mut self.session_picker {
                Some(picker) => {
                    picker.sessions = sessions;
                    picker.clamp();
                }
                None => {
                    self.session_picker = Some(SessionPicker::new(sessions));
                }
            },
            AgentEvent::SessionLoaded(session) => {
                let was_new = self.pending_new_session;
                self.pending_new_session = false;
                // Keep anything the user added after `/new` (e.g. a message
                // typed and sent while the fresh session was loading).
                let keep = if was_new {
                    self.items.split_off(self.pending_new_baseline)
                } else {
                    Vec::new()
                };
                self.items.clear();
                self.provider = session.provider.clone();
                self.model = session.model.clone();
                self.thinking = session.thinking;
                self.mode = session.mode;
                self.cwd = session.cwd.clone();
                self.busy = false;
                self.ai_thinking = false;
                self.permission = None;
                self.follow = true;
                self.scroll = 0;
                self.max_offset = 0;
                self.input_scroll = 0;
                self.input_sel = None;
                self.paste_blocks.clear();
                self.interrupting = false;
                if was_new {
                    self.items
                        .push(Item::System("New conversation started".to_string()));
                }
                self.items.extend(keep);
                self.push_messages(&session.messages);
            }
            AgentEvent::Error(message) => {
                self.items.push(Item::Error(message));
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
            }
        }
    }

    fn on_permission(&mut self, request: PermissionRequest) {
        self.items.push(Item::Permission {
            tool: request.tool.clone(),
            reason: request.reason.clone(),
        });
        // The inline card must be visible; force the view back to the bottom.
        self.follow = true;
        self.scroll = 0;
        self.permission = Some(request);
    }

    fn on_question(&mut self, request: QuestionRequest) {
        self.question_input.clear();
        self.items
            .push(Item::System(format!("❓ {}", request.question)));
        // The question modal must be visible; force the view back to the bottom.
        self.follow = true;
        self.scroll = 0;
        self.question = Some(request);
    }

    fn on_question_key(&mut self, key: KeyEvent) -> bool {
        let Some(question) = self.question.take() else {
            return false;
        };
        let answer = match key.code {
            KeyCode::Char(d) if d.is_ascii_digit() && d != '0' => {
                let idx = (d as usize) - ('1' as usize);
                question.options.get(idx).cloned()
            }
            KeyCode::Enter => {
                let typed: String = self.question_input.iter().collect();
                let typed = typed.trim().to_string();
                if typed.is_empty() { None } else { Some(typed) }
            }
            KeyCode::Backspace => {
                self.question_input.pop();
                self.question = Some(question);
                return false;
            }
            KeyCode::Esc => None,
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.question_input.push(ch);
                self.question = Some(question);
                return false;
            }
            _ => {
                self.question = Some(question);
                return false;
            }
        };
        self.question_input.clear();
        let _ = question.reply.send(answer);
        false
    }

    fn on_ui(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key_with_burst(key),
            Event::Paste(text) => {
                self.paste_burst.clear();
                self.apply_burst_outputs();
                self.insert_text_at_cursor(&text, true);
                false
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_up(3);
                    false
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_down(3);
                    false
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(idx) = self.cell_to_input(mouse.column, mouse.row) {
                        self.cursor = idx;
                        self.input_sel = Some((idx, idx));
                        self.selection = None;
                    } else {
                        self.input_sel = None;
                        self.selection =
                            self.cell_to_content(mouse.column, mouse.row)
                                .map(|(row, col)| Selection {
                                    anchor_row: row,
                                    anchor_col: col,
                                    row,
                                    col,
                                });
                    }
                    false
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some((anchor, _)) = self.input_sel {
                        if let Some(idx) = self.cell_to_input(mouse.column, mouse.row) {
                            self.input_sel = Some((anchor, idx));
                            self.cursor = idx;
                        }
                    } else if let Some((row, col)) = self.cell_to_content(mouse.column, mouse.row)
                        && let Some(selection) = &mut self.selection
                    {
                        selection.row = row;
                        selection.col = col;
                    }
                    false
                }
                MouseEventKind::Up(MouseButton::Left) => false,
                MouseEventKind::Down(MouseButton::Right) => {
                    if self.input_sel.is_some() && self.input_selection_text().is_some() {
                        self.copy_input_selection();
                    } else if self.selection.is_some() {
                        self.copy_selection();
                    } else {
                        self.paste_clipboard();
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Key entry point: runs paste-burst detection first, then falls back to
    /// the original key handling.
    fn on_key_with_burst(&mut self, key: KeyEvent) -> bool {
        if self.permission.is_some()
            || self.question.is_some()
            || self.model_picker.is_some()
            || self.session_picker.is_some()
        {
            return self.on_key(key);
        }
        self.on_key_burst(key, Instant::now())
    }

    /// Key handling with paste-burst detection; `now` lets tests inject time.
    fn on_key_burst(&mut self, key: KeyEvent, now: Instant) -> bool {
        self.paste_burst.flush_if_due(now);
        match key.code {
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.paste_burst.on_plain_char(ch, now, self.cursor);
                self.apply_burst_outputs_at(now);
                false
            }
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let action = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.paste_burst.on_shift_enter(now)
                } else {
                    self.paste_burst.on_enter(now)
                };
                match action {
                    EnterAction::Submit => {
                        self.paste_burst.clear();
                        self.apply_burst_outputs_at(now);
                        self.submit();
                    }
                    EnterAction::Newline => {
                        self.apply_burst_outputs_at(now);
                        self.insert_char('\n');
                    }
                    EnterAction::BufferNewline => {}
                }
                false
            }
            _ => {
                self.paste_burst.clear();
                self.apply_burst_outputs_at(now);
                self.on_key(key)
            }
        }
    }

    /// Apply outputs queued by the paste burst; returns whether anything was
    /// applied.
    fn apply_burst_outputs_at(&mut self, now: Instant) -> bool {
        self.paste_burst.flush_if_due(now);
        let mut applied = false;
        for out in self.paste_burst.drain_outputs() {
            match out {
                PasteOut::InsertChar(c) => self.insert_char(c),
                PasteOut::RemoveAt(pos, expected) => {
                    if pos < self.input.len() && self.input[pos] == expected {
                        self.input.remove(pos);
                        if self.cursor > pos {
                            self.cursor -= 1;
                        }
                    }
                }
                PasteOut::HandlePaste(text) => self.insert_text_at_cursor(&text, true),
            }
            applied = true;
        }
        applied
    }

    fn apply_burst_outputs(&mut self) -> bool {
        self.apply_burst_outputs_at(Instant::now())
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        if self.permission.is_some() {
            return self.on_permission_key(key);
        }
        if self.question.is_some() {
            return self.on_question_key(key);
        }
        if self.model_picker.is_some() {
            return self.on_picker_key(key);
        }
        if self.session_picker.is_some() {
            return self.on_session_picker_key(key);
        }
        match key.code {
            KeyCode::Char('c')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.copy_last_output();
                false
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.copy_primary_selection();
                false
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.paste_clipboard();
                false
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
                true
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_model_picker();
                false
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                self.input_sel = None;
                false
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.input.len();
                self.input_sel = None;
                false
            }
            KeyCode::Char(ch) => {
                self.insert_char(ch);
                false
            }
            KeyCode::Backspace => {
                self.backspace();
                false
            }
            KeyCode::Delete => {
                self.delete_char();
                false
            }
            KeyCode::Left => {
                self.move_cursor_left();
                false
            }
            KeyCode::Right => {
                self.move_cursor_right();
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.input_sel = None;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                self.input_sel = None;
                false
            }
            KeyCode::Up => {
                if self.history_pos.is_some() || (self.input.is_empty() && !self.history.is_empty())
                {
                    self.history_up();
                } else if !self.input.is_empty() && self.input_line_count() > 1 {
                    self.move_input_cursor(-1);
                } else {
                    self.scroll_up(1);
                }
                false
            }
            KeyCode::Down => {
                if self.history_pos.is_some() {
                    self.history_down();
                } else if !self.input.is_empty() && self.input_line_count() > 1 {
                    self.move_input_cursor(1);
                } else {
                    self.scroll_down(1);
                }
                false
            }
            KeyCode::PageUp => {
                self.scroll_up(10);
                false
            }
            KeyCode::PageDown => {
                self.scroll_down(10);
                false
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_char('\n');
                false
            }
            KeyCode::Enter => {
                self.submit();
                false
            }
            KeyCode::Esc => {
                if self.busy {
                    self.request_interrupt();
                } else {
                    self.input.clear();
                    self.cursor = 0;
                    self.input_sel = None;
                    self.paste_blocks.clear();
                }
                false
            }
            _ => false,
        }
    }

    fn request_interrupt(&mut self) {
        if self.interrupting {
            return;
        }
        self.interrupting = true;
        let _ = self.cmd_tx.try_send(AgentCmd::Cancel);
        self.items
            .push(Item::System("⏹ Interrupt request sent…".to_string()));
    }

    /// Soft-wrap the input to the display width; returns (lines, line start
    /// char indexes, cursor line, cursor column). Cursor positions are char
    /// indexes and account for CJK wide chars.
    fn input_layout(&self, width: usize) -> (Vec<String>, Vec<usize>, usize, usize) {
        let chars = &self.input;
        let mut lines: Vec<String> = Vec::new();
        let mut line_starts: Vec<usize> = Vec::new();
        let mut line_start = 0usize;
        let mut current = String::new();
        let mut current_w = 0usize;
        for (pos, &ch) in chars.iter().enumerate() {
            if ch == '\n' {
                lines.push(std::mem::take(&mut current));
                line_starts.push(line_start);
                current_w = 0;
                line_start = pos + 1;
                continue;
            }
            let w = ch.width().unwrap_or(0);
            if current_w + w > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                line_starts.push(line_start);
                line_start = pos;
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(current);
            line_starts.push(line_start);
        }
        let cursor = self.cursor.min(chars.len());
        let cursor_line = line_starts
            .iter()
            .rposition(|&start| cursor >= start)
            .unwrap_or(0);
        let cursor_col: usize = chars[line_starts[cursor_line]..cursor]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum();
        (lines, line_starts, cursor_line, cursor_col)
    }

    fn on_permission_key(&mut self, key: KeyEvent) -> bool {
        let Some(prompt) = self.permission.take() else {
            return false;
        };
        let (allowed, always) = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => (true, false),
            KeyCode::Char('a') | KeyCode::Char('A') => (true, true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => (false, false),
            _ => {
                self.permission = Some(prompt);
                return false;
            }
        };
        if always {
            self.always.lock().unwrap().insert(prompt.tool.clone());
        }
        let _ = prompt.reply.send(allowed);
        if let Some(idx) = self
            .items
            .iter()
            .rposition(|item| matches!(item, Item::Permission { .. }))
        {
            self.items.remove(idx);
        }
        self.items.push(Item::System(format!(
            "{}: {}",
            if allowed { "✓ Allowed" } else { "✗ Denied" },
            prompt.tool
        )));
        false
    }

    fn insert_char(&mut self, ch: char) {
        self.history_pos = None;
        self.input_sel = None;
        let cursor = self.snap_cursor(self.cursor);
        self.cursor = cursor;
        if cursor >= self.input.len() {
            self.input.push(ch);
        } else {
            self.input.insert(cursor, ch);
        }
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        self.input_sel = None;
        if self.cursor == 0 {
            return;
        }
        if let Some((start, end, idx)) = self.placeholder_range_with_end(self.cursor) {
            self.input.drain(start..end);
            self.paste_blocks.remove(idx);
            self.cursor = start;
            return;
        }
        self.history_pos = None;
        self.input.remove(self.cursor - 1);
        self.cursor -= 1;
    }

    fn delete_char(&mut self) {
        self.input_sel = None;
        if let Some((start, end, idx)) = self.placeholder_range_with_start(self.cursor) {
            self.input.drain(start..end);
            self.paste_blocks.remove(idx);
            return;
        }
        if self.cursor < self.input.len() {
            self.history_pos = None;
            self.input.remove(self.cursor);
        }
    }

    /// Move the cursor left; a collapsed placeholder is treated as one unit
    /// (jump from its tail to its head).
    fn move_cursor_left(&mut self) {
        self.input_sel = None;
        if let Some((start, _end, _)) = self.placeholder_range_with_end(self.cursor) {
            self.cursor = start;
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.cursor = self.snap_cursor(self.cursor);
    }

    /// Move the cursor right; a collapsed placeholder is treated as one unit
    /// (jump from its head to its tail).
    fn move_cursor_right(&mut self) {
        self.input_sel = None;
        if let Some((_start, end, _)) = self.placeholder_range_with_start(self.cursor) {
            self.cursor = end;
            return;
        }
        self.cursor = (self.cursor + 1).min(self.input.len());
        self.cursor = self.snap_cursor(self.cursor);
    }

    /// Move the cursor up/down inside the input (by display line), keeping the
    /// column position.
    fn move_input_cursor(&mut self, delta: isize) {
        if self.input.is_empty() {
            return;
        }
        let (lines, line_starts, cursor_line, cursor_col) =
            self.input_layout(self.input_width.max(1));
        let target_line = if delta < 0 {
            cursor_line.saturating_sub(1)
        } else {
            (cursor_line + 1).min(lines.len().saturating_sub(1))
        };
        if target_line == cursor_line {
            return;
        }
        let line_text = &lines[target_line];
        let col = cursor_col.min(cell_width(line_text));
        let char_in_line = char_index_at_cell(line_text, col);
        let target = line_starts[target_line] + char_in_line;
        self.cursor = self.snap_cursor(target);
        self.history_pos = None;
        self.input_sel = None;
    }

    fn input_line_count(&self) -> usize {
        if self.input.is_empty() {
            return 0;
        }
        self.input_layout(self.input_width.max(1)).0.len()
    }

    /// Positions of all collapsed placeholders in the current input (in input
    /// order).
    fn placeholder_ranges(&self) -> Vec<(usize, usize, usize)> {
        let mut ranges = Vec::new();
        let mut search_from = 0usize;
        for (idx, block) in self.paste_blocks.iter().enumerate() {
            let needle: Vec<char> = block.placeholder.chars().collect();
            if let Some(pos) = find_subslice(&self.input, &needle, search_from) {
                ranges.push((pos, pos + needle.len(), idx));
                search_from = pos + needle.len();
            }
        }
        ranges
    }

    fn placeholder_range_with_end(&self, cursor: usize) -> Option<(usize, usize, usize)> {
        self.placeholder_ranges()
            .into_iter()
            .find(|(_, end, _)| *end == cursor)
    }

    fn placeholder_range_with_start(&self, cursor: usize) -> Option<(usize, usize, usize)> {
        self.placeholder_ranges()
            .into_iter()
            .find(|(start, _, _)| *start == cursor)
    }

    /// If the cursor lands inside a collapsed placeholder, jump to its end.
    fn snap_cursor(&self, cursor: usize) -> usize {
        for (start, end, _) in self.placeholder_ranges() {
            if start < cursor && cursor < end {
                return end;
            }
        }
        cursor
    }

    /// Expand the input to full text: placeholders are replaced by their
    /// original pasted content.
    fn expand_input(&self) -> String {
        let chars = &self.input;
        let mut out = String::new();
        let mut last = 0usize;
        for (start, end, idx) in self.placeholder_ranges() {
            out.extend(chars[last..start].iter());
            out.push_str(&self.paste_blocks[idx].text);
            last = end;
        }
        out.extend(chars[last..].iter());
        out
    }

    /// Full text of the input selection (with collapsed blocks expanded).
    fn input_selection_text(&self) -> Option<String> {
        let (a, b) = self.input_sel?;
        let (s0, s1) = if a <= b { (a, b) } else { (b, a) };
        if s0 == s1 {
            return None;
        }
        let chars = &self.input;
        let mut out = String::new();
        let mut last = s0;
        for (start, end, idx) in self.placeholder_ranges() {
            if start >= s1 {
                break;
            }
            if end <= s0 {
                continue;
            }
            let seg_start = start.max(s0);
            let seg_end = end.min(s1);
            if last < seg_start {
                out.extend(chars[last..seg_start].iter());
            }
            out.push_str(&self.paste_blocks[idx].text);
            last = seg_end;
        }
        if last < s1 {
            out.extend(chars[last..s1].iter());
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Insert text at the cursor; with `collapse` true, large text is folded
    /// into a placeholder.
    fn insert_text_at_cursor(&mut self, text: &str, collapse: bool) {
        let text: String = text.chars().filter(|c| *c != '\r').collect();
        if text.is_empty() {
            return;
        }
        self.history_pos = None;
        self.input_sel = None;
        let start = self.snap_cursor(self.cursor);
        self.cursor = start;
        let insert: Vec<char> = if collapse && Self::needs_collapse(&text) {
            let placeholder = Self::collapse_label(&text, &self.paste_blocks);
            self.paste_blocks.push(PasteBlock {
                placeholder: placeholder.clone(),
                text: text.clone(),
            });
            placeholder.chars().collect()
        } else {
            text.chars().collect()
        };
        if start >= self.input.len() {
            self.input.extend(insert.iter().copied());
        } else {
            self.input.splice(start..start, insert.iter().copied());
        }
        self.cursor = start + insert.len();
    }

    fn needs_collapse(text: &str) -> bool {
        text.lines().count() > 2 || text.chars().count() > 150
    }

    fn collapse_label(text: &str, existing: &[PasteBlock]) -> String {
        let line_count = text.lines().count().max(1);
        let mut label = if line_count > 1 {
            format!("【line 1-{line_count}】")
        } else {
            format!("【collapsed {} chars】", text.chars().count())
        };
        let mut id = 1usize;
        while existing.iter().any(|b| b.placeholder == label) {
            id += 1;
            label = if line_count > 1 {
                format!("【line 1-{line_count}#{id}】")
            } else {
                format!("【collapsed {} chars#{id}】", text.chars().count())
            };
        }
        label
    }

    fn copy_input_selection(&mut self) {
        let Some(text) = self.input_selection_text() else {
            return;
        };
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match copy_to_clipboard(text) {
            Ok(()) => self.items.push(Item::System(format!(
                "Copied input selection ({} chars)",
                text.chars().count()
            ))),
            Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
        }
    }

    /// Ctrl+C priority: input selection > transcript selection > last reply.
    fn copy_primary_selection(&mut self) {
        if self.input_sel.is_some() && self.input_selection_text().is_some() {
            self.copy_input_selection();
        } else if self.selection.is_some() {
            self.copy_selection();
        } else if let Some(text) = self.last_output_text() {
            match copy_to_clipboard(&text) {
                Ok(()) => self.items.push(Item::System(format!(
                    "Copied the last reply ({} chars)",
                    text.chars().count()
                ))),
                Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
            }
        }
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.history_pos {
            Some(pos) if pos > 0 => pos - 1,
            _ => self.history.len() - 1,
        };
        self.history_pos = Some(next);
        self.input = self.history[next].chars().collect();
        self.cursor = self.input.len();
        self.paste_blocks.clear();
        self.input_sel = None;
    }

    fn history_down(&mut self) {
        let Some(pos) = self.history_pos else {
            return;
        };
        if pos + 1 < self.history.len() {
            let next = pos + 1;
            self.history_pos = Some(next);
            self.input = self.history[next].chars().collect();
            self.cursor = self.input.len();
            self.paste_blocks.clear();
            self.input_sel = None;
        } else {
            self.history_pos = None;
            self.input.clear();
            self.cursor = 0;
            self.paste_blocks.clear();
            self.input_sel = None;
        }
    }

    fn open_model_picker(&mut self) {
        if self.model_picker.is_some() {
            return;
        }
        self.model_picker = Some(ModelPicker::new(Vec::new()));
        let _ = self.cmd_tx.try_send(AgentCmd::OpenModelPicker);
    }

    fn on_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = self.model_picker.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.model_picker = None;
            }
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                let count = picker.filtered().len();
                if count > 0 && picker.selected + 1 < count {
                    picker.selected += 1;
                }
            }
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => {
                let count = picker.filtered().len();
                if count > 0 {
                    picker.selected = count - 1;
                }
            }
            KeyCode::Enter => {
                if let Some(model) = picker.selected_model() {
                    self.model = model.clone();
                    self.items
                        .push(Item::System(format!("model -> {model} (switching…)")));
                    let _ = self.cmd_tx.try_send(AgentCmd::SetModel(model));
                }
                self.model_picker = None;
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.clamp();
            }
            KeyCode::Char(ch) => {
                picker.query.push(ch);
                picker.clamp();
            }
            _ => {}
        }
        false
    }

    fn open_session_picker(&mut self) {
        if self.session_picker.is_some() {
            return;
        }
        self.session_picker = Some(SessionPicker::new(Vec::new()));
        let _ = self.cmd_tx.try_send(AgentCmd::OpenSessionPicker);
    }

    fn on_session_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = self.session_picker.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.session_picker = None;
            }
            KeyCode::Up => {
                if picker.selected > 0 {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down => {
                let count = picker.filtered().len();
                if count > 0 && picker.selected + 1 < count {
                    picker.selected += 1;
                }
            }
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => {
                let count = picker.filtered().len();
                if count > 0 {
                    picker.selected = count - 1;
                }
            }
            KeyCode::Enter => {
                if let Some(session) = picker.filtered().get(picker.selected) {
                    let id = session.id.clone();
                    self.items
                        .push(Item::System(format!("Loading session {id}…")));
                    let _ = self.cmd_tx.try_send(AgentCmd::LoadSession(id));
                }
                self.session_picker = None;
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.clamp();
            }
            KeyCode::Char(ch) => {
                picker.query.push(ch);
                picker.clamp();
            }
            _ => {}
        }
        false
    }

    fn cell_to_content(&self, column: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.transcript_rect;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if row <= area.y || row >= area.y + area.height - 1 {
            return None;
        }
        if column <= area.x || column >= area.x + area.width - 1 {
            return None;
        }
        let visible = (row - area.y - 1) as usize;
        let content_row = self.offset().saturating_add(visible);
        Some((content_row, (column - area.x - 1) as usize))
    }

    /// Terminal cell → input char index (for click/drag selection in the input
    /// box).
    fn cell_to_input(&self, column: u16, row: u16) -> Option<usize> {
        let area = self.input_rect;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if row <= area.y || row >= area.y + area.height - 1 {
            return None;
        }
        if column <= area.x || column >= area.x + area.width - 1 {
            return None;
        }
        if self.input.is_empty() {
            return Some(0);
        }
        let (lines, line_starts, _, _) = self.input_layout(self.input_width.max(1));
        let line = (row - area.y - 1) as usize + self.input_scroll;
        let line_text = lines.get(line)?;
        let col = (column - area.x - 1) as usize;
        let char_in_line = char_index_at_cell(line_text, col.min(cell_width(line_text)));
        let idx = line_starts[line] + char_in_line;
        Some(self.snap_cursor(idx))
    }

    fn offset(&self) -> usize {
        if self.follow {
            self.max_offset
        } else {
            self.max_offset.saturating_sub(self.scroll)
        }
    }

    fn selection_text(&self, selection: Selection) -> String {
        let width = self.content_width.max(1);
        let rows = self.render_rows(width);
        let ((r0, c0), (r1, c1)) = selection.normalized();
        let mut out = Vec::new();
        for row_idx in r0..=r1 {
            let Some(row) = rows.get(row_idx) else {
                break;
            };
            let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
            let (start, end) = if r0 == r1 {
                (c0, c1)
            } else if row_idx == r0 {
                (c0, usize::MAX)
            } else if row_idx == r1 {
                (0, c1)
            } else {
                (0, usize::MAX)
            };
            let total_cells = cell_width(&text);
            let start = start.min(total_cells);
            let end = end.min(total_cells);
            let char_start = char_index_at_cell(&text, start);
            let char_end = char_index_at_cell(&text, end);
            out.push(
                text.chars()
                    .skip(char_start)
                    .take(char_end.saturating_sub(char_start))
                    .collect::<String>(),
            );
        }
        out.join("\n")
    }

    fn copy_selection(&mut self) {
        let Some(selection) = self.selection.take() else {
            return;
        };
        let text = self.selection_text(selection);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match copy_to_clipboard(text) {
            Ok(()) => self.items.push(Item::System(format!(
                "Copied selection ({} chars)",
                text.chars().count()
            ))),
            Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
        }
    }

    fn paste_clipboard(&mut self) {
        let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else {
            return;
        };
        self.insert_text_at_cursor(&text, true);
    }

    fn last_output_text(&self) -> Option<String> {
        self.items.iter().rev().find_map(|item| match item {
            Item::Assistant(text) if !text.trim().is_empty() => Some(text.clone()),
            _ => None,
        })
    }

    fn copy_last_output(&mut self) {
        match self.last_output_text() {
            Some(text) => match copy_to_clipboard(&text) {
                Ok(()) => self.items.push(Item::System(format!(
                    "Copied the last reply ({} chars)",
                    text.chars().count()
                ))),
                Err(e) => self.items.push(Item::System(format!("Copy failed: {e}"))),
            },
            None => self
                .items
                .push(Item::System("No reply to copy yet".to_string())),
        }
    }

    fn highlight_selection(&self, rows: &mut Vec<Line<'static>>) {
        let Some(selection) = self.selection else {
            return;
        };
        let ((r0, c0), (r1, c1)) = selection.normalized();
        for row_idx in r0..=r1 {
            let Some(row) = rows.get_mut(row_idx) else {
                break;
            };
            let (start, end) = if r0 == r1 {
                (c0, c1)
            } else if row_idx == r0 {
                (c0, usize::MAX)
            } else if row_idx == r1 {
                (0, c1)
            } else {
                (0, usize::MAX)
            };
            let mut col = 0usize;
            let mut new_spans = Vec::new();
            for span in std::mem::take(&mut row.spans) {
                let content: String = span.content.into_owned();
                let span_start = col;
                let span_end = col + cell_width(&content);
                let sel_start = span_start.max(start);
                let sel_end = span_end.min(end);
                if sel_start < sel_end {
                    let char_start = char_index_at_cell(&content, sel_start - span_start);
                    let char_end = char_index_at_cell(&content, sel_end - span_start);
                    let before: String = content.chars().take(char_start).collect();
                    let selected: String = content
                        .chars()
                        .skip(char_start)
                        .take(char_end.saturating_sub(char_start))
                        .collect();
                    let after: String = content.chars().skip(char_end).collect();
                    if !before.is_empty() {
                        new_spans.push(Span::styled(before, span.style));
                    }
                    new_spans.push(Span::styled(
                        selected,
                        span.style.add_modifier(Modifier::REVERSED),
                    ));
                    if !after.is_empty() {
                        new_spans.push(Span::styled(after, span.style));
                    }
                } else {
                    new_spans.push(Span::styled(content, span.style));
                }
                col = span_end;
            }
            row.spans = new_spans;
        }
    }

    fn scroll_up(&mut self, amount: usize) {
        if self.max_offset == 0 {
            return;
        }
        self.follow = false;
        self.scroll = (self.scroll + amount).min(self.max_offset);
    }

    fn scroll_down(&mut self, amount: usize) {
        if self.follow {
            return;
        }
        self.scroll = self.scroll.saturating_sub(amount);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    fn submit(&mut self) {
        let text = self.expand_input();
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) != Some(text.as_str()) {
            self.history.push(text.clone());
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
        self.history_pos = None;
        self.input.clear();
        self.cursor = 0;
        self.input_sel = None;
        self.paste_blocks.clear();
        if let Some(command) = text.strip_prefix('/') {
            self.run_command(command);
            return;
        }
        if self.busy {
            self.items.push(Item::System(
                "Agent is busy; wait for it to finish.".to_string(),
            ));
            return;
        }
        self.items.push(Item::User(text.clone()));
        self.busy = true;
        self.ai_thinking = true;
        self.follow = true;
        self.scroll = 0;
        let _ = self.cmd_tx.try_send(AgentCmd::User(text));
    }

    fn run_command(&mut self, command: &str) {
        let (name, arg) = command
            .split_once(char::is_whitespace)
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((command, ""));
        match name {
            "help" => self.items.push(Item::System(
                "Commands: /new  /plan [on|off]  /agent  /models  /model <id>  /sessions (use ↑/↓ to select)  /session <id>  /undo  /ledger  /pin <path>  /unpin <path>  /copy  /provider <name>  /add-provider <name> <openai|anthropic> <base_url> <model>  /apikey [provider] <key>  /thinking [off|low|medium|high|xhigh|max]  /config  /clear  /help  /quit\nKeys: ↑/↓ browse history when input is empty, move the input cursor on multi-line input, scroll the transcript on single-line input · Shift+Enter manual newline · PgUp/PgDn/wheel always scroll · Ctrl+P model picker · drag with left mouse to select · right-click copies the selection (pastes when there is no selection) · Ctrl+C copies the selection (copies the last reply when there is none) · Ctrl+V paste · Ctrl+Shift+C copy last reply · ←/→ move the input cursor · y/n/a permission answers · Esc interrupts AI output (clears input when idle) · Ctrl+Q quit\nInput box: auto-wraps and grows to up to 5 lines; taller content scrolls, large pastes collapse into 【line x-y】, and the title shows hidden/collapsed line counts before sending"
                    .to_string(),
            )),
            "new" => {
                self.paste_burst.clear();
                self.apply_burst_outputs();
                self.input.clear();
                self.cursor = 0;
                self.input_sel = None;
                self.paste_blocks.clear();
                // Clear the transcript immediately and stop any running turn,
                // so the previous conversation cannot linger on screen while
                // the agent task processes the fresh session.
                let was_busy = self.busy;
                self.items.clear();
                self.busy = false;
                self.ai_thinking = false;
                self.interrupting = false;
                self.permission = None;
                self.follow = true;
                self.scroll = 0;
                self.pending_new_session = true;
                if was_busy {
                    let _ = self.cmd_tx.try_send(AgentCmd::Cancel);
                }
                let _ = self.cmd_tx.try_send(AgentCmd::NewSession);
                self.items
                    .push(Item::System("Starting a new conversation…".to_string()));
                self.pending_new_baseline = self.items.len();
            }
            "plan" => {
                let mode = match arg {
                    "on" => SessionMode::Plan,
                    "off" => SessionMode::Agent,
                    _ if self.mode == SessionMode::Plan => SessionMode::Agent,
                    _ => SessionMode::Plan,
                };
                self.mode = mode;
                let _ = self.cmd_tx.try_send(AgentCmd::SetMode(mode));
                let queued = if self.busy { " (takes effect after the current turn)" } else { "" };
                self.items.push(Item::System(format!(
                    "mode -> {}{queued}",
                    mode.label()
                )));
            }
            "agent" => {
                self.mode = SessionMode::Agent;
                let _ = self.cmd_tx.try_send(AgentCmd::SetMode(SessionMode::Agent));
                let queued = if self.busy { " (takes effect after the current turn)" } else { "" };
                self.items
                    .push(Item::System(format!("mode -> agent{queued}")));
            }
            "thinking" => {
                let level = if arg.is_empty() {
                    next_thinking(self.thinking)
                } else {
                    match arg.parse::<ThinkingLevel>() {
                        Ok(level) => level,
                        Err(_) => {
                            self.items.push(Item::System(
                                "invalid level; use: off / low / medium / high / xhigh / max"
                                    .to_string(),
                            ));
                            return;
                        }
                    }
                };
                self.thinking = level;
                let _ = self.cmd_tx.try_send(AgentCmd::SetThinking(level));
                self.items
                    .push(Item::System(format!("thinking -> {}", level.label())));
            }
            "provider" if !arg.is_empty() => {
                let _ = self
                    .cmd_tx
                    .try_send(AgentCmd::SetProvider(arg.to_string()));
                self.items
                    .push(Item::System(format!("Switching to provider {arg}…")));
            }
            "model" if !arg.is_empty() => {
                self.model = arg.to_string();
                let _ = self.cmd_tx.try_send(AgentCmd::SetModel(arg.to_string()));
                self.items
                    .push(Item::System(format!("model -> {arg}")));
            }
            "model" => {
                self.open_model_picker();
            }
            "models" => {
                let _ = self.cmd_tx.try_send(AgentCmd::ListModels);
                self.items.push(Item::System(format!(
                    "Fetching model list for {}…",
                    self.provider
                )));
            }
            "sessions" => {
                self.open_session_picker();
            }
            "session" if !arg.is_empty() => {
                let _ = self
                    .cmd_tx
                    .try_send(AgentCmd::LoadSession(arg.to_string()));
                self.items
                    .push(Item::System(format!("Loading session {arg}…")));
            }
            "session" => {
                self.open_session_picker();
            }
            "undo" => {
                let _ = self.cmd_tx.try_send(AgentCmd::Undo);
                self.items.push(Item::System(
                    "Undoing the last committed edit…".to_string(),
                ));
            }
            "ledger" => {
                let _ = self.cmd_tx.try_send(AgentCmd::Ledger);
                self.items.push(Item::System("Reading the change ledger…".to_string()));
            }
            "pin" if !arg.is_empty() => {
                let _ = self
                    .cmd_tx
                    .try_send(AgentCmd::Pin { path: arg.to_string() });
                self.items
                    .push(Item::System(format!("Pinning {arg}…")));
            }
            "pin" => {
                self.items.push(Item::System(
                    "Usage: /pin <path> (keeps the file's full content during compaction)"
                        .to_string(),
                ));
            }
            "unpin" if !arg.is_empty() => {
                let _ = self
                    .cmd_tx
                    .try_send(AgentCmd::Unpin { path: arg.to_string() });
                self.items
                    .push(Item::System(format!("Unpinning {arg}…")));
            }
            "unpin" => {
                self.items.push(Item::System("Usage: /unpin <path>".to_string()));
            }
            "copy" => self.copy_last_output(),
            "apikey" | "key" if !arg.is_empty() => {
                let (provider, key) = match arg.split_once(char::is_whitespace) {
                    Some((p, k)) => (Some(p.to_string()), k.to_string()),
                    None => (None, arg.to_string()),
                };
                let _ = self.cmd_tx.try_send(AgentCmd::SetApiKey { provider, key });
                self.items
                    .push(Item::System("Saving API key…".to_string()));
            }
            "apikey" | "key" => {
                self.items.push(Item::System(
                    "Usage: /apikey <key> (current provider) or /apikey <provider> <key>; saved \
                     to auth.json so you won't need to configure it again"
                        .to_string(),
                ));
            }
            "add-provider" | "addprovider" => {
                let parts: Vec<&str> = arg.split_whitespace().collect();
                if parts.len() != 4 {
                    self.items.push(Item::System(
                        "Usage: /add-provider <name> <openai|anthropic> <base_url> <model>\n\
                         Example: /add-provider deepseek openai https://api.deepseek.com/v1 \
                         deepseek-v4-flash"
                            .to_string(),
                    ));
                    return;
                }
                let (name, r#type, base_url, model) = (parts[0], parts[1], parts[2], parts[3]);
                let _ = self.cmd_tx.try_send(AgentCmd::AddProvider {
                    name: name.to_string(),
                    r#type: r#type.to_string(),
                    base_url: base_url.to_string(),
                    model: model.to_string(),
                });
                self.items
                    .push(Item::System(format!("Saving provider {name}…")));
            }
            "config" => {
                self.items.push(Item::System(format!(
                    "provider: {} · model: {} · thinking: {} · cwd: {}\nconfig: {}\nauth: {}",
                    self.provider,
                    self.model,
                    self.thinking.label(),
                    self.cwd.display(),
                    self.config_path.display(),
                    firment_core::auth_path().display(),
                )));
            }
            "clear" => {
                self.items.clear();
                self.follow = true;
                self.scroll = 0;
            }
            "quit" | "exit" => self.quit = true,
            other => self
                .items
                .push(Item::System(format!("unknown command: /{other}"))),
        }
    }

    fn render_rows(&self, width: usize) -> Vec<Line<'static>> {
        let mut rows = Vec::new();
        for item in &self.items {
            match item {
                Item::User(text) => {
                    let wrapped = wrap_text(text, width.saturating_sub(2));
                    for (idx, seg) in wrapped.iter().enumerate() {
                        if idx == 0 {
                            rows.push(Line::from(vec![
                                Span::styled(
                                    "❯ ",
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(
                                    seg.clone(),
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ]));
                        } else {
                            rows.push(Line::from(Span::styled(
                                seg.clone(),
                                Style::default().fg(Color::Cyan),
                            )));
                        }
                    }
                    rows.push(Line::from(""));
                }
                Item::Assistant(text) => {
                    for seg in wrap_text(text, width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(
                            seg,
                            Style::default().fg(Color::LightGreen),
                        )));
                    }
                    rows.push(Line::from(""));
                }
                Item::Tool {
                    name,
                    running,
                    ok,
                    summary,
                } => {
                    let (symbol, color) = if *running {
                        const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
                        (
                            SPINNER[(self.frame as usize) % SPINNER.len()],
                            Color::Yellow,
                        )
                    } else if *ok {
                        ('✓', Color::Green)
                    } else {
                        ('✗', Color::Red)
                    };
                    let line = format!("{symbol} {name}  {}", truncate_chars(summary, 140));
                    for seg in wrap_text(&line, width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(seg, Style::default().fg(color))));
                    }
                }
                Item::Permission { tool, reason } => {
                    rows.push(Line::from(Span::styled(
                        "⚠ Permission required",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for seg in wrap_text(&format!("Tool: {tool}"), width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(
                            seg,
                            Style::default().fg(Color::Yellow),
                        )));
                    }
                    for seg in wrap_text(&format!("Reason: {reason}"), width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(
                            seg,
                            Style::default().fg(Color::White),
                        )));
                    }
                    rows.push(Line::from(Span::styled(
                        "[y] allow    [a] always allow for this session    [n] / Esc deny",
                        Style::default().fg(Color::Green),
                    )));
                    rows.push(Line::from(""));
                }
                Item::System(text) => {
                    for seg in wrap_text(text, width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(
                            seg,
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                Item::Error(text) => {
                    for seg in wrap_text(&format!("⚠ {text}"), width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(
                            seg,
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        )));
                    }
                }
            }
        }
        rows
    }

    fn render(&mut self, frame: &mut Frame) {
        self.frame = self.frame.wrapping_add(1);
        let frame_width = frame.area().width.saturating_sub(2) as usize;
        let (input_lines, line_starts, cursor_line, cursor_col) = if self.input.is_empty() {
            (Vec::<String>::new(), Vec::new(), 0, 0)
        } else {
            self.input_layout(frame_width.max(1))
        };
        let input_height = (input_lines.len() + 2).clamp(3, MAX_INPUT_HEIGHT) as u16;
        let [transcript_area, status_area, input_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(input_height),
        ])
        .areas(frame.area());
        self.input_width = frame_width;
        self.input_rect = input_area;

        let content_width = transcript_area.width.saturating_sub(2) as usize;
        self.transcript_rect = transcript_area;
        self.content_width = content_width.max(1);
        let mut rows = self.render_rows(content_width.max(1));
        if self.ai_thinking {
            const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
            let ch = SPINNER[(self.frame as usize) % SPINNER.len()];
            rows.push(Line::from(Span::styled(
                format!(" {ch} thinking…"),
                Style::default().fg(Color::Yellow),
            )));
        }
        let height = transcript_area.height.saturating_sub(2) as usize;
        let max_offset = rows.len().saturating_sub(height);
        self.max_offset = max_offset;
        self.scroll = self.scroll.min(max_offset);
        if self.scroll == 0 {
            self.follow = true;
        }
        let offset = if self.follow {
            max_offset
        } else {
            max_offset.saturating_sub(self.scroll)
        };
        self.highlight_selection(&mut rows);
        let title = if self.follow {
            " Firment ".to_string()
        } else {
            format!(" Firment · ↑ {} ", self.scroll)
        };
        let paragraph = Paragraph::new(rows).scroll((offset as u16, 0)).block(
            Block::bordered()
                .title(Span::styled(title, Style::default().fg(Color::Cyan)))
                .border_style(Style::default().fg(Color::DarkGray)),
        );
        frame.render_widget(paragraph, transcript_area);

        let spinner = if self.busy {
            const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
            SPINNER[(self.frame as usize) % SPINNER.len()].to_string()
        } else {
            "•".to_string()
        };
        let state = self.status_text();
        let mut cwd_str = self.cwd.display().to_string();
        if cwd_str.width() > 36 {
            cwd_str = format!("…{}", truncate_tail(&cwd_str, 35));
        }
        let left = format!(
            " {} {}/{} · T:{} · {}  ",
            self.mode.label().to_uppercase(),
            self.provider,
            self.model,
            self.thinking.label(),
            cwd_str
        );
        let right = if self.busy && !self.interrupting {
            format!(" {} · {state} · Esc interrupt ", spinner)
        } else {
            format!(" {} · {state} ", spinner)
        };
        let pad = (status_area.width as usize).saturating_sub(left.width() + right.width());
        let status_line = Line::from(vec![
            Span::styled(left, Style::default().fg(Color::Cyan)),
            Span::raw(" ".repeat(pad)),
            Span::styled(right, Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(status_line), status_area);

        let visible_text_height = input_area.height.saturating_sub(2) as usize;
        let hidden_lines = input_lines.len().saturating_sub(visible_text_height.max(1));
        let collapsed_lines: usize = self
            .paste_blocks
            .iter()
            .map(|b| b.text.lines().count().max(1))
            .sum();
        let title = if hidden_lines > 0 {
            format!(" input · ↑{hidden_lines} lines hidden (Enter sends everything) ")
        } else if collapsed_lines > 0 {
            format!(" input · {collapsed_lines} lines collapsed (Enter sends full text) ")
        } else {
            " input ".to_string()
        };
        let block = Block::bordered()
            .title(Span::styled(title, Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::DarkGray));
        let content = if self.input.is_empty() {
            self.input_scroll = 0;
            Paragraph::new(Line::from(Span::styled(
                "Type a task, Enter to send · Shift+Enter newline · Esc interrupt/clear · Ctrl+C copy · Ctrl+V paste · Ctrl+P model · Ctrl+Q quit · /help",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block)
        } else {
            let max_scroll = input_lines.len().saturating_sub(visible_text_height.max(1));
            if cursor_line < self.input_scroll {
                self.input_scroll = cursor_line;
            }
            if visible_text_height > 0 && cursor_line >= self.input_scroll + visible_text_height {
                self.input_scroll = cursor_line + 1 - visible_text_height;
            }
            self.input_scroll = self.input_scroll.min(max_scroll);
            let shown = input_lines
                .iter()
                .skip(self.input_scroll)
                .take(visible_text_height.max(1))
                .enumerate()
                .map(|(shown_idx, line)| {
                    let abs_line = self.input_scroll + shown_idx;
                    let line_start = line_starts.get(abs_line).copied().unwrap_or(0);
                    let line_len = line.chars().count();
                    let (sel_min, sel_max) = self
                        .input_sel
                        .map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
                        .unwrap_or((0, 0));
                    let seg_start = sel_min.saturating_sub(line_start);
                    let seg_end = sel_max.saturating_sub(line_start).min(line_len);
                    if seg_start < seg_end {
                        let before: String = line.chars().take(seg_start).collect();
                        let selected: String = line
                            .chars()
                            .skip(seg_start)
                            .take(seg_end - seg_start)
                            .collect();
                        let after: String = line.chars().skip(seg_end).collect();
                        Line::from(vec![
                            Span::styled(before, Style::default().fg(Color::White)),
                            Span::styled(
                                selected,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::REVERSED),
                            ),
                            Span::styled(after, Style::default().fg(Color::White)),
                        ])
                    } else {
                        Line::from(Span::styled(
                            line.clone(),
                            Style::default().fg(Color::White),
                        ))
                    }
                })
                .collect::<Vec<_>>();
            Paragraph::new(shown).block(block)
        };
        frame.render_widget(content, input_area);
        // Permission cards are inline now, so the input always keeps the
        // cursor; even with empty input, pin it to the input start so IME/first
        // chars are not drawn outside the box.
        let modal_open =
            self.model_picker.is_some() || self.session_picker.is_some() || self.question.is_some();
        if !modal_open {
            let cursor_x = input_area.x + 1 + cursor_col as u16;
            let cursor_y = input_area.y + 1 + cursor_line.saturating_sub(self.input_scroll) as u16;
            frame.set_cursor_position((cursor_x, cursor_y));
        }

        if self.permission.is_none() {
            if let Some(picker) = &self.model_picker {
                let area = centered_rect(60, 48, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .title(Span::styled(
                        " Model picker ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Cyan));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                let query: String = picker.query.iter().collect();
                lines.push(Line::from(Span::styled(
                    format!("Filter: {query} (Enter select · Esc close)"),
                    Style::default().fg(Color::DarkGray),
                )));
                if picker.models.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Fetching model list…",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    let filtered = picker.filtered();
                    for (idx, model) in filtered.iter().take(12).enumerate() {
                        let (marker, style) = if idx == picker.selected {
                            (
                                "❯ ",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("  ", Style::default().fg(Color::White))
                        };
                        lines.push(Line::from(Span::styled(format!("{marker}{model}"), style)));
                    }
                    if filtered.len() > 12 {
                        lines.push(Line::from(Span::styled(
                            format!("… {} more", filtered.len() - 12),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }

            if let Some(picker) = &self.session_picker {
                let area = centered_rect(76, 52, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .title(Span::styled(
                        " Session picker ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Magenta));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                let query: String = picker.query.iter().collect();
                lines.push(Line::from(Span::styled(
                    format!("Filter: {query} (↑/↓ select · Enter open · Esc close)"),
                    Style::default().fg(Color::DarkGray),
                )));
                if picker.sessions.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Loading session list…",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    let filtered = picker.filtered();
                    let start = picker.selected.saturating_sub(6);
                    for (idx, session) in filtered.iter().enumerate().skip(start).take(12) {
                        let (marker, style) = if idx == picker.selected {
                            (
                                "❯ ",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("  ", Style::default().fg(Color::White))
                        };
                        let id_short: String = session.id.chars().take(8).collect();
                        let preview = truncate_chars(&session.preview, 42);
                        lines.push(Line::from(Span::styled(
                            format!(
                                "{marker}{}  {:<22}  {}  ({id_short})",
                                format_ts(session.updated_at),
                                session.model,
                                preview
                            ),
                            style,
                        )));
                    }
                    if filtered.len() > 12 {
                        lines.push(Line::from(Span::styled(
                            format!("… {} more", filtered.len() - 12),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                }
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }

            if let Some(question) = &self.question {
                let area = centered_rect(68, 42, frame.area());
                frame.render_widget(Clear, area);
                let block = Block::bordered()
                    .title(Span::styled(
                        " Question ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(Color::Yellow));
                frame.render_widget(block, area);
                let inner = area.inner(Margin {
                    horizontal: 2,
                    vertical: 1,
                });
                let mut lines = Vec::new();
                for line in wrap_text(&question.question, inner.width as usize) {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default().fg(Color::White),
                    )));
                }
                if !question.options.is_empty() {
                    lines.push(Line::default());
                    for (idx, option) in question.options.iter().enumerate() {
                        lines.push(Line::from(Span::styled(
                            format!("  {}  {option}", idx + 1),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }
                lines.push(Line::default());
                let typed: String = self.question_input.iter().collect();
                lines.push(Line::from(vec![
                    Span::styled("Answer: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(typed, Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(Span::styled(
                    "1-9 pick an option · type + Enter free answer · Esc dismiss",
                    Style::default().fg(Color::DarkGray),
                )));
                frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
            }
        }
    }

    /// Short status text shown in the status bar while the agent is running.
    fn status_text(&self) -> String {
        if self.permission.is_some() {
            "waiting for approval".to_string()
        } else if self.question.is_some() {
            "question".to_string()
        } else if self.interrupting {
            "interrupting…".to_string()
        } else if self.ai_thinking {
            "thinking".to_string()
        } else if self.busy {
            if let Some((_, label)) = self.active_tools.last() {
                let count = if self.active_tools.len() > 1 {
                    format!("{}× ", self.active_tools.len())
                } else {
                    String::new()
                };
                format!("working · {count}{label}")
            } else {
                "working".to_string()
            }
        } else {
            "ready".to_string()
        }
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in text.split('\n') {
        if width == 0 {
            out.push(raw_line.to_string());
            continue;
        }
        let mut current = String::new();
        let mut current_w = 0;
        for ch in raw_line.chars() {
            let w = ch.width().unwrap_or(0);
            if current_w + w > width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn truncate_chars(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

fn truncate_tail(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for ch in text.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.insert(0, ch);
        w += cw;
    }
    format!("…{out}")
}

/// Human-readable in-progress hint for a running tool, e.g. "searching" or
/// "flashing target/out.elf…". Falls back to the raw tool name.
fn tool_activity(name: &str, args: &serde_json::Value) -> String {
    let label = match name {
        "grep" | "glob" | "symbols" | "list_dir" => "searching",
        "read_file" => "reading",
        "edit_file" | "write_file" => "editing",
        "build" => "building",
        "flash" => "flashing",
        "run" => "running target",
        "monitor" => "monitoring serial",
        "verify" => "verifying",
        "shell" => "running shell command",
        other => other,
    };
    let target = ["file", "path", "pattern"]
        .iter()
        .find_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .and_then(|s| {
            let base = s.rsplit(['/', '\\']).next().unwrap_or(s);
            if base.is_empty() {
                None
            } else {
                Some(base.to_string())
            }
        });
    match target {
        Some(target) => format!("{label} {target}…"),
        None => format!("{label}…"),
    }
}

fn next_thinking(level: ThinkingLevel) -> ThinkingLevel {
    match level {
        ThinkingLevel::Off => ThinkingLevel::Low,
        ThinkingLevel::Low => ThinkingLevel::Medium,
        ThinkingLevel::Medium => ThinkingLevel::High,
        ThinkingLevel::High => ThinkingLevel::XHigh,
        ThinkingLevel::XHigh => ThinkingLevel::Max,
        ThinkingLevel::Max => ThinkingLevel::Off,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}

fn format_ts(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| secs.to_string())
}

fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// Display width of `text` in terminal cells (CJK chars count as 2).
fn cell_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Character index whose starting cell is at or after `cell` (0-based cells).
fn char_index_at_cell(text: &str, cell: usize) -> usize {
    let mut width = 0usize;
    for (idx, ch) in text.chars().enumerate() {
        if width >= cell {
            return idx;
        }
        width += ch.width().unwrap_or(0);
    }
    text.chars().count()
}

/// Find a subslice in a char slice (starting at `from`); returns the start
/// index.
fn find_subslice(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let from = from.min(haystack.len());
    (from..=haystack.len().saturating_sub(needle.len()))
        .find(|&i| haystack[i..i + needle.len()] == *needle)
}

async fn run_loop(
    terminal: &mut Tui,
    app: &mut App,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    mut perm_rx: mpsc::Receiver<PermissionRequest>,
    mut ask_rx: mpsc::Receiver<QuestionRequest>,
    mut ui_rx: mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    // A 25ms tick lands paste-burst buffers on time without slowing animations.
    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    let mut dirty = true;
    loop {
        let mut spinner_tick = false;
        tokio::select! {
            event = event_rx.recv() => {
                if let Some(event) = event {
                    app.on_agent(event);
                    dirty = true;
                }
            }
            request = perm_rx.recv() => {
                if let Some(request) = request {
                    app.on_permission(request);
                    dirty = true;
                }
            }
            question = ask_rx.recv() => {
                if let Some(question) = question {
                    app.on_question(question);
                    dirty = true;
                }
            }
            ui_event = ui_rx.recv() => {
                if let Some(ui_event) = ui_event {
                    let quit = app.on_ui(ui_event);
                    dirty = true;
                    if quit {
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                spinner_tick = true;
                // Held/buffered chars land here when due (e.g. after a paste
                // stream ends with no further keys).
                if app.apply_burst_outputs() {
                    dirty = true;
                }
            }
        }
        // No animation plays while waiting for approval; stop the 25ms
        // redraws to avoid flicker.
        let animate =
            (app.busy || app.ai_thinking) && app.permission.is_none() && app.question.is_none();
        if dirty || (animate && spinner_tick) {
            terminal.draw(|frame| app.render(frame))?;
            dirty = false;
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        )
    }

    #[test]
    fn input_history_navigates_when_empty_and_scrolls_when_typing() {
        let mut app = test_app();
        app.max_offset = 10;

        app.input = "first".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        app.input = "second".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.history, vec!["first", "second"]);

        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "second");
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "first");
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.input.iter().collect::<String>(), "second");
        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(app.input.is_empty());

        // Non-empty input: Up/Down keep scrolling the transcript.
        app.scroll = 0;
        app.follow = true;
        app.input = "abc".chars().collect();
        app.cursor = app.input.len();
        app.on_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.scroll, 1);
        assert_eq!(app.input.iter().collect::<String>(), "abc");
    }

    #[test]
    fn esc_while_busy_requests_cancel_and_keeps_input() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.busy = true;
        app.input = "draft".chars().collect();
        app.cursor = app.input.len();

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.interrupting);
        assert_eq!(app.input.iter().collect::<String>(), "draft");
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::Cancel => {}
            _ => panic!("expected Cancel"),
        }

        // The interrupt request can only be sent once.
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn esc_when_idle_clears_input() {
        let mut app = test_app();
        app.input = "abc".chars().collect();
        app.cursor = 1;
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn permission_card_is_inline_and_answer_removes_it() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_permission(PermissionRequest {
            tool: "write_file".to_string(),
            reason: "need to write a file".to_string(),
            reply: tx,
        });
        assert!(matches!(
            app.items.last(),
            Some(Item::Permission { tool, .. }) if tool == "write_file"
        ));

        let rows = app.render_rows(80);
        let text: String = rows
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("[y] allow"));

        app.on_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(true));
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { .. }))
        );
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text.starts_with("✓ Allowed")
        ));
    }

    #[test]
    fn permission_esc_denies_inline_card() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_permission(PermissionRequest {
            tool: "shell".to_string(),
            reason: "test".to_string(),
            reply: tx,
        });
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(false));
        assert!(
            !app.items
                .iter()
                .any(|item| matches!(item, Item::Permission { .. }))
        );
    }

    #[test]
    fn question_modal_answers_by_option_key() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "which chip?".to_string(),
            options: vec!["stm32f407".to_string(), "stm32g0".to_string()],
            reply: tx,
        });
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text.contains("which chip?")
        ));
        assert!(app.question.is_some());

        app.on_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(Some("stm32g0".to_string())));
        assert!(app.question.is_none());
    }

    #[test]
    fn question_modal_accepts_free_form_answer_and_esc_dismisses() {
        let mut app = test_app();
        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "which toolchain?".to_string(),
            options: Vec::new(),
            reply: tx,
        });
        for ch in ['g', 'n', 'u', '-', 'r', 'm'] {
            app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.question_input.iter().collect::<String>(), "gnu-rm");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(Some("gnu-rm".to_string())));
        assert!(app.question.is_none());

        let (tx, mut rx) = oneshot::channel();
        app.on_question(QuestionRequest {
            question: "still there?".to_string(),
            options: Vec::new(),
            reply: tx,
        });
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(rx.try_recv(), Ok(None));
        assert!(app.question.is_none());
    }

    #[test]
    fn input_layout_wraps_long_text_and_tracks_cursor() {
        let mut app = test_app();
        // 12 ASCII + 2 CJK chars: width = 12 + 4 = 16, so a 4-column wrap puts
        // 4 chars per line.
        app.input = "abcdefghijkl你好".chars().collect();
        app.cursor = app.input.len();
        let (lines, line_starts, cursor_line, cursor_col) = app.input_layout(4);
        assert_eq!(lines, vec!["abcd", "efgh", "ijkl", "你好"]);
        assert_eq!(line_starts, vec![0, 4, 8, 12]);
        assert_eq!(cursor_line, 3);
        assert_eq!(cursor_col, 4);

        // Cursor in the middle: column counts cell widths.
        app.input = "你abc".chars().collect();
        app.cursor = 3; // CJK char 你(width 2) + a(1) + b(1) → col 4
        let (_, _, cursor_line, cursor_col) = app.input_layout(10);
        assert_eq!(cursor_line, 0);
        assert_eq!(cursor_col, 4);
    }

    #[test]
    fn move_input_cursor_moves_between_wrapped_lines() {
        let mut app = test_app();
        app.input_width = 4;
        app.input = "abcdefghijkl".chars().collect();
        app.cursor = app.input.len(); // lines: abcd / efgh / ijkl, cursor at end of line 2

        app.move_input_cursor(-1);
        assert_eq!(app.cursor, 8); // end of previous line (efgh)
        app.move_input_cursor(-1);
        assert_eq!(app.cursor, 4); // end of the line before that (abcd)
        app.move_input_cursor(1);
        assert_eq!(app.cursor, 8); // back to the end of efgh
    }

    #[test]
    fn paste_collapses_long_text_into_placeholder() {
        let mut app = test_app();
        let text = "a\nb\nc\nd";
        app.insert_text_at_cursor(text, true);
        assert_eq!(app.input.iter().collect::<String>(), "【line 1-4】");
        assert_eq!(app.expand_input(), text);
        assert_eq!(app.cursor, "【line 1-4】".chars().count());

        // Moving left from the placeholder tail skips the whole placeholder;
        // the same applies to the right.
        app.move_cursor_left();
        assert_eq!(app.cursor, 0);
        app.move_cursor_right();
        assert_eq!(app.cursor, "【line 1-4】".chars().count());

        // Backspace at the placeholder tail deletes it as a whole.
        app.backspace();
        assert!(app.input.is_empty());
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_small_text_stays_inline() {
        let mut app = test_app();
        app.insert_text_at_cursor("hello", true);
        assert_eq!(app.input.iter().collect::<String>(), "hello");
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_burst_multi_line_flow_is_collapsed_not_submitted() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        // Simulate Windows Terminal injecting a multi-line paste as a fast
        // keystroke stream (ending with Enter).
        app.on_key_burst(ch('a'), t0);
        app.on_key_burst(ch('b'), t0 + Duration::from_millis(10));
        app.on_key_burst(enter, t0 + Duration::from_millis(20));
        app.on_key_burst(ch('c'), t0 + Duration::from_millis(30));
        app.on_key_burst(enter, t0 + Duration::from_millis(40));
        app.on_key_burst(ch('d'), t0 + Duration::from_millis(50));

        // Before the buffer flushes: nothing is submitted and no partial text
        // appears in the input.
        assert!(app.items.is_empty());
        assert!(app.input.is_empty());

        // After the idle timeout, the whole buffer goes through collapsed
        // paste.
        app.apply_burst_outputs_at(t0 + Duration::from_millis(300));
        assert_eq!(app.input.iter().collect::<String>(), "【line 1-3】");
        assert_eq!(app.expand_input(), "ab\nc\nd");
        assert!(app.items.is_empty());
    }

    #[test]
    fn paste_burst_single_char_enter_does_not_submit() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), t0);
        app.on_key_burst(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            t0 + Duration::from_millis(10),
        );
        assert!(app.items.is_empty());
        assert_eq!(app.input.iter().collect::<String>(), "x\n");
    }

    #[test]
    fn paste_burst_slow_typing_inserts_normally() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);

        app.on_key_burst(ch('a'), t0);
        assert!(app.input.is_empty()); // first ASCII char is held
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(50)));
        assert_eq!(app.input.iter().collect::<String>(), "a");

        app.on_key_burst(ch('b'), t0 + Duration::from_millis(100));
        app.on_key_burst(ch('c'), t0 + Duration::from_millis(200));
        assert_eq!(app.input.iter().collect::<String>(), "ab");
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(250)));
        assert_eq!(app.input.iter().collect::<String>(), "abc");
        assert!(app.paste_blocks.is_empty());
    }

    #[test]
    fn paste_burst_non_ascii_retro_captures_inserted_prefix() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.input = "AB".chars().collect();
        app.cursor = 1;

        app.on_key_burst(KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE), t0);
        assert_eq!(app.input.iter().collect::<String>(), "A你B");

        // A second char arriving quickly reclaims the inserted first char and
        // moves both into the buffer.
        app.on_key_burst(
            KeyEvent::new(KeyCode::Char('好'), KeyModifiers::NONE),
            t0 + Duration::from_millis(10),
        );
        assert_eq!(app.input.iter().collect::<String>(), "AB");

        app.apply_burst_outputs_at(t0 + Duration::from_millis(300));
        assert_eq!(app.input.iter().collect::<String>(), "A你好B");
        assert!(app.items.is_empty());
    }

    #[test]
    fn paste_burst_enter_after_flush_is_newline_within_window() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        let ch = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        app.on_key_burst(ch('a'), t0);
        app.on_key_burst(ch('b'), t0 + Duration::from_millis(10));
        assert!(app.apply_burst_outputs_at(t0 + Duration::from_millis(200)));
        assert_eq!(app.input.iter().collect::<String>(), "ab");

        // Enter within 100ms after flush: treated as a newline, not sent.
        app.on_key_burst(enter, t0 + Duration::from_millis(250));
        assert!(app.items.is_empty());
        assert_eq!(app.input.iter().collect::<String>(), "ab\n");

        // After the protection window, Enter submits normally.
        app.on_key_burst(enter, t0 + Duration::from_millis(500));
        assert!(app.input.is_empty());
        assert!(matches!(
            app.items.last(),
            Some(Item::User(text)) if text == "ab"
        ));
    }

    #[test]
    fn paste_burst_modified_keys_pass_through_and_clear_state() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), t0);
        // Ctrl+P bypasses the burst and flushes the held char.
        let quit = app.on_key_burst(
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            t0 + Duration::from_millis(5),
        );
        assert!(!quit);
        assert!(app.model_picker.is_some());
        assert_eq!(app.input.iter().collect::<String>(), "a");
    }

    #[test]
    fn paste_burst_held_char_is_not_lost_on_other_keys() {
        let mut app = test_app();
        let t0 = Instant::now() - Duration::from_millis(1000);
        app.on_key_burst(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), t0);
        app.on_key_burst(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            t0 + Duration::from_millis(5),
        );
        assert_eq!(app.input.iter().collect::<String>(), "a");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn new_command_clears_input_and_starts_fresh_session() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.input = "old draft".chars().collect();
        app.cursor = app.input.len();
        app.items.push(Item::User("old message".to_string()));
        app.busy = true;

        app.run_command("new");
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
        assert!(app.paste_blocks.is_empty());
        assert!(app.items.iter().any(|i| matches!(
            i,
            Item::System(text) if text == "Starting a new conversation…"
        )));
        assert!(!app.busy);
        assert!(app.pending_new_session);
        // A running turn is cancelled first so the fresh session is processed
        // without waiting for the old turn to finish.
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::Cancel => {}
            _ => panic!("expected Cancel"),
        }
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::NewSession => {}
            _ => panic!("expected NewSession"),
        }
    }

    #[test]
    fn new_command_suppresses_old_turn_events_until_session_loaded() {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.items.push(Item::User("old blue message".to_string()));
        app.items.push(Item::Assistant("old reply".to_string()));

        app.run_command("new");
        // Old-turn events arriving before the fresh session must not leak back
        // into the cleared transcript.
        app.on_agent(AgentEvent::TextDelta("stale delta".to_string()));
        app.on_agent(AgentEvent::Info("stale info".to_string()));
        app.on_agent(AgentEvent::ToolStart {
            name: "read_file".to_string(),
            args: serde_json::json!({}),
        });
        assert_eq!(app.items.len(), 1); // only the "Starting…" hint
        assert!(matches!(
            app.items.last(),
            Some(Item::System(text)) if text == "Starting a new conversation…"
        ));

        // A message typed and sent while the fresh session is loading must
        // survive the transcript clear.
        app.input = "new message".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert!(matches!(app.items.last(), Some(Item::User(text)) if text == "new message"));

        let fresh = Session::new(PathBuf::from("."), "default", "m");
        app.on_agent(AgentEvent::SessionLoaded(fresh));
        assert!(!app.pending_new_session);
        assert_eq!(app.items.len(), 2); // confirmation + the user's new message
        assert!(matches!(
            app.items[0],
            Item::System(ref text) if text == "New conversation started"
        ));
        assert!(matches!(
            app.items[1],
            Item::User(ref text) if text == "new message"
        ));
    }

    #[test]
    fn shift_enter_inserts_newline_instead_of_sending() {
        let mut app = test_app();
        app.input = "abc".chars().collect();
        app.cursor = app.input.len();
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(app.input.iter().collect::<String>(), "abc\n");
        assert!(!app.busy);
        assert!(app.items.is_empty());
    }

    #[test]
    fn input_selection_expands_collapsed_blocks() {
        let mut app = test_app();
        app.input = "A【line 1-2】B".chars().collect();
        app.paste_blocks.push(PasteBlock {
            placeholder: "【line 1-2】".to_string(),
            text: "x\ny".to_string(),
        });
        app.input_sel = Some((0, app.input.len()));
        assert_eq!(app.input_selection_text().unwrap(), "Ax\nyB");
    }

    #[test]
    fn model_picker_filters_and_selects() {
        let mut app = test_app();
        app.on_agent(AgentEvent::Models(vec![
            "deepseek-v4-flash".to_string(),
            "deepseek-v4-pro".to_string(),
            "gpt-xhigh".to_string(),
        ]));
        assert!(app.model_picker.is_some());
        assert_eq!(
            app.model_picker.as_ref().unwrap().selected_model().unwrap(),
            "deepseek-v4-flash"
        );

        app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(
            app.model_picker.as_ref().unwrap().filtered(),
            vec!["gpt-xhigh"]
        );
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.model_picker.is_none());
        assert_eq!(app.model, "gpt-xhigh");
    }

    #[test]
    fn mouse_selection_extracts_rendered_text() {
        let mut app = test_app();
        app.items.push(Item::Assistant("hello world".to_string()));
        app.items.push(Item::Assistant("second line".to_string()));
        app.transcript_rect = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 6,
        };
        app.content_width = 28;
        app.max_offset = 0;
        app.follow = true;

        let across_rows = Selection {
            anchor_row: 0,
            anchor_col: 0,
            row: 2,
            col: 2,
        };
        assert_eq!(app.selection_text(across_rows), "hello world\n\nse");

        let within_row = Selection {
            anchor_row: 0,
            anchor_col: 6,
            row: 0,
            col: 11,
        };
        assert_eq!(app.selection_text(within_row), "world");
    }

    #[test]
    fn selection_uses_cell_widths_for_cjk() {
        let mut app = test_app();
        app.items.push(Item::Assistant("你好世界 ok".to_string()));
        app.transcript_rect = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 5,
        };
        app.content_width = 38;
        app.max_offset = 0;
        app.follow = true;

        // Cells: 你(0-2) 好(2-4) 世(4-6) 界(6-8) space(8) o(9) k(10)
        let first_four = Selection {
            anchor_row: 0,
            anchor_col: 0,
            row: 0,
            col: 8,
        };
        assert_eq!(app.selection_text(first_four), "你好世界");

        let mixed = Selection {
            anchor_row: 0,
            anchor_col: 4,
            row: 0,
            col: 9,
        };
        assert_eq!(app.selection_text(mixed), "世界 ");
    }

    #[test]
    fn session_picker_navigates_and_loads_selected() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let mut app = App::new(
            cmd_tx,
            Arc::new(Mutex::new(HashSet::new())),
            "test-model".to_string(),
            PathBuf::from("."),
            "default".to_string(),
            ThinkingLevel::Off,
            SessionMode::Agent,
            PathBuf::from("config.toml"),
            None,
            Vec::new(),
        );
        app.on_agent(AgentEvent::Sessions(vec![
            firment_core::SessionSummary {
                id: "11111111-aaaa".to_string(),
                updated_at: 1,
                model: "m1".to_string(),
                cwd: PathBuf::from("."),
                preview: "first".to_string(),
            },
            firment_core::SessionSummary {
                id: "22222222-bbbb".to_string(),
                updated_at: 2,
                model: "m2".to_string(),
                cwd: PathBuf::from("."),
                preview: "second".to_string(),
            },
        ]));
        assert!(app.session_picker.is_some());

        app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.session_picker.is_none());
        match cmd_rx.try_recv().unwrap() {
            AgentCmd::LoadSession(id) => assert_eq!(id, "22222222-bbbb"),
            _ => panic!("expected LoadSession"),
        }
    }

    #[test]
    fn last_output_text_returns_most_recent_assistant_message() {
        let mut app = test_app();
        assert!(app.last_output_text().is_none());
        app.items.push(Item::Assistant("first".to_string()));
        app.items.push(Item::System("note".to_string()));
        app.items.push(Item::Assistant("second".to_string()));
        assert_eq!(app.last_output_text().unwrap(), "second");
    }

    #[test]
    fn session_loaded_repopulates_transcript() {
        let mut app = test_app();
        let mut session = Session::new(PathBuf::from("."), "default", "m");
        session.push(ChatMessage::User {
            content: "hello".to_string(),
        });
        session.push(ChatMessage::Assistant {
            content: "reply".to_string(),
            tool_calls: Vec::new(),
        });
        session.push(ChatMessage::Tool {
            tool_call_id: "c1".to_string(),
            name: "read_file".to_string(),
            content: "ok".to_string(),
        });

        app.on_agent(AgentEvent::SessionLoaded(session));
        assert!(matches!(&app.items[0], Item::User(t) if t == "hello"));
        assert!(matches!(&app.items[1], Item::Assistant(t) if t == "reply"));
        assert!(matches!(
            &app.items[2],
            Item::Tool {
                name,
                running: false,
                ok: true,
                ..
            } if name == "read_file"
        ));
    }

    #[test]
    fn plan_mode_toggle_updates_status_through_settings() {
        let mut app = test_app();
        assert_eq!(app.mode, SessionMode::Agent);
        app.on_agent(AgentEvent::Settings {
            provider: None,
            model: None,
            thinking: None,
            mode: Some(SessionMode::Plan),
        });
        assert_eq!(app.mode, SessionMode::Plan);
    }

    #[test]
    fn plan_command_toggles_both_ways_and_agent_is_explicit() {
        let mut app = test_app();
        app.input = "/plan".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/plan".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);

        app.input = "/plan on".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/plan off".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);

        app.input = "/plan on".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Plan);

        app.input = "/agent".chars().collect();
        app.cursor = app.input.len();
        app.submit();
        assert_eq!(app.mode, SessionMode::Agent);
    }

    #[test]
    fn scroll_counts_rows_away_from_bottom_and_clamps_at_top() {
        let mut app = test_app();
        app.max_offset = 80;
        assert!(app.follow);

        app.scroll_up(3);
        assert!(!app.follow);
        assert_eq!(app.scroll, 3);

        app.scroll_up(5);
        assert_eq!(app.scroll, 8);

        // cannot scroll past the top of the transcript
        app.scroll_up(1000);
        assert_eq!(app.scroll, 80);
        assert!(!app.follow);

        app.scroll_down(80);
        assert_eq!(app.scroll, 0);
        assert!(app.follow);

        // nothing to scroll when the whole transcript fits on screen
        app.max_offset = 0;
        app.scroll_up(10);
        assert!(app.follow);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn tool_activity_names_common_tools_and_targets() {
        assert_eq!(
            tool_activity("grep", &serde_json::json!({ "pattern": "fn main" })),
            "searching fn main…"
        );
        assert_eq!(
            tool_activity("glob", &serde_json::json!({ "path": "src/main.rs" })),
            "searching main.rs…"
        );
        assert_eq!(
            tool_activity(
                "flash",
                &serde_json::json!({ "file": "target/thumbv7em/debug/app.elf" })
            ),
            "flashing app.elf…"
        );
        assert_eq!(
            tool_activity("monitor", &serde_json::json!({ "port": "COM3" })),
            "monitoring serial…"
        );
        assert_eq!(
            tool_activity("read_file", &serde_json::json!({ "path": "" })),
            "reading…"
        );
        // Unknown tools fall back to their raw name.
        assert_eq!(tool_activity("deploy", &serde_json::json!({})), "deploy…");
    }

    #[test]
    fn status_tracks_active_tools_with_count() {
        let mut app = test_app();
        assert_eq!(app.status_text(), "ready");

        app.on_agent(AgentEvent::TurnStart);
        assert_eq!(app.status_text(), "thinking");

        app.on_agent(AgentEvent::ToolStart {
            name: "grep".to_string(),
            args: serde_json::json!({ "pattern": "fn main" }),
        });
        assert_eq!(app.status_text(), "working · searching fn main…");

        app.on_agent(AgentEvent::ToolStart {
            name: "flash".to_string(),
            args: serde_json::json!({ "file": "app.elf" }),
        });
        assert_eq!(app.status_text(), "working · 2× flashing app.elf…");

        app.on_agent(AgentEvent::ToolEnd {
            name: "flash".to_string(),
            ok: true,
            summary: String::new(),
        });
        assert_eq!(app.status_text(), "working · searching fn main…");

        app.on_agent(AgentEvent::ToolEnd {
            name: "grep".to_string(),
            ok: true,
            summary: String::new(),
        });
        assert_eq!(app.status_text(), "working");
    }
}
