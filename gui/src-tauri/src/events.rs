use firment_core::types::ChatMessage;
use firment_core::AgentEvent;
use firment_core::Session;
use firment_core::SessionSummary;
use serde::Serialize;

/// Frontend-facing event DTO. `firment-core` types carry no serde derives
/// (except `ChatMessage`, `ToolCall`, ...), so we map into a small tagged
/// enum before emitting over Tauri's event bus.
///
/// Turn-flow variants carry `session_id` so the frontend can route them to
/// the right chat when several sessions run turns in parallel. `None` means
/// the event is app-global (settings, session list, ...) or came from a
/// context without a session.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendEvent {
    TurnStart {
        session_id: Option<String>,
    },
    TextDelta {
        session_id: Option<String>,
        text: String,
    },
    /// Live extended-thinking snippet from the provider.
    Thinking {
        session_id: Option<String>,
        text: String,
    },
    ToolStart {
        session_id: Option<String>,
        name: String,
        args: serde_json::Value,
        seq: u64,
    },
    ToolEnd {
        session_id: Option<String>,
        name: String,
        ok: bool,
        summary: String,
        seq: u64,
    },
    TurnEnd {
        session_id: Option<String>,
        text: String,
    },
    Info {
        session_id: Option<String>,
        message: String,
    },
    Settings {
        provider: Option<String>,
        model: Option<String>,
        thinking: Option<String>,
        mode: Option<String>,
    },
    Models {
        models: Vec<String>,
    },
    Sessions {
        sessions: Vec<SessionSummaryDto>,
    },
    SessionLoaded {
        session: SessionDto,
    },
    Error {
        session_id: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    pub id: String,
    pub cwd: String,
    pub provider: String,
    pub model: String,
    pub mode: String,
    pub thinking: String,
    /// Per-session compaction budget in chars; 0 = agent default.
    pub context_budget_chars: usize,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummaryDto {
    pub id: String,
    pub updated_at: u64,
    pub model: String,
    pub cwd: String,
    pub preview: String,
    /// Workbench tree linkage ("main" | "branch").
    pub kind: String,
    pub parent_session: Option<String>,
}

pub fn session_dto(s: &Session) -> SessionDto {
    SessionDto {
        id: s.id.clone(),
        cwd: s.cwd.to_string_lossy().into_owned(),
        provider: s.provider.clone(),
        model: s.model.clone(),
        mode: s.mode.label().to_string(),
        thinking: s.thinking.label().to_string(),
        context_budget_chars: s.context_budget_chars,
        created_at: s.created_at,
        updated_at: s.updated_at,
        messages: s.messages.clone(),
    }
}

pub fn session_summary_dto(s: &SessionSummary) -> SessionSummaryDto {
    SessionSummaryDto {
        id: s.id.clone(),
        updated_at: s.updated_at,
        model: s.model.clone(),
        cwd: s.cwd.to_string_lossy().into_owned(),
        preview: s.preview.clone(),
        kind: s.kind.label().to_string(),
        parent_session: s.parent_session.clone(),
    }
}

pub fn frontend_event(e: &AgentEvent, session_id: Option<&str>) -> FrontendEvent {
    let sid = session_id.map(|s| s.to_string());
    match e {
        AgentEvent::TurnStart => FrontendEvent::TurnStart { session_id: sid },
        AgentEvent::TextDelta(text) => FrontendEvent::TextDelta {
            session_id: sid,
            text: text.clone(),
        },
        AgentEvent::Thinking(text) => FrontendEvent::Thinking {
            session_id: sid,
            text: text.clone(),
        },
        AgentEvent::ToolStart { name, args, seq } => FrontendEvent::ToolStart {
            session_id: sid,
            name: name.clone(),
            args: args.clone(),
            seq: *seq,
        },
        AgentEvent::ToolEnd {
            name,
            ok,
            summary,
            seq,
        } => FrontendEvent::ToolEnd {
            session_id: sid,
            name: name.clone(),
            ok: *ok,
            summary: summary.clone(),
            seq: *seq,
        },
        AgentEvent::TurnEnd { text } => FrontendEvent::TurnEnd {
            session_id: sid,
            text: text.clone(),
        },
        AgentEvent::Info(message) => FrontendEvent::Info {
            session_id: sid,
            message: message.clone(),
        },
        AgentEvent::Settings {
            provider,
            model,
            thinking,
            mode,
        } => FrontendEvent::Settings {
            provider: provider.clone(),
            model: model.clone(),
            thinking: thinking.map(|t| t.label().to_string()),
            mode: mode.map(|m| m.label().to_string()),
        },
        AgentEvent::Models(models) => FrontendEvent::Models {
            models: models.clone(),
        },
        AgentEvent::Sessions(sessions) => FrontendEvent::Sessions {
            sessions: sessions.iter().map(session_summary_dto).collect(),
        },
        AgentEvent::SessionLoaded(session) => FrontendEvent::SessionLoaded {
            session: session_dto(session),
        },
        AgentEvent::Error(message) => FrontendEvent::Error {
            session_id: sid,
            message: message.clone(),
        },
    }
}
