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
        /// Full tool output (a unified diff for edit/write tools), so the card
        /// can render the change instead of a one-line summary. `None` when
        /// the path produced no output at all.
        detail: Option<String>,
        seq: u64,
    },
    /// A self-review of one tool's change finished (plan §4-A). Carries the same `Finding`
    /// shape every review capability uses, so the card can render a badge and a list
    /// without a second vocabulary.
    Review {
        session_id: Option<String>,
        seq: u64,
        findings: Vec<firment_core::review::Finding>,
    },
    TurnEnd {
        session_id: Option<String>,
        text: String,
    },
    /// A long tool reporting a phase (plan §6-10, item 7). The GUI carries it and does not draw
    /// it yet; the field list is the whole event, so the frontend half is a render and not a
    /// protocol change.
    Progress {
        session_id: Option<String>,
        tool: String,
        seq: u64,
        phase: String,
        current: u64,
        total: u64,
        eta_ms: Option<u64>,
    },
    Info {
        session_id: Option<String>,
        message: String,
    },
    /// A nested agent started, and everything the parent's stream carries until
    /// the matching `SubagentEnd` belongs to it.
    ///
    /// The nested agent emits through the parent's sink, so without this pair
    /// its tool calls are stamped with the parent's `session_id` and read as the
    /// main agent's work. The frontend keeps a stack of these and routes
    /// in-between events to the top entry.
    SubagentStart {
        session_id: Option<String>,
        id: String,
        label: String,
        depth: usize,
    },
    /// See [`FrontendEvent::SubagentStart`]. Sent on the error path too, so the
    /// frontend's stack cannot be left unbalanced by a subagent that failed.
    SubagentEnd {
        session_id: Option<String>,
        id: String,
        depth: usize,
    },
    /// One frame from the SBC data plane (device telemetry/state/alert/echo).
    DeviceFrame {
        node: String,
        /// Topic suffix: telemetry | state | alert | echo | ...
        kind: String,
        /// Raw JSON payload as received.
        frame: String,
    },
    /// Guard service status/heartbeat (retained on firment/guard/status),
    /// plus the GUI link's own connected flag.
    GuardStatus {
        frame: String,
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
            detail,
            seq,
        } => FrontendEvent::ToolEnd {
            session_id: sid,
            name: name.clone(),
            ok: *ok,
            summary: summary.clone(),
            detail: detail.clone(),
            seq: *seq,
        },
        AgentEvent::TurnEnd { text } => FrontendEvent::TurnEnd {
            session_id: sid,
            text: text.clone(),
        },
        AgentEvent::Review { seq, findings } => FrontendEvent::Review {
            session_id: sid,
            seq: *seq,
            findings: findings.clone(),
        },
        AgentEvent::Progress { tool, seq, event } => FrontendEvent::Progress {
            session_id: sid,
            tool: tool.clone(),
            seq: *seq,
            phase: event.phase.clone(),
            current: event.current,
            total: event.total,
            eta_ms: event.eta_ms,
        },
        AgentEvent::Info(message) => FrontendEvent::Info {
            session_id: sid,
            message: message.clone(),
        },
        AgentEvent::SubagentStart { id, label, depth } => FrontendEvent::SubagentStart {
            session_id: sid,
            id: id.clone(),
            label: label.clone(),
            depth: *depth,
        },
        AgentEvent::SubagentEnd { id, depth } => FrontendEvent::SubagentEnd {
            session_id: sid,
            id: id.clone(),
            depth: *depth,
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
