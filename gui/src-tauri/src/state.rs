use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio::sync::watch;

use firment_core::Agent;
use firment_core::Cancellable;
use firment_core::SessionStore;
use firment_core::ToolRegistry;

use crate::collab::CollabBackend;
use crate::hardware::SerialMonitor;

/// Pre-extracted turn cancellation handles, kept OUTSIDE the agent lock.
/// `run_turn` takes `&mut self` and holds the lock for the whole turn, so
/// `cancel_turn` must fire these directly instead of going through
/// `agent.cancel()` — otherwise cancel would block on the same lock and
/// never take effect (mirrors the TUI's command-loop design).
pub type CancelHandles = (watch::Sender<bool>, Cancellable);

#[derive(Clone)]
pub struct Shared {
    pub app: tauri::AppHandle,
    pub config_path: PathBuf,
    pub config: Arc<Mutex<firment_core::Config>>,
    pub store: Arc<Mutex<SessionStore>>,
    #[allow(dead_code)] // reserved for the M4 tool-schema pane
    pub registry: Arc<ToolRegistry>,
    pub agent: Arc<tokio::sync::Mutex<Option<Agent>>>,
    pub cancel: Arc<Mutex<Option<CancelHandles>>>,
    pub perm_waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<bool>>>>,
    pub ask_waiters: Arc<Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>>,
    #[allow(dead_code)] // reserved for the M4 collaboration panel
    pub collab: Arc<dyn CollabBackend>,
    pub running: Arc<AtomicBool>,
    pub monitors: Arc<Mutex<HashMap<String, Arc<SerialMonitor>>>>,
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
