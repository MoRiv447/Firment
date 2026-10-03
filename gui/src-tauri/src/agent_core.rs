use std::sync::Arc;

use firment_core::{Agent, Config, Session};

use crate::events::FrontendEvent;
use crate::state::{CancelHandles, Shared};
use crate::ui::{route_event, GuiAsker, GuiPermission, GuiSink, TerminalHold};

/// Build the agent around a session, wiring GUI sinks for events,
/// permissions and questions. All agent knobs (registries, plan-mode policy,
/// tool config, subagents) come from the shared [`firment_tools::assembly`]
/// module so the GUI, TUI and CLI stay behaviourally identical.
///
/// Returns the cancellation handles alongside the agent: `run_turn` holds
/// the agent lock for the whole turn, so cancel must be able to fire these
/// directly without contending for that lock. The caller stores them in the
/// session's [`crate::state::AgentSlot`].
///
/// `terminal` is where the turn's closing notices are parked instead of sent — see
/// [`crate::ui::TerminalHold`]. The caller owns it and drains it after releasing the slot, and
/// everything else this function emits goes through it so there is one door.
pub fn build_agent(
    shared: &Arc<Shared>,
    session: Session,
    terminal: &Arc<TerminalHold>,
) -> anyhow::Result<(Agent, CancelHandles)> {
    let config = shared
        .config
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let merged = config.merged_for(&session.cwd);

    let sink: Arc<GuiSink> = Arc::new(GuiSink::new(
        shared.clone(),
        session.id.clone(),
        terminal.clone(),
    ));
    let permission: Arc<GuiPermission> = Arc::new(GuiPermission {
        shared: shared.clone(),
        session_id: session.id.clone(),
    });
    let asker: Arc<GuiAsker> = Arc::new(GuiAsker {
        shared: shared.clone(),
        session_id: session.id.clone(),
    });

    // Captured before `session` moves into the assembly: everything reported here belongs to
    // this chat, and `App.tsx` routes an unstamped event to whichever chat happens to be open.
    let session_id = session.id.clone();

    // The GUI permission dialog is the decision point, so dangerous shell
    // commands may reach it (the frontend labels them ⚠).
    let mut assembly = firment_tools::assembly::assemble_agent(
        &merged,
        session,
        shared
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
        sink,
        permission,
        Some(asker),
        true,
    );

    for refusal in assembly.plugin_refusals.drain(..) {
        // A plugin the user configured that quietly does nothing is a plugin they believe is
        // working. The CLI prints these and the TUI emits them; the GUI showed neither.
        route_event(
            shared,
            terminal,
            FrontendEvent::Info {
                session_id: Some(session_id.clone()),
                message: refusal,
            },
        );
    }

    for warning in &merged.config_warnings {
        // Same reason as above, one layer down: a project file that exists but cannot be used
        // leaves the turn running on settings the user believes are in effect.
        route_event(
            shared,
            terminal,
            FrontendEvent::Info {
                session_id: Some(session_id.clone()),
                message: warning.clone(),
            },
        );
    }

    if let Some(error) = assembly.provider_error.take() {
        // Held, like every other way this turn can end: the slot is already reserved by the time
        // this line runs, so a window reading `running_sessions` right now would be told about a
        // turn whose notice it has already had.
        route_event(
            shared,
            terminal,
            FrontendEvent::Error {
                session_id: Some(session_id.clone()),
                message: error,
            },
        );
    }

    let handles = (assembly.cancel_tx, assembly.cancel_signal);
    Ok((assembly.agent, handles))
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
