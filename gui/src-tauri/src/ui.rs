use async_trait::async_trait;
use firment_core::AgentEvent;
use firment_core::Asker;
use firment_core::EventSink;
use firment_core::PermissionChecker;
use firment_core::PermissionError;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::events::frontend_event;
use crate::state::{Shared, next_seq};

/// How long a permission dialog may stay unanswered before the tool call is
/// denied. The GUI waves run tools concurrently, so an unrendered dialog must
/// never wedge the whole batch forever.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);
/// Same guard for `ask_user`: the agent waits on a human, but a missing
/// dialog must not stall the turn past this.
const ASK_TIMEOUT: Duration = Duration::from_secs(180);

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
            config.auto_approve.to_vec()
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
        match timeout(PERMISSION_TIMEOUT, rx).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(PermissionError::denied(format!("user denied tool '{tool}'"))),
            Ok(Err(_)) => Err(PermissionError::denied("permission dialog closed")),
            Err(_) => {
                self.shared.perm_waiters.lock().unwrap().remove(&id);
                Err(PermissionError::denied(format!(
                    "permission request for tool '{tool}' timed out after {}s",
                    PERMISSION_TIMEOUT.as_secs()
                )))
            }
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
        match timeout(ASK_TIMEOUT, rx).await {
            Ok(Ok(Some(answer))) => Ok(answer),
            Ok(Ok(None)) => Err("user dismissed the question".to_string()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => {
                self.shared.ask_waiters.lock().unwrap().remove(&id);
                Err(format!("question timed out after {}s", ASK_TIMEOUT.as_secs()))
            }
        }
    }
}