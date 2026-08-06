use async_trait::async_trait;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEventKind, read,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use firment_core::{
    Agent, AgentEvent, Config, EventSink, PermissionChecker, PermissionError, PlanModePermission,
    ProviderConfig, Session, SessionMode, SessionStore, ThinkingLevel,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::HashSet;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub async fn run(
    config: Config,
    config_path: std::path::PathBuf,
    session: Session,
) -> anyhow::Result<()> {
    // The TUI must start even without an API key, so the user can configure
    // it from inside with /apikey or /provider.
    let (provider, startup_hint) =
        match config.build_provider(Some(&session.provider), Some(&session.model)) {
            Ok(provider) => (Some(provider), None),
            Err(e) => (
                None,
                Some(format!(
                    "⚠ {e}（在 TUI 里执行 /apikey sk-xxx 即可配置，无需退出）"
                )),
            ),
        };
    let store = SessionStore::default();
    let default_registry = firment_tools::default_registry();
    let plan_registry = firment_tools::plan_registry();

    let (event_tx, event_rx) = mpsc::channel(256);
    let (perm_tx, perm_rx) = mpsc::channel(16);
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
                    if let Err(e) = agent.run_turn(&text).await {
                        agent.emit(AgentEvent::Error(e.to_string())).await;
                    }
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
                        .emit(AgentEvent::Info(format!("model -> {model}（已保存）")))
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
                            "thinking -> {}（已保存）",
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
                            "mode -> {}（下一条消息起生效）",
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
                                .emit(AgentEvent::Error(format!("获取模型列表失败: {e}")))
                                .await;
                            agent.emit(AgentEvent::Models(Vec::new())).await;
                        }
                    }
                }
                AgentCmd::ListSessions => match store.list() {
                    Ok(sessions) => {
                        if sessions.is_empty() {
                            agent.emit(AgentEvent::Info("暂无会话。".to_string())).await;
                        } else {
                            let mut msg = format!("会话列表（共 {}）:", sessions.len());
                            for summary in sessions.iter().take(20) {
                                let preview = store
                                    .load(&summary.id)
                                    .map(|s| s.title())
                                    .unwrap_or_default();
                                msg.push_str(&format!(
                                    "\n{}  {}  {:<24}  {}  {}",
                                    summary.id,
                                    format_ts(summary.updated_at),
                                    summary.model,
                                    summary.cwd.display(),
                                    preview
                                ));
                            }
                            if sessions.len() > 20 {
                                msg.push_str(&format!("… 还有 {} 个", sessions.len() - 20));
                            }
                            agent.emit(AgentEvent::Info(msg)).await;
                        }
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("列出会话失败: {e}")))
                            .await;
                    }
                },
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
                                        "会话已切换，但重建 provider 失败: {e}"
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
                                "已切换到会话 {}（{} · {} · {}）",
                                loaded.id,
                                loaded.provider,
                                loaded.model,
                                mode.label()
                            )))
                            .await;
                        agent
                            .emit(AgentEvent::Settings {
                                provider: Some(loaded.provider),
                                model: Some(loaded.model),
                                thinking: Some(loaded.thinking),
                                mode: Some(mode),
                            })
                            .await;
                    }
                    Err(e) => {
                        agent
                            .emit(AgentEvent::Error(format!("加载会话失败: {e}")))
                            .await;
                    }
                },
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
                                    "provider -> {name} · model -> {configured_model}（已保存）"
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
                                .emit(AgentEvent::Error(format!("切换 provider 失败: {e}")))
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
                                            "{provider_name} 的 API key 已保存到 {}（无需每次配置）",
                                            firment_core::auth_path().display()
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    agent
                                        .emit(AgentEvent::Error(format!(
                                            "保存后重建 provider 失败: {e}"
                                        )))
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("保存 API key 失败: {e}")))
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
                                "已配置 provider: {}（当前: {}）\n可用模型:",
                                providers.join(", "),
                                provider_name
                            );
                            if models.is_empty() {
                                msg.push_str("\n  （接口未返回模型，可 /model <id> 手动指定）");
                            } else {
                                for model in models {
                                    msg.push_str(&format!("\n  {model}"));
                                }
                            }
                            msg.push_str("\n切换: /model <id> 或 /provider <名字>");
                            agent.emit(AgentEvent::Info(msg)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("获取模型列表失败: {e}")))
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
                                    "provider {name} 已保存，接下来用 /apikey {name} sk-xxx 设置密钥"
                                )))
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("保存 provider 失败: {e}")))
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
    );
    let result = run_loop(&mut terminal, &mut app, event_rx, perm_rx, ui_rx).await;
    restore_terminal(&mut terminal)?;
    agent_task.abort();
    result
}

type Tui = Terminal<CrosstermBackend<Stdout>>;

fn init_terminal() -> anyhow::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Tui) -> anyhow::Result<()> {
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    disable_raw_mode()?;
    Ok(())
}

enum AgentCmd {
    User(String),
    SetModel(String),
    SetThinking(ThinkingLevel),
    SetMode(SessionMode),
    OpenModelPicker,
    ListSessions,
    LoadSession(String),
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

struct App {
    items: Vec<Item>,
    input: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    busy: bool,
    ai_thinking: bool,
    permission: Option<PermissionRequest>,
    scroll: usize,
    max_offset: usize,
    follow: bool,
    quit: bool,
    model: String,
    provider: String,
    thinking: ThinkingLevel,
    mode: SessionMode,
    model_picker: Option<ModelPicker>,
    cwd: PathBuf,
    config_path: PathBuf,
    cmd_tx: mpsc::Sender<AgentCmd>,
    always: Arc<Mutex<HashSet<String>>>,
    frame: u64,
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

enum Item {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        running: bool,
        ok: bool,
        summary: String,
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
    ) -> Self {
        let mut app = Self {
            items: Vec::new(),
            input: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            history_pos: None,
            busy: false,
            ai_thinking: false,
            permission: None,
            scroll: 0,
            max_offset: 0,
            follow: true,
            quit: false,
            model,
            provider,
            thinking,
            mode,
            model_picker: None,
            cwd,
            config_path,
            cmd_tx,
            always,
            frame: 0,
        };
        if let Some(hint) = startup_hint {
            app.items.push(Item::System(hint));
        }
        app
    }

    fn on_agent(&mut self, event: AgentEvent) {
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
                self.items.push(Item::Tool {
                    name,
                    running: true,
                    ok: false,
                    summary: args.to_string(),
                });
            }
            AgentEvent::ToolEnd { name, ok, summary } => {
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
            AgentEvent::Error(message) => {
                self.items.push(Item::Error(message));
                self.busy = false;
                self.ai_thinking = false;
            }
        }
    }

    fn on_permission(&mut self, request: PermissionRequest) {
        self.permission = Some(request);
    }

    fn on_ui(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.on_key(key),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_up(3);
                    false
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_down(3);
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        if self.permission.is_some() {
            return self.on_permission_key(key);
        }
        if self.model_picker.is_some() {
            return self.on_picker_key(key);
        }
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
                true
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_model_picker();
                false
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                false
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.input.len();
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
                self.cursor = self.cursor.saturating_sub(1);
                false
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.input.len());
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                false
            }
            KeyCode::Up => {
                if self.history_pos.is_some() || (self.input.is_empty() && !self.history.is_empty())
                {
                    self.history_up();
                } else {
                    self.scroll_up(1);
                }
                false
            }
            KeyCode::Down => {
                if self.history_pos.is_some() {
                    self.history_down();
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
            KeyCode::Enter => {
                self.submit();
                false
            }
            KeyCode::Esc => {
                self.input.clear();
                self.cursor = 0;
                false
            }
            _ => false,
        }
    }

    fn on_permission_key(&mut self, key: KeyEvent) -> bool {
        let Some(prompt) = self.permission.take() else {
            return false;
        };
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let _ = prompt.reply.send(true);
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.always.lock().unwrap().insert(prompt.tool.clone());
                let _ = prompt.reply.send(true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                let _ = prompt.reply.send(false);
            }
            _ => {
                self.permission = Some(prompt);
            }
        }
        false
    }

    fn insert_char(&mut self, ch: char) {
        self.history_pos = None;
        if self.cursor >= self.input.len() {
            self.input.push(ch);
        } else {
            self.input.insert(self.cursor, ch);
        }
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.history_pos = None;
        self.input.remove(self.cursor - 1);
        self.cursor -= 1;
    }

    fn delete_char(&mut self) {
        if self.cursor < self.input.len() {
            self.history_pos = None;
            self.input.remove(self.cursor);
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
        } else {
            self.history_pos = None;
            self.input.clear();
            self.cursor = 0;
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
                        .push(Item::System(format!("model -> {model}（切换中…）")));
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
        let text: String = self.input.iter().collect();
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
                "命令: /plan [on|off]  /agent  /models  /model <id>  /sessions  /session <id>  /provider <名字>  /add-provider <名字> <openai|anthropic> <base_url> <模型>  /apikey [provider] <key>  /thinking [off|low|medium|high|xhigh|max]  /config  /clear  /help  /quit\n键位: ↑/↓ 空输入时浏览历史，非空时滚动对话 · PgUp/PgDn/滚轮始终滚动 · Ctrl+P 模型选择器 · ←/→ 移动输入光标 · y/n/a 权限确认 · Ctrl-C 退出"
                    .to_string(),
            )),
            "plan" => {
                let mode = match arg {
                    "on" => SessionMode::Plan,
                    "off" => SessionMode::Agent,
                    _ if self.mode == SessionMode::Plan => SessionMode::Agent,
                    _ => SessionMode::Plan,
                };
                self.mode = mode;
                let _ = self.cmd_tx.try_send(AgentCmd::SetMode(mode));
                let queued = if self.busy { "（当前回合结束后生效）" } else { "" };
                self.items.push(Item::System(format!(
                    "mode -> {}{queued}",
                    mode.label()
                )));
            }
            "agent" => {
                self.mode = SessionMode::Agent;
                let _ = self.cmd_tx.try_send(AgentCmd::SetMode(SessionMode::Agent));
                let queued = if self.busy { "（当前回合结束后生效）" } else { "" };
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
                                "无效级别，可用: off / low / medium / high / xhigh / max"
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
                    .push(Item::System(format!("切换到 provider {arg}…")));
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
                    "正在获取 {} 的模型列表…",
                    self.provider
                )));
            }
            "sessions" => {
                let _ = self.cmd_tx.try_send(AgentCmd::ListSessions);
                self.items
                    .push(Item::System("正在列出会话…".to_string()));
            }
            "session" if !arg.is_empty() => {
                let _ = self
                    .cmd_tx
                    .try_send(AgentCmd::LoadSession(arg.to_string()));
                self.items
                    .push(Item::System(format!("正在加载会话 {arg}…")));
            }
            "session" => {
                self.items.push(Item::System(
                    "用法: /session <id>（先 /sessions 查看列表，/session <id> 切换）".to_string(),
                ));
            }
            "apikey" | "key" if !arg.is_empty() => {
                let (provider, key) = match arg.split_once(char::is_whitespace) {
                    Some((p, k)) => (Some(p.to_string()), k.to_string()),
                    None => (None, arg.to_string()),
                };
                let _ = self.cmd_tx.try_send(AgentCmd::SetApiKey { provider, key });
                self.items
                    .push(Item::System("正在保存 API key…".to_string()));
            }
            "apikey" | "key" => {
                self.items.push(Item::System(
                    "用法: /apikey <key>（当前 provider）或 /apikey <provider> <key>；保存后写入 auth.json，之后无需每次配置"
                        .to_string(),
                ));
            }
            "add-provider" | "addprovider" => {
                let parts: Vec<&str> = arg.split_whitespace().collect();
                if parts.len() != 4 {
                    self.items.push(Item::System(
                        "用法: /add-provider <名字> <openai|anthropic> <base_url> <模型>\n示例: /add-provider deepseek openai https://api.deepseek.com/v1 deepseek-v4-flash"
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
                    .push(Item::System(format!("正在保存 provider {name}…")));
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
                        ("◌", Color::Yellow)
                    } else if *ok {
                        ("✓", Color::Green)
                    } else {
                        ("✗", Color::Red)
                    };
                    let line = format!("{symbol} {name}  {}", truncate_chars(summary, 140));
                    for seg in wrap_text(&line, width.saturating_sub(1)) {
                        rows.push(Line::from(Span::styled(seg, Style::default().fg(color))));
                    }
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
        let [transcript_area, status_area, input_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .areas(frame.area());

        let content_width = transcript_area.width.saturating_sub(2) as usize;
        let mut rows = self.render_rows(content_width.max(1));
        if self.ai_thinking {
            const SPINNER: [char; 4] = ['◐', '◓', '◑', '◒'];
            let ch = SPINNER[(self.frame as usize) % SPINNER.len()];
            rows.push(Line::from(Span::styled(
                format!(" {ch} 思考中…"),
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
        let state = if self.permission.is_some() {
            "等待确认"
        } else if self.ai_thinking {
            "思考中"
        } else if self.busy {
            "工作中"
        } else {
            "就绪"
        };
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
        let right = format!(" {} · {state} ", spinner);
        let pad = (status_area.width as usize).saturating_sub(left.width() + right.width());
        let status_line = Line::from(vec![
            Span::styled(left, Style::default().fg(Color::Cyan)),
            Span::raw(" ".repeat(pad)),
            Span::styled(right, Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(status_line), status_area);

        let input_width = input_area.width.saturating_sub(2) as usize;
        let (visible, visible_cursor) = input_window(&self.input, self.cursor, input_width);
        let block = Block::bordered()
            .title(Span::styled(" input ", Style::default().fg(Color::Cyan)))
            .border_style(Style::default().fg(Color::DarkGray));
        let content = if self.input.is_empty() {
            Paragraph::new(Line::from(Span::styled(
                "输入任务，Enter 发送 · /help · ↑/↓ 空输入时浏览历史 · Ctrl+P 模型选择器 · Ctrl-C 退出",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block)
        } else {
            Paragraph::new(Line::from(Span::styled(
                &visible,
                Style::default().fg(Color::White),
            )))
            .block(block)
        };
        frame.render_widget(content, input_area);
        let prefix: usize = visible
            .chars()
            .take(visible_cursor)
            .map(|c| c.width().unwrap_or(0))
            .sum();
        let cursor_x = input_area.x + 1 + prefix as u16;
        frame.set_cursor_position((cursor_x, input_area.y + 1));

        if let Some(prompt) = &self.permission {
            let area = centered_rect(72, 34, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::bordered()
                .title(Span::styled(
                    " 权限确认 ",
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
            let lines = vec![
                Line::from(Span::styled("工具", Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(
                    prompt.tool.clone(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled("原因", Style::default().fg(Color::DarkGray))),
                Line::from(Span::styled(
                    prompt.reason.clone(),
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "[y] 允许    [n] 拒绝    [a] 本次会话总是允许",
                    Style::default().fg(Color::Green),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
        }

        if let Some(picker) = &self.model_picker {
            let area = centered_rect(60, 48, frame.area());
            frame.render_widget(Clear, area);
            let block = Block::bordered()
                .title(Span::styled(
                    " 模型选择 ",
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
                format!("过滤: {query}（Enter 选择 · Esc 关闭）"),
                Style::default().fg(Color::DarkGray),
            )));
            if picker.models.is_empty() {
                lines.push(Line::from(Span::styled(
                    "正在获取模型列表…",
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
                        format!("… 还有 {} 个", filtered.len() - 12),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
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

fn input_window(chars: &[char], cursor: usize, width: usize) -> (String, usize) {
    let cursor = cursor.min(chars.len());
    let total: usize = chars.iter().map(|c| c.width().unwrap_or(0)).sum();
    if width == 0 || total <= width {
        return (chars.iter().collect(), cursor);
    }
    let mut start = cursor;
    let mut used = 0;
    while start > 0 {
        let w = chars[start - 1].width().unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() {
        let w = chars[end].width().unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        end += 1;
    }
    let visible: String = chars[start..end].iter().collect();
    (visible, cursor.saturating_sub(start))
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

async fn run_loop(
    terminal: &mut Tui,
    app: &mut App,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    mut perm_rx: mpsc::Receiver<PermissionRequest>,
    mut ui_rx: mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                if let Some(event) = event {
                    app.on_agent(event);
                }
            }
            request = perm_rx.recv() => {
                if let Some(request) = request {
                    app.on_permission(request);
                }
            }
            ui_event = ui_rx.recv() => {
                if let Some(ui_event) = ui_event
                    && app.on_ui(ui_event)
                {
                    break;
                }
            }
            _ = ticker.tick() => {}
        }
        terminal.draw(|frame| app.render(frame))?;
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
}
