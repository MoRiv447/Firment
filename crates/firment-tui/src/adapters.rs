//! UI-thread adapters between the agent core and the Tauri-style event loop:
//! the event sink, the permission checker and the ask_user bridge. Each one
//! owns a channel whose other end lives on the UI thread.

use async_trait::async_trait;
use firment_core::{
    AgentEvent, Asker, EventSink, PermissionChecker, PermissionError, QuestionRequest,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
pub(crate) struct ChannelSink {
    pub(crate) tx: mpsc::Sender<AgentEvent>,
}

#[async_trait]
impl EventSink for ChannelSink {
    async fn event(&self, event: AgentEvent) {
        let _ = self.tx.send(event).await;
    }
}

pub(crate) struct PermissionRequest {
    pub(crate) tool: String,
    pub(crate) reason: String,
    pub(crate) reply: oneshot::Sender<bool>,
}

pub(crate) struct TuiPermission {
    pub(crate) req_tx: mpsc::Sender<PermissionRequest>,
    pub(crate) always: Arc<Mutex<HashSet<String>>>,
}

#[async_trait]
impl PermissionChecker for TuiPermission {
    async fn confirm(
        &self,
        tool: &str,
        _args: &serde_json::Value,
        reason: &str,
    ) -> Result<(), PermissionError> {
        if self.already_approved(tool) {
            return Ok(());
        }
        let (reply, rx) = oneshot::channel();
        self.req_tx
            .send(PermissionRequest {
                tool: tool.to_string(),
                reason: reason.to_string(),
                reply,
            })
            .await
            .map_err(|_| PermissionError::denied("TUI closed while asking for approval"))?;
        match rx.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(PermissionError::denied("denied by user")),
            Err(_) => Err(PermissionError::denied(
                "TUI closed while waiting for approval",
            )),
        }
    }
}

impl TuiPermission {
    fn already_approved(&self, tool: &str) -> bool {
        self.always
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(tool)
    }
}

/// Forwards `ask_user` questions to the UI thread, which shows a modal; the
/// agent blocks until the user answers or dismisses it.
pub(crate) struct TuiAsker {
    pub(crate) req_tx: mpsc::Sender<QuestionRequest>,
}

#[async_trait]
impl Asker for TuiAsker {
    async fn ask(&self, question: &str, options: &[String]) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.req_tx
            .send(QuestionRequest {
                question: question.to_string(),
                options: options.to_vec(),
                reply,
            })
            .await
            .map_err(|_| "TUI closed while asking a question".to_string())?;
        rx.await
            .map_err(|_| "TUI closed while waiting for an answer".to_string())?
            .ok_or_else(|| "user declined the question".to_string())
    }
}
