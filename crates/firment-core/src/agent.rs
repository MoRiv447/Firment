use crate::provider::{Provider, ProviderError, ProviderEvent};
use crate::session::SessionStore;
use crate::tool::{ToolContext, ToolRegistry};
use crate::types::{ChatMessage, ChatRequest, SessionMode, ThinkingLevel, ToolCall};
use crate::{PermissionChecker, Session, system_prompt_for};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;

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

        for _ in 0..self.max_iterations {
            let request = self.build_request();
            let provider = self.provider.as_ref().ok_or(AgentError::NoProvider)?;
            let mut stream = provider.stream(request).await?;
            let mut content = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();

            while let Some(event) = stream.next().await {
                match event? {
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
            };
            for call in &tool_calls {
                self.sink
                    .event(AgentEvent::ToolStart {
                        name: call.name.clone(),
                        args: call.arguments.clone(),
                    })
                    .await;
                let result = self
                    .registry
                    .run(&call.name, call.arguments.clone(), &ctx)
                    .await;
                let (ok, text) = match result {
                    Ok(output) => (true, output.text),
                    Err(e) => (false, e.message),
                };
                let summary = summarize(&text);
                self.sink
                    .event(AgentEvent::ToolEnd {
                        name: call.name.clone(),
                        ok,
                        summary: summary.clone(),
                    })
                    .await;
                self.session.push(ChatMessage::Tool {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    content: text,
                });
            }
        }

        Err(AgentError::MaxIterations(self.max_iterations))
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
