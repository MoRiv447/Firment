use std::sync::Arc;

use firment_core::Agent;
use firment_core::Config;
use firment_core::Session;
use firment_core::SessionMode;
use tauri::Emitter;

use crate::events::FrontendEvent;
use crate::state::Shared;
use crate::ui::{GuiAsker, GuiPermission, GuiSink};

/// Rebuild the agent around a session, wiring GUI sinks for events,
/// permissions and questions. Mirrors `firment-tui`'s setup so behaviour
/// matches the TUI exactly.
pub fn build_agent(shared: &Arc<Shared>, session: Session) -> anyhow::Result<Agent> {
    let config = shared.config.lock().unwrap().clone();
    let merged = config.clone().merged_for(&session.cwd);

    let provider = match merged.build_provider(Some(&session.provider), Some(&session.model)) {
        Ok(p) => Some(p),
        Err(e) => {
            let _ = shared
                .app
                .emit("agent-event", FrontendEvent::Error { message: e.to_string() });
            None
        }
    };

    let sink: Arc<GuiSink> = Arc::new(GuiSink {
        shared: shared.clone(),
    });
    let permission: Arc<GuiPermission> = Arc::new(GuiPermission {
        shared: shared.clone(),
    });
    let asker: Arc<GuiAsker> = Arc::new(GuiAsker {
        shared: shared.clone(),
    });

    let default_registry = firment_tools::default_registry();
    let plan_registry = firment_tools::plan_registry();

    let plan = session.mode == SessionMode::Plan;
    let registry = if plan { plan_registry } else { default_registry };
    let plan_permission: Arc<dyn firment_core::PermissionChecker> = permission.clone();
    let permission: Arc<dyn firment_core::PermissionChecker> = if plan {
        Arc::new(firment_core::PlanModePermission::new(permission))
    } else {
        permission
    };

    let store = shared.store.lock().unwrap().clone();
    let mut agent = Agent::new(
        provider,
        registry,
        session.clone(),
        store,
        permission,
        sink,
        merged.max_iterations,
    );

    // The GUI permission dialog is the decision point, so dangerous shell
    // commands may reach it (the frontend labels them ⚠).
    agent.set_allow_dangerous(true);
    agent.set_context_budget_chars(merged.context_budget_chars);
    agent.set_compaction_strategy(merged.compaction_strategy);
    agent.set_symbols_backend(merged.tools.symbols_backend.clone());
    agent.set_build_command(merged.tools.build_command.clone());
    agent.set_default_chip(merged.tools.default_chip.clone());
    agent.set_monitor_port(merged.tools.monitor_port.clone());
    agent.set_monitor_baud(merged.tools.monitor_baud);
    agent.set_elf_glob(merged.tools.elf.clone());
    agent.set_asker(Some(asker.clone() as Arc<dyn firment_core::Asker>));
    agent.set_web_search(
        merged.tools.web_search.clone(),
        merged.tools.resolved_web_search_api_key(),
    );
    let store_dir = firment_core::config::config_dir();
    agent.set_session_dir(Some(store_dir.join("sessions").join("work")));

    let subagent_factory: Arc<firment_core::SubagentRunner> = Arc::new(
        firment_core::SubagentRunner::new(
            Arc::new(merged.clone()),
            firment_tools::plan_registry(),
            session.provider.clone(),
            session.model.clone(),
            Some(asker),
            plan_permission,
        ),
    );
    agent.set_subagent_factory(Some(subagent_factory));

    // Extract cancellation handles BEFORE the agent moves into the lock.
    // `run_turn` holds the agent lock for the whole turn, so cancel must be
    // able to fire these directly without contending for the same lock.
    let (cancel_tx, cancel_signal) = agent.cancel_handle();
    *shared.cancel.lock().unwrap() = Some((cancel_tx, cancel_signal));

    Ok(agent)
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