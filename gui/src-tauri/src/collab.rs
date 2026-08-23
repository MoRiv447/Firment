use serde::Serialize;
use std::sync::mpsc;

/// Collaboration event: anything another team member should see about this
/// session or this working tree. Injected from remote backends via
/// `CollabBackend::subscribe`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)] // reserved for the M4 collaboration panel
pub enum CollabEvent {
    Agent {
        session_id: String,
        event: serde_json::Value,
    },
    FileChange {
        path: String,
        summary: String,
        undo_ref: String,
    },
    Presence {
        user: String,
        session_id: String,
        status: String,
    },
}

/// Transport-agnostic collaboration backend. The GUI only talks to this
/// trait; M4 can swap in a Git-backed or live-relay implementation without
/// touching the UI layer.
#[allow(dead_code)] // reserved for the M4 collaboration panel
pub trait CollabBackend: Send + Sync {
    fn workspace_id(&self) -> String;
    fn publish(&self, ev: CollabEvent);
    fn subscribe(&self) -> mpsc::Receiver<CollabEvent>;
}

/// Single-user default: publish is a no-op, subscribe yields nothing.
pub struct NoopBackend;

impl CollabBackend for NoopBackend {
    fn workspace_id(&self) -> String {
        "local".to_string()
    }
    fn publish(&self, _ev: CollabEvent) {}
    fn subscribe(&self) -> mpsc::Receiver<CollabEvent> {
        let (_tx, rx) = mpsc::channel();
        rx
    }
}
