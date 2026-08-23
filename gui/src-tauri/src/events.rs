use firment_core::types::ChatMessage;
use firment_core::AgentEvent;
use firment_core::Session;
use firment_core::SessionSummary;
use serde::Serialize;

/// Frontend-facing event DTO. `firment-core` types carry no serde derives
/// (except `ChatMessage`, `ToolCall`, ...), so we map into a small tagged
/// enum before emitting over Tauri's event bus.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendEvent {
    TurnStart,
    TextDelta {
        text: String,
    },
    ToolStart {
        name: String,
        args: serde_json::Value,
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
    Info {
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

pub fn frontend_event(e: &AgentEvent) -> FrontendEvent {
    match e {
        AgentEvent::TurnStart => FrontendEvent::TurnStart,
        AgentEvent::TextDelta(text) => FrontendEvent::TextDelta { text: text.clone() },
        AgentEvent::ToolStart { name, args, seq } => FrontendEvent::ToolStart {
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
            name: name.clone(),
            ok: *ok,
            summary: summary.clone(),
            seq: *seq,
        },
        AgentEvent::TurnEnd { text } => FrontendEvent::TurnEnd { text: text.clone() },
        AgentEvent::Info(message) => FrontendEvent::Info {
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
            message: message.clone(),
        },
    }
}