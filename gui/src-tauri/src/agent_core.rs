use std::sync::Arc;

use firment_core::{Agent, Config, Session};
use tauri::Emitter;

use crate::events::FrontendEvent;
use crate::state::Shared;
use crate::ui::{GuiAsker, GuiPermission, GuiSink};

/// Rebuild the agent around a session, wiring GUI sinks for events,
/// permissions and questions. All agent knobs (registries, plan-mode policy,
/// tool config, subagents) come from the shared [`firment_tools::assembly`]
/// module so the GUI, TUI and CLI stay behaviourally identical.
pub fn build_agent(shared: &Arc<Shared>, session: Session) -> anyhow::Result<Agent> {
    let config = shared.config.lock().unwrap().clone();
    let merged = config.merged_for(&session.cwd);

    let sink: Arc<GuiSink> = Arc::new(GuiSink {
        shared: shared.clone(),
    });
    let permission: Arc<GuiPermission> = Arc::new(GuiPermission {
        shared: shared.clone(),
    });
    let asker: Arc<GuiAsker> = Arc::new(GuiAsker {
        shared: shared.clone(),
    });

    // The GUI permission dialog is the decision point, so dangerous shell
    // commands may reach it (the frontend labels them ⚠).
    let mut assembly = firment_tools::assembly::assemble_agent(
        &merged,
        session,
        shared.store.lock().unwrap().clone(),
        sink,
        permission,
        Some(asker),
        true,
    );

    if let Some(error) = assembly.provider_error.take() {
        let _ = shared
            .app
            .emit("agent-event", FrontendEvent::Error { message: error });
    }

    // Cancellation handles were extracted by the assembly BEFORE the agent
    // moves into the lock. `run_turn` holds the agent lock for the whole
    // turn, so cancel must be able to fire these directly without
    // contending for the same lock.
    *shared.cancel.lock().unwrap() = Some((assembly.cancel_tx, assembly.cancel_signal));

    Ok(assembly.agent)
}

pub fn default_provider_model(config: &Config) -> (String, String) {
    let provider = config.default_provider.clone();
    let model = config
        .providers
        .get(&provider)
        .map(|p| p.model.clone())
        .unwrap_or_default();
    (provider, model)
}
