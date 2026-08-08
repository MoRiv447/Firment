use crate::journal::EditJournal;
use crate::provider::{Provider, ProviderError, ProviderEvent};
use crate::session::{SessionStore, SessionSummary};
use crate::tool::{ToolContext, ToolRegistry};
use crate::types::{ChatMessage, ChatRequest, SessionMode, ThinkingLevel, ToolCall};
use crate::{PermissionChecker, Session, system_prompt_for};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
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
        self.session.push(ChatMessage::User {
            content: input.to_string(),
        });
        self.sink.event(AgentEvent::TurnStart).await;

        let journal = Arc::new(Mutex::new(EditJournal::new(
            self.store.undo_dir(&self.session.id),
        )));

        for _ in 0..self.max_iterations {
            self.compact_if_needed();
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

            if tool_calls.is_empty() {
                let commit_result = lock_journal(&journal).commit();
                if let Err(e) = commit_result {
                    self.sink
                        .event(AgentEvent::Info(format!("编辑日志写入失败: {e}")))
                        .await;
                }
                self.sink
                    .event(AgentEvent::TurnEnd {
                        text: content.clone(),
                    })
                    .await;
                self.store.save(&self.session)?;
                return Ok(content);
            }

            let ctx = ToolContext {
                cwd: self.session.cwd.clone(),
                permission: self.permission.clone(),
                allow_dangerous: self.allow_dangerous,
                journal: journal.clone(),
                verify_command: self.verify_command.clone(),
            };
            execute_tool_calls(self, &tool_calls, &ctx, &journal).await;
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
    fn compact_if_needed(&mut self) {
        const KEEP_LAST: usize = 10;
        const DIGEST_CHARS: usize = 6000;
        let total: usize = self
            .session
            .messages
            .iter()
            .map(|m| message_text(m).chars().count())
            .sum();
        if total <= self.context_budget_chars || self.session.messages.len() <= KEEP_LAST {
            return;
        }
        let cut = self.session.messages.len() - KEEP_LAST;
        let old = self.session.messages.drain(..cut).collect::<Vec<_>>();
        let digest = compact_summary(&old, DIGEST_CHARS);
        let mut messages = vec![ChatMessage::User {
            content: format!("[早期对话已压缩] 摘要：\n{digest}"),
        }];
        messages.extend(self.session.messages.iter().cloned());
        self.session.messages = messages;
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

/// Execute the turn's tool calls in dependency waves: independent calls run
/// concurrently; a failed mutation rolls the whole turn's edit batch back.
async fn execute_tool_calls(
    agent: &mut Agent,
    tool_calls: &[ToolCall],
    ctx: &ToolContext,
    journal: &Arc<Mutex<EditJournal>>,
) {
    let n = tool_calls.len();
    let deps = tool_call_dependencies(tool_calls);
    let mut done = vec![false; n];
    let mut remaining = n;
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
            let summary = summarize(&text);
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
                content: text,
            });
            done[i] = true;
            remaining -= 1;
        }
    }
}

fn lock_journal(journal: &Arc<Mutex<EditJournal>>) -> std::sync::MutexGuard<'_, EditJournal> {
    journal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
