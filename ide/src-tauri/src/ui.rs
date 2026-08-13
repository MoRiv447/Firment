use async_trait::async_trait;
use firment_core::AgentEvent;
use firment_core::Asker;
use firment_core::EventSink;
use firment_core::PermissionChecker;
use firment_core::PermissionError;
use serde_json::json;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::oneshot;

use crate::events::frontend_event;
use crate::state::{Shared, next_seq};

/// Forwards agent events onto the Tauri event bus ("agent-event") and the
/// collaboration bus. Mirrors the TUI's `ChannelSink`, exchanging the mpsc
/// channel for `AppHandle::emit`.
pub struct GuiSink {
    pub shared: Arc<Shared>,
}

#[async_trait]
impl EventSink for GuiSink {
    async fn event(&self, event: AgentEvent) {
        let fe = frontend_event(&event);
        let _ = self.shared.app.emit("agent-event", fe);
    }
}

/// Permission gate that surfaces each request as a modal in the frontend.
/// The frontend replies via `respond_permission(id, allowed)`.
pub struct GuiPermission {
    pub shared: Arc<Shared>,
}

#[async_trait]
impl PermissionChecker for GuiPermission {
    async fn confirm(
        &self,
        tool: &str,
        args: &serde_json::Value,
        reason: &str,
    ) -> Result<(), PermissionError> {
        let always = {
            let config = self.shared.config.lock().unwrap();
            config.auto_approve.iter().cloned().collect::<Vec<_>>()
        };
        if always.iter().any(|t| t == tool) {
            return Ok(());
        }
        let id = next_seq();
        let (tx, rx) = oneshot::channel();
        self.shared.perm_waiters.lock().unwrap().insert(id, tx);
        let _ = self.shared.app.emit(
            "permission-request",
            json!({ "id": id, "tool": tool, "args": args, "reason": reason }),
        );
        match rx.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(PermissionError::denied(format!("user denied tool '{tool}'"))),
            Err(_) => Err(PermissionError::denied("permission dialog closed")),
        }
    }
}

/// Question asker for the `ask_user` tool. Frontend replies via
/// `respond_ask(id, answer)`.
pub struct GuiAsker {
    pub shared: Arc<Shared>,
}

#[async_trait]
impl Asker for GuiAsker {
    async fn ask(&self, question: &str, options: &[String]) -> Result<String, String> {
        let id = next_seq();
        let (tx, rx) = oneshot::channel();
        self.shared.ask_waiters.lock().unwrap().insert(id, tx);
        let _ = self.shared
            .app
            .emit("ask-request", json!({ "id": id, "question": question, "options": options }));
        match rx.await {
            Ok(Some(answer)) => Ok(answer),
            Ok(None) => Err("user dismissed the question".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }
}