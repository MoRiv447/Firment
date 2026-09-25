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
use crate::state::{next_seq, Shared};

/// How long a permission dialog may stay unanswered before the tool call is
/// denied. The GUI waves run tools concurrently, so an unrendered dialog must
/// never wedge the whole batch forever.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);
/// Same guard for `ask_user`: the agent waits on a human, but a missing
/// dialog must not stall the turn past this.
const ASK_TIMEOUT: Duration = Duration::from_secs(180);

/// Runs a cleanup on the way out of an awaited dialog — including the way that is not a return.
///
/// A cancelled turn DROPS the tool's future mid-`.await`. The permission or question was never
/// answered and never would be, yet its sender stayed in the map and the dialog stayed on screen,
/// where a later click answers a request nothing is waiting for. Cleaning up on the timeout arm
/// alone covered the least likely way to leave a dialog behind.
struct WaiterGuard {
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl WaiterGuard {
    fn with(cleanup: impl FnOnce() + Send + 'static) -> Self {
        WaiterGuard {
            cleanup: Some(Box::new(cleanup)),
        }
    }

    /// The dialog was answered: there is nothing to clean, and emitting "expired" would dismiss
    /// a dialog the user has already decided.
    fn disarm(&mut self) {
        self.cleanup = None;
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

/// Forwards agent events onto the Tauri event bus ("agent-event") and the
/// collaboration bus. Mirrors the TUI's `ChannelSink`, exchanging the mpsc
/// channel for `AppHandle::emit`. Each sink is bound to one session so
/// turn-flow events can be routed to the right chat in parallel mode.
pub struct GuiSink {
    pub shared: Arc<Shared>,
    pub session_id: String,
}

#[async_trait]
impl EventSink for GuiSink {
    async fn event(&self, event: AgentEvent) {
        let fe = frontend_event(&event, Some(&self.session_id));
        let _ = self.shared.app.emit("agent-event", fe);
    }
}

/// Permission gate that surfaces each request as a modal in the frontend.
/// The frontend replies via `respond_permission(id, allowed)`.
pub struct GuiPermission {
    pub shared: Arc<Shared>,
    /// Shown on the dialog so the user knows WHICH chat is asking.
    pub session_id: String,
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
            let config = self
                .shared
                .config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            config.auto_approve.to_vec()
        };
        if always.iter().any(|t| t == tool) {
            return Ok(());
        }
        let id = next_seq();
        let (tx, rx) = oneshot::channel();
        self.shared
            .perm_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, tx);
        let _ = self.shared.app.emit(
            "permission-request",
            json!({ "id": id, "tool": tool, "args": args, "reason": reason, "session_id": self.session_id }),
        );
        // Registered before the await: every way out of it has to leave the same state behind,
        // and being cancelled is not a return.
        let waiters = self.shared.perm_waiters.clone();
        let app = self.shared.app.clone();
        let mut guard = WaiterGuard::with(move || {
            waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id);
            // Tell the frontend to drop the stale dialog: a later Allow click must not
            // pretend it approved anything.
            let _ = app.emit("permission-expired", json!({ "id": id }));
        });
        match timeout(PERMISSION_TIMEOUT, rx).await {
            Ok(Ok(true)) => {
                guard.disarm();
                Ok(())
            }
            Ok(Ok(false)) => {
                guard.disarm();
                Err(PermissionError::denied(format!(
                    "user denied tool '{tool}'"
                )))
            }
            // The sender went away without answering — a dialog still open is a dialog the
            // guard should close.
            Ok(Err(_)) => Err(PermissionError::denied("permission dialog closed")),
            Err(_) => Err(PermissionError::denied(format!(
                "permission request for tool '{tool}' timed out after {}s",
                PERMISSION_TIMEOUT.as_secs()
            ))),
        }
    }
}

/// Question asker for the `ask_user` tool. Frontend replies via
/// `respond_ask(id, answer)`.
pub struct GuiAsker {
    pub shared: Arc<Shared>,
    /// Shown on the dialog so the user knows WHICH chat is asking.
    pub session_id: String,
}

#[async_trait]
impl Asker for GuiAsker {
    async fn ask(&self, question: &str, options: &[String]) -> Result<String, String> {
        let id = next_seq();
        let (tx, rx) = oneshot::channel();
        self.shared
            .ask_waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, tx);
        let _ = self.shared.app.emit(
            "ask-request",
            json!({ "id": id, "question": question, "options": options, "session_id": self.session_id }),
        );
        let waiters = self.shared.ask_waiters.clone();
        let app = self.shared.app.clone();
        let mut guard = WaiterGuard::with(move || {
            waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id);
            let _ = app.emit("ask-expired", json!({ "id": id }));
        });
        match timeout(ASK_TIMEOUT, rx).await {
            Ok(Ok(Some(answer))) => {
                guard.disarm();
                Ok(answer)
            }
            Ok(Ok(None)) => {
                guard.disarm();
                Err("user dismissed the question".to_string())
            }
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(format!(
                "question timed out after {}s",
                ASK_TIMEOUT.as_secs()
            )),
        }
    }
}
