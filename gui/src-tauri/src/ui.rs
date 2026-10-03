use async_trait::async_trait;
use firment_core::AgentEvent;
use firment_core::Approval;
use firment_core::Asker;
use firment_core::EventSink;
use firment_core::PermissionChecker;
use firment_core::PermissionError;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::events::{frontend_event, FrontendEvent};
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
    /// Streamed text waiting to go out — see [`GuiSink::push`].
    pending: Arc<std::sync::Mutex<Pending>>,
}

/// How much streamed text to hold before sending it, and for how long.
///
/// One token was one event: a paragraph cost eighty `emit` calls, eighty serialisations across
/// the IPC boundary, and — because each one arrives in the webview as its own task, so React
/// cannot batch them — eighty renders for text the user reads as one flowing line.
const COALESCE_BYTES: usize = 1024;
/// The other bound, so a slow model is not held back by a batch it never fills.
const COALESCE_WINDOW: Duration = Duration::from_millis(40);

#[derive(Default)]
struct Pending {
    /// `(kind, text)` in arrival order, consecutive same-kind parts merged. One list rather
    /// than one buffer per kind: thinking and answer can interleave, and two buffers would
    /// reorder them.
    parts: Vec<(&'static str, String)>,
    bytes: usize,
    /// When the first held part arrived. `None` means nothing is buffered, which is also what
    /// keeps a long idle gap from making the next single token wait.
    since: Option<Instant>,
}

impl Pending {
    /// Take one streamed part, merging it into the run it continues.
    fn hold(&mut self, kind: &'static str, text: &str) {
        self.bytes += text.len();
        self.since.get_or_insert_with(Instant::now);
        match self.parts.last_mut() {
            Some((previous, buf)) if *previous == kind => buf.push_str(text),
            _ => self.parts.push((kind, text.to_string())),
        }
    }

    /// Whether what is held should go out now.
    ///
    /// Both bounds are read off the stream itself, so no timer task is needed: a burst reaches
    /// the byte cap, a trickle passes the window on its next arrival, and the next structural
    /// event flushes whatever is left *before* it. Which also means a held tail can only sit
    /// until something else happens — the reason `Drop` flushes too.
    fn due_to_send(&self, now: Instant) -> bool {
        self.bytes >= COALESCE_BYTES
            || self
                .since
                .is_some_and(|since| now.duration_since(since) >= COALESCE_WINDOW)
    }
}

impl GuiSink {
    pub fn new(shared: Arc<Shared>, session_id: String) -> Self {
        Self {
            shared,
            session_id,
            pending: Arc::new(std::sync::Mutex::new(Pending::default())),
        }
    }

    /// Hold a streamed part, and send what is held once it is worth a message.
    ///
    /// Both triggers are driven by the stream itself, so no timer task is needed: a burst of
    /// tokens reaches the byte cap, a trickle passes the window on its next arrival, and the
    /// turn's next structural event (`tool_start`, `turn_end`, …) flushes whatever is left
    /// *before* it, so the order the user sees is the order the model wrote.
    fn push(&self, kind: &'static str, text: &str) {
        // Never held across an `await`: `std::sync::Mutex` is deliberate.
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.hold(kind, text);
        let due = pending.due_to_send(Instant::now());
        drop(pending);
        if due {
            self.flush();
        }
    }

    /// Forward everything held, as the events they would have been.
    fn flush(&self) {
        let held = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *pending)
        }; // the guard is gone before anything is emitted
        for (kind, text) in held.parts {
            let session_id = Some(self.session_id.clone());
            let fe = match kind {
                "thinking" => FrontendEvent::Thinking { session_id, text },
                _ => FrontendEvent::TextDelta { session_id, text },
            };
            let _ = self.shared.app.emit("agent-event", fe);
        }
    }
}

impl Drop for GuiSink {
    fn drop(&mut self) {
        // A turn abandoned mid-stream — an aborted task, a panic — sends no further event to
        // flush through, and the tokens it did stream would die in the buffer.
        self.flush();
    }
}

#[async_trait]
impl EventSink for GuiSink {
    async fn event(&self, event: AgentEvent) {
        match &event {
            AgentEvent::TextDelta(text) => self.push("text_delta", text),
            AgentEvent::Thinking(text) => self.push("thinking", text),
            other => {
                self.flush();
                if let Some(fe) = frontend_event(other, Some(&self.session_id)) {
                    let _ = self.shared.app.emit("agent-event", fe);
                }
            }
        }
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
    async fn confirm(&self, tool: &str, args: &serde_json::Value, reason: &str) -> Approval {
        let always = {
            let config = self
                .shared
                .config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            config.auto_approve.to_vec()
        };
        if always.iter().any(|t| t == tool) {
            // A rule answered this one, so no person spent any time on it.
            return Approval::auto(Ok(()));
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
        // The person's clock starts here, with the dialog on screen — not at the top of
        // this function, which the auto-approve check above reached without asking anyone.
        let asked = Instant::now();
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
        let answered = timeout(PERMISSION_TIMEOUT, rx).await;
        let waited = asked.elapsed();
        match answered {
            Ok(Ok(true)) => {
                guard.disarm();
                Approval::human(waited, Ok(()))
            }
            Ok(Ok(false)) => {
                guard.disarm();
                Approval::human(
                    waited,
                    Err(PermissionError::denied(format!(
                        "user denied tool '{tool}'"
                    ))),
                )
            }
            // The sender went away without answering — a dialog still open is a dialog the
            // guard should close.
            Ok(Err(_)) => Approval::human(
                waited,
                Err(PermissionError::denied("permission dialog closed")),
            ),
            // Nobody answered within the window. The reading time was still theirs, which is
            // why this reports a wait rather than nothing: the card it belongs to spent the
            // whole window on screen.
            Err(_) => Approval::human(
                waited,
                Err(PermissionError::denied(format!(
                    "permission request for tool '{tool}' timed out after {}s",
                    PERMISSION_TIMEOUT.as_secs()
                ))),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_text_keeps_its_runs_and_their_order() {
        let mut pending = Pending::default();
        pending.hold("text_delta", "Hel");
        pending.hold("text_delta", "lo ");
        pending.hold("thinking", "because");
        pending.hold("text_delta", "world");
        let runs: Vec<(&str, &str)> = pending
            .parts
            .iter()
            .map(|(kind, text)| (*kind, text.as_str()))
            .collect();
        assert_eq!(
            runs,
            vec![
                ("text_delta", "Hello "),
                ("thinking", "because"),
                ("text_delta", "world"),
            ],
            "thinking between two answer runs must not merge them or move either"
        );
    }

    #[test]
    fn a_run_goes_out_on_whichever_bound_it_reaches_first() {
        let mut pending = Pending::default();
        assert!(
            !pending.due_to_send(Instant::now()),
            "nothing held is not a message"
        );

        pending.hold("text_delta", "a few words");
        assert!(
            !pending.due_to_send(pending.since.unwrap()),
            "a short burst waits — measured from its own arrival, so a stalled CI host cannot \
             make this flaky"
        );

        for _ in 0..200 {
            pending.hold("text_delta", "0123456789");
        }
        assert!(
            pending.due_to_send(Instant::now()),
            "the byte cap is what bounds a fast stream"
        );

        let mut trickle = Pending::default();
        trickle.hold("text_delta", "one token");
        let arrived = trickle.since.unwrap();
        assert!(
            !trickle.due_to_send(arrived),
            "a lone token is never late on arrival — that is the latency a slow model keeps"
        );
        assert!(
            trickle.due_to_send(arrived + COALESCE_WINDOW),
            "the window is what bounds a slow one"
        );
    }
}
