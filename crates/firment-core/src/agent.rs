use crate::config::CompactionStrategy;
use crate::journal::{EditJournal, Ledger};
use crate::provider::{Provider, ProviderError, ProviderEvent};
use crate::session::{SessionStore, SessionSummary};
use crate::tool::{ToolContext, ToolRegistry};
use crate::types::{ChatMessage, ChatRequest, SessionMode, ThinkingLevel, ToolCall};
use crate::{PermissionChecker, Session, system_prompt_for};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
            max_iterations,
            allow_dangerous: false,
            verify_command: None,
            context_budget_chars: 60_000,
            ledger_seq_appended: 0,
            compaction_strategy: CompactionStrategy::default(),
            symbols_backend: None,
            build_command: None,
            default_chip: None,
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
        let mut message = format!("已固定 {}（压缩时保留全文）", path.display());
        if let Ok(meta) = fs::metadata(&path) {
            let budget = self.context_budget_chars.max(1);
            if meta.len() as usize >= budget * 30 / 100 {
                message.push_str(&format!(
                    "\n⚠ 文件约 {} KB，已达上下文预算的 30% 以上；固定后可能挤占摘要空间，建议只固定关键源码文件",
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
            Ok(format!("{} 不在固定列表", path.display()))
        } else {
            Ok(format!("已取消固定 {}", path.display()))
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
                    "[输出过长（{} 字符），完整内容已外溢到 {}；需要时用 read_file 查看]\n{}",
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
        let (delta, last_seq) = Ledger::new(self.store.ledger_path(&self.session.id))
            .delta_text(self.ledger_seq_appended, 5);
        let input = if delta.is_empty() {
            input.to_string()
        } else {
            self.ledger_seq_appended = last_seq;
            format!("[最近改动台账]\n{delta}\n\n{input}")
        };
        self.session.push(ChatMessage::User { content: input });
        self.sink.event(AgentEvent::TurnStart).await;

        let journal = Arc::new(Mutex::new(EditJournal::new(
            self.store.undo_dir(&self.session.id),
        )));
        let ledger = Ledger::new(self.store.ledger_path(&self.session.id));
        let mut mutations_since_verify = 0usize;

        for _ in 0..self.max_iterations {
            self.compact_if_needed().await;
            let request = self.build_request();
            let provider = self.provider.as_ref().ok_or(AgentError::NoProvider)?;
            let mut stream = provider.stream(request).await?;
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = stream.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(e) => {
                        let summary = rollback_journal(&journal);
                        self.sink
                            .event(AgentEvent::Error(format!(
                                "provider error; 已回滚本回合编辑: {summary}"
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

            self.session.push(ChatMessage::Assistant {
                content: content.clone(),
                tool_calls: tool_calls.clone(),
            });

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
            };

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
                    } else {
                        self.sink
                            .event(AgentEvent::Info(
                                "verify 硬门未通过：修复后重试，跑通前不会标记完成".to_string(),
                            ))
                            .await;
                        continue;
                    }
                } else {
                    self.session.push(plain_assistant);
                }

                let commit_result = lock_journal(&journal).commit();
                match commit_result {
                    Ok(changes) if !changes.is_empty() => {
                        if let Err(e) = ledger.append(&changes) {
                            self.sink
                                .event(AgentEvent::Info(format!("改动台账写入失败: {e}")))
                                .await;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        self.sink
                            .event(AgentEvent::Info(format!("编辑日志写入失败: {e}")))
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
            if stats.verify_passed {
                mutations_since_verify = 0;
            }
        }

        let summary = rollback_journal(&journal);
        self.sink
            .event(AgentEvent::Info(format!(
                "达到最大迭代次数，已回滚本回合编辑: {summary}"
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
        let mut content = format!("[对话已压缩] 摘要：\n{summary}");
        if drop_until > 0 {
            content.push_str("\n\n（更早的对话已按 drop 策略直接丢弃，不再保留摘要）");
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
            Some(format!("[已固定文件（压缩时保留全文）]\n{out}"))
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
            Some(format!("[压缩后回填最近读取的文件]\n{out}"))
        }
    }

    /// Restore the most recently committed edit batch for this session.
    pub async fn undo_last(&mut self) -> Result<String, String> {
        let dir = self.store.undo_dir(&self.session.id);
        let summary = EditJournal::undo_latest(&dir)?;
        Ok(format!(
            "已恢复 {} 个文件: {}",
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
                .event(AgentEvent::Info(format!("编辑批次失败，已回滚: {summary}")))
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
                        "[文件未变化（内容同之前的 read_file 结果）：{}；如需最新内容请重新 read_file]",
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
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
                parts.push(format!("用户: {}", truncate_chars(content, 120)));
            }
            ChatMessage::Assistant { content, .. } => {
                parts.push(format!("助手: {}", truncate_chars(content, 120)));
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
        Ok(files) if files.is_empty() => "没有已记录的文件改动".to_string(),
        Ok(files) => format!("已恢复 {} 个文件: {}", files.len(), files.join(", ")),
        Err(e) => format!("回滚不完整: {e}"),
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

fn thinking_opt(level: ThinkingLevel) -> Option<ThinkingLevel> {
    if level == ThinkingLevel::Off {
        None
    } else {
        Some(level)
    }
}
