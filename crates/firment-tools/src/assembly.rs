//! Shared agent assembly used by every frontend (TUI, GUI, CLI headless).
//!
//! Historically each frontend wired the `Agent` by hand — an identical
//! `set_*` sequence duplicated in `firment-tui`, `gui/src-tauri` and
//! `firment-cli` — and the copies had already drifted (the verify gate and
//! `max_subagent_depth` were only wired in the CLI). This module is the
//! single wiring point: frontends supply their I/O adapters (sink,
//! permission checker, asker) and get back a fully configured agent.

use crate::{attacker_registry, default_registry, plan_registry};
use firment_core::{
    Agent, Asker, Cancellable, Config, EventSink, PermissionChecker, PlanModePermission, Session,
    SessionMode, SessionStore, SubagentRunner,
};
use std::sync::Arc;
use tokio::sync::watch::Sender;

/// The assembled agent plus the handles that must be extracted up front.
///
/// `cancel_tx` / `cancel_signal` are pre-extracted because `run_turn` holds
/// the agent lock for the whole turn: cancel must fire directly, without
/// contending for that lock (Esc in the TUI, `cancel_turn` in the GUI).
pub struct AgentAssembly {
    pub agent: Agent,
    pub cancel_tx: Sender<bool>,
    pub cancel_signal: Cancellable,
    /// Set when the provider failed to build (missing API key, bad endpoint).
    /// The agent is still returned so interactive frontends can start anyway
    /// (the TUI offers `/apikey` inside the session); headless callers should
    /// treat this as a fatal error.
    pub provider_error: Option<String>,
}

/// Build a fully wired agent for `session` from the merged config.
///
/// `permission` is the frontend's base checker; in plan mode it is wrapped in
/// [`PlanModePermission`] and the registry is swapped to the read-only set,
/// so callers never hand-roll the plan-mode policy again. The same base
/// checker (unwrapped) backs research subagents — they already run the
/// read-only registry, so the plan wrapper would be redundant.
pub fn assemble_agent(
    merged: &Config,
    session: Session,
    store: SessionStore,
    sink: Arc<dyn EventSink>,
    permission: Arc<dyn PermissionChecker>,
    asker: Option<Arc<dyn Asker>>,
    allow_dangerous: bool,
) -> AgentAssembly {
    let (provider, provider_error) =
        match merged.build_provider(Some(&session.provider), Some(&session.model)) {
            Ok(provider) => (Some(provider), None),
            Err(e) => (None, Some(e.to_string())),
        };

    let plan = session.mode == SessionMode::Plan;
    let registry = if plan {
        plan_registry()
    } else {
        default_registry()
    };
    let agent_permission: Arc<dyn PermissionChecker> = if plan {
        Arc::new(PlanModePermission::new(permission.clone()))
    } else {
        permission.clone()
    };
    // Keep a handle on the sink for the attacker runner (Agent::new moves it).
    let attacker_sink = sink.clone();
    // Same for the permission: the attacker runner is built after the
    // research runner has consumed its clones.
    let attacker_permission = agent_permission.clone();

    let mut agent = Agent::new(
        provider,
        registry,
        session,
        store.clone(),
        agent_permission,
        sink,
        merged.max_iterations,
    );

    agent.set_allow_dangerous(allow_dangerous);
    agent.set_verify_command(merged.tools.verify_command.clone());
    agent.set_context_budget_chars(merged.context_budget_chars);
    // Device-log location for the device_log tool: the desktop MQTT link
    // writes next to config.toml.
    agent.set_device_log_dir(Some(firment_core::config::config_dir()));
    // Delegation guidance: surface every configured provider (name + model)
    // so the model knows which cheap backends it can dispatch subtasks to.
    agent.set_providers(
        merged
            .providers
            .iter()
            .map(|(name, p)| (name.clone(), p.model.clone()))
            .collect(),
    );
    // Model discovery for the `models` tool: base_url defaulted by provider
    // type, API key resolved inline → env → auth.json (shared helper).
    agent.set_provider_endpoints(firment_core::config::provider_endpoints(merged));
    agent.set_compaction_strategy(merged.compaction_strategy);
    agent.set_symbols_backend(merged.tools.symbols_backend.clone());
    agent.set_build_command(merged.tools.build_command.clone());
    agent.set_default_chip(merged.tools.default_chip.clone());
    agent.set_monitor_port(merged.tools.monitor_port.clone());
    agent.set_monitor_baud(merged.tools.monitor_baud);
    agent.set_elf_config(merged.tools.elf.clone());
    agent.set_la_config(merged.tools.la.clone());
    agent.set_max_subagent_depth(merged.tools.max_subagent_depth);
    agent.set_asker(asker.clone());
    agent.set_web_search(
        merged.tools.web_search.clone(),
        merged.tools.resolved_web_search_api_key(),
    );
    agent.set_session_dir(Some(store.dir.join("work")));

    let subagent_factory: Arc<SubagentRunner> = Arc::new(SubagentRunner::new(
        Arc::new(merged.clone()),
        plan_registry(),
        agent.session().provider.clone(),
        agent.session().model.clone(),
        asker,
        permission,
    ));
    agent.set_subagent_factory(Some(subagent_factory));

    // Attacker-profile runner for the `redteam` campaign: hardware-capable
    // registry, the parent's sink (attack tool cards stream into the live
    // UI), and the parent's permission — approval popups still reach the
    // user; the campaign wraps it in TargetLockPermission at call time to
    // confine the attack to the suite's declared interfaces.
    if !plan {
        let attacker = SubagentRunner {
            max_iterations: 16,
            sink: attacker_sink,
            ..SubagentRunner::new(
                Arc::new(merged.clone()),
                attacker_registry(),
                agent.session().provider.clone(),
                agent.session().model.clone(),
                None,
                attacker_permission,
            )
        };
        agent.set_attacker_factory(Some(Arc::new(attacker)));
    }

    let (cancel_tx, cancel_signal) = agent.cancel_handle();

    AgentAssembly {
        agent,
        cancel_tx,
        cancel_signal,
        provider_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firment_core::{AgentEvent, ProviderConfig};
    use std::path::PathBuf;

    struct NullSink;

    #[async_trait::async_trait]
    impl EventSink for NullSink {
        async fn event(&self, _event: AgentEvent) {}
    }

    struct AllowAll;

    #[async_trait::async_trait]
    impl PermissionChecker for AllowAll {
        async fn confirm(
            &self,
            _tool: &str,
            _args: &serde_json::Value,
            _reason: &str,
        ) -> Result<(), firment_core::PermissionError> {
            Ok(())
        }
    }

    fn test_config() -> Config {
        let mut config = Config::default_config();
        config.providers.insert(
            "test".to_string(),
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: Some("http://127.0.0.1:9/v1".to_string()),
                api_key_env: None,
                api_key: Some("sk-test".to_string()),
                model: "test-model".to_string(),
                max_tokens: None,
                temperature: None,
            },
        );
        config.default_provider = "test".to_string();
        config
    }

    fn test_session(mode: SessionMode) -> Session {
        let mut session = Session::new(PathBuf::from("."), "test", "test-model");
        session.mode = mode;
        session
    }

    fn assemble(mode: SessionMode) -> AgentAssembly {
        let config = test_config();
        assemble_agent(
            &config,
            test_session(mode),
            SessionStore::new(std::env::temp_dir().join("firment-assembly-test")),
            Arc::new(NullSink),
            Arc::new(AllowAll),
            None,
            false,
        )
    }

    #[test]
    fn agent_mode_exposes_mutating_tools() {
        let assembly = assemble(SessionMode::Agent);
        let names: Vec<String> = assembly
            .agent
            .registry()
            .specs()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.iter().any(|n| n == "write_file"));
        assert!(names.iter().any(|n| n == "shell"));
        assert_eq!(assembly.provider_error, None);
    }

    #[test]
    fn plan_mode_hides_mutating_tools() {
        let assembly = assemble(SessionMode::Plan);
        let names: Vec<String> = assembly
            .agent
            .registry()
            .specs()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(!names.iter().any(|n| n == "write_file"));
        assert!(!names.iter().any(|n| n == "shell"));
        assert!(names.iter().any(|n| n == "read_file"));
    }

    #[test]
    fn config_knobs_flow_into_the_agent() {
        let mut config = test_config();
        config.tools.verify_command = Some("cargo check".to_string());
        config.tools.max_subagent_depth = 5;
        let assembly = assemble_agent(
            &config,
            test_session(SessionMode::Agent),
            SessionStore::new(std::env::temp_dir().join("firment-assembly-test")),
            Arc::new(NullSink),
            Arc::new(AllowAll),
            None,
            false,
        );
        // Regression guards: these two were historically wired only in the
        // CLI, silently disabling the verify gate and the configured subagent
        // depth in the TUI/GUI.
        assert_eq!(assembly.agent.verify_command(), Some("cargo check"));
        assert_eq!(assembly.agent.max_subagent_depth(), 5);
        assert_eq!(assembly.agent.session().mode, SessionMode::Agent);
        assert_eq!(assembly.agent.session().model, "test-model");
    }

    #[test]
    fn defaults_hold_when_config_leaves_knobs_unset() {
        let assembly = assemble(SessionMode::Agent);
        assert_eq!(assembly.agent.verify_command(), None);
        // ToolsConfig::default() ships max_subagent_depth = 2.
        assert_eq!(assembly.agent.max_subagent_depth(), 2);
    }

    #[test]
    fn provider_failure_is_reported_not_fatal() {
        let mut config = Config::default_config();
        config.providers.insert(
            "firment-assembly-test-noauth".to_string(),
            ProviderConfig {
                r#type: "openai".to_string(),
                base_url: Some("http://127.0.0.1:9/v1".to_string()),
                // Neither inline key nor this env var exists, so
                // build_provider must fail with MissingApiKey.
                api_key_env: Some("FIRMENT_ASSEMBLY_TEST_UNSET_KEY".to_string()),
                api_key: None,
                model: "m".to_string(),
                max_tokens: None,
                temperature: None,
            },
        );
        config.default_provider = "firment-assembly-test-noauth".to_string();
        let assembly = assemble_agent(
            &config,
            Session::new(PathBuf::from("."), "firment-assembly-test-noauth", "m"),
            SessionStore::new(std::env::temp_dir().join("firment-assembly-test")),
            Arc::new(NullSink),
            Arc::new(AllowAll),
            None,
            false,
        );
        assert!(assembly.provider_error.is_some());
    }
}
