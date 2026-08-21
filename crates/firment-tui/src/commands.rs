//! The agent command loop: a dedicated task that owns the `Agent` behind its
//! lock and services `AgentCmd` requests from the UI thread. Turns run on
//! their own task (so the loop stays responsive) and `Cancel` fires the
//! pre-extracted cancellation handles directly instead of going through the
//! agent lock — together that is what makes Esc interrupt a running turn.

use firment_core::{
    Agent, AgentEvent, Cancellable, Config, PermissionChecker, ProviderConfig, Session,
    SessionMode, SessionStore, ThinkingLevel, ToolRegistry,
};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_agent_task(
    mut cmd_rx: mpsc::Receiver<AgentCmd>,
    agent: Arc<tokio::sync::Mutex<Agent>>,
    cancel_tx: watch::Sender<bool>,
    cancel_signal: Cancellable,
    turn_lock: Arc<tokio::sync::Mutex<()>>,
    store: SessionStore,
    mut task_config: Config,
    task_config_path: std::path::PathBuf,
    plan_registry: Arc<ToolRegistry>,
    default_registry: Arc<ToolRegistry>,
    plan_permission: Arc<dyn PermissionChecker>,
    tui_permission: Arc<dyn PermissionChecker>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                AgentCmd::User(text) => {
                    // Run the turn on its own task so the command loop stays
                    // responsive and Cancel can interrupt mid-turn.
                    let agent = agent.clone();
                    let turn_lock = turn_lock.clone();
                    tokio::spawn(async move {
                        let _turn = turn_lock.lock().await;
                        let mut agent = agent.lock().await;
                        agent.reset_cancel();
                        if let Err(e) = agent.run_turn(&text).await {
                            agent.emit(AgentEvent::Error(e.to_string())).await;
                            // Error paths of run_turn (max iterations, provider
                            // failure, ...) emit no TurnEnd, so the TUI would
                            // stay busy forever; close the turn explicitly.
                            agent
                                .emit(AgentEvent::TurnEnd {
                                    text: String::new(),
                                })
                                .await;
                            let _ = agent.save_session();
                        }
                    });
                }
                AgentCmd::Cancel => {
                    let _ = cancel_tx.send(true);
                    cancel_signal.cancel();
                }
                AgentCmd::SetModel(model) => {
                    let mut agent = agent.lock().await;
                    agent.set_model(model.clone());
                    if let Some(provider) = task_config.providers.get_mut(&agent.session().provider)
                    {
                        provider.model = model.clone();
                    }
                    let _ = task_config.save(&task_config_path);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!("model -> {model} (saved)")))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: Some(model),
                            thinking: None,
                            mode: None,
                        })
                        .await;
                }
                AgentCmd::SetThinking(level) => {
                    let mut agent = agent.lock().await;
                    agent.set_thinking(level);
                    task_config.thinking = level;
                    let _ = task_config.save(&task_config_path);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!(
                            "thinking -> {} (saved)",
                            level.label()
                        )))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: Some(level),
                            mode: None,
                        })
                        .await;
                }
                AgentCmd::SetContextBudget(chars) => {
                    let mut agent = agent.lock().await;
                    agent.set_context_budget_chars(chars);
                    task_config.context_budget_chars = chars;
                    let _ = task_config.save(&task_config_path);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!(
                            "context budget -> {chars} chars (saved)"
                        )))
                        .await;
                }
                AgentCmd::SetMaxOutputTokens(tokens) => {
                    let mut agent = agent.lock().await;
                    task_config.max_output_tokens = Some(tokens);
                    let _ = task_config.save(&task_config_path);
                    // Rebuild the provider so the new cap applies to the very
                    // next request (max_tokens is fixed at provider creation).
                    let model = agent.session().model.clone();
                    match task_config.build_provider(Some(&agent.session().provider), Some(&model))
                    {
                        Ok(new_provider) => {
                            agent.set_provider(new_provider);
                            let _ = agent.save_session();
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "max output tokens -> {tokens} (saved, applies to next \
                                     request)"
                                )))
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!(
                                    "failed to apply output cap: {e}"
                                )))
                                .await;
                        }
                    }
                }
                AgentCmd::ShowContext => {
                    let agent = agent.lock().await;
                    agent.emit(AgentEvent::Info(agent.context_usage())).await;
                }
                AgentCmd::DeleteSession(id) => {
                    let agent = agent.lock().await;
                    if id == agent.session().id {
                        agent
                            .emit(AgentEvent::Error(
                                "refusing to delete the active session: the in-memory \
                                 session would re-create its files on the next save; \
                                 start a new session first (/new), then delete the old id"
                                    .to_string(),
                            ))
                            .await;
                    } else {
                        match agent.session_store().delete(&id) {
                            Ok(()) => {
                                agent
                                    .emit(AgentEvent::Info(format!(
                                        "deleted session {id} (transcript, undo, spill, ledger)"
                                    )))
                                    .await;
                            }
                            Err(e) => {
                                agent
                                    .emit(AgentEvent::Error(format!("delete failed: {e}")))
                                    .await;
                            }
                        }
                    }
                }
                AgentCmd::SetMode(mode) => {
                    let mut agent = agent.lock().await;
                    let registry = if mode == SessionMode::Plan {
                        plan_registry.clone()
                    } else {
                        default_registry.clone()
                    };
                    let permission: Arc<dyn PermissionChecker> = if mode == SessionMode::Plan {
                        plan_permission.clone()
                    } else {
                        tui_permission.clone()
                    };
                    agent.set_mode(mode, registry, permission);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(format!(
                            "mode -> {} (takes effect from the next message)",
                            mode.label()
                        )))
                        .await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: None,
                            mode: Some(mode),
                        })
                        .await;
                }
                AgentCmd::OpenModelPicker => {
                    // Fetch the provider name under a short lock, then run the
                    // (slow, network) model list request OUTSIDE the agent lock
                    // so a laggy provider cannot freeze the command loop.
                    let provider_name = {
                        let agent = agent.lock().await;
                        agent.session().provider.clone()
                    };
                    match task_config.list_models(&provider_name).await {
                        Ok(models) => {
                            let agent = agent.lock().await;
                            agent.emit(AgentEvent::Models(models)).await;
                        }
                        Err(e) => {
                            let agent = agent.lock().await;
                            agent
                                .emit(AgentEvent::Error(format!(
                                    "failed to fetch the model list: {e}"
                                )))
                                .await;
                            agent.emit(AgentEvent::Models(Vec::new())).await;
                        }
                    }
                }
                AgentCmd::OpenSessionPicker => match store.list() {
                    Ok(mut sessions) => {
                        let agent = agent.lock().await;
                        for summary in &mut sessions {
                            if summary.preview.is_empty() {
                                summary.preview = store
                                    .load(&summary.id)
                                    .map(|s| s.title())
                                    .unwrap_or_default();
                            }
                        }
                        agent.emit(AgentEvent::Sessions(sessions)).await;
                    }
                    Err(e) => {
                        let agent = agent.lock().await;
                        agent
                            .emit(AgentEvent::Error(format!("failed to list sessions: {e}")))
                            .await;
                        agent.emit(AgentEvent::Sessions(Vec::new())).await;
                    }
                },
                AgentCmd::NewSession => {
                    let mut agent = agent.lock().await;
                    let fresh = Session::new(
                        agent.session().cwd.clone(),
                        agent.session().provider.clone(),
                        agent.session().model.clone(),
                    );
                    let registry = default_registry.clone();
                    let permission: Arc<dyn PermissionChecker> = tui_permission.clone();
                    agent.replace_session(fresh.clone());
                    agent.set_mode(SessionMode::Agent, registry, permission);
                    let _ = agent.save_session();
                    agent
                        .emit(AgentEvent::Info(
                            "Started a new conversation (current provider/model kept)".to_string(),
                        ))
                        .await;
                    agent.emit(AgentEvent::SessionLoaded(fresh)).await;
                    agent
                        .emit(AgentEvent::Settings {
                            provider: None,
                            model: None,
                            thinking: None,
                            mode: Some(SessionMode::Agent),
                        })
                        .await;
                }
                AgentCmd::LoadSession(id) => match store.load(&id) {
                    Ok(loaded) => {
                        let mut agent = agent.lock().await;
                        let mode = loaded.mode;
                        agent.replace_session(loaded.clone());
                        match task_config
                            .build_provider(Some(&loaded.provider), Some(&loaded.model))
                        {
                            Ok(provider) => agent.set_provider(provider),
                            Err(e) => {
                                agent
                                    .emit(AgentEvent::Error(format!(
                                        "session switched, but rebuilding the provider failed: {e}"
                                    )))
                                    .await;
                            }
                        }
                        let registry = if mode == SessionMode::Plan {
                            plan_registry.clone()
                        } else {
                            default_registry.clone()
                        };
                        let permission: Arc<dyn PermissionChecker> = if mode == SessionMode::Plan {
                            plan_permission.clone()
                        } else {
                            tui_permission.clone()
                        };
                        agent.set_mode(mode, registry, permission);
                        let _ = agent.save_session();
                        agent
                            .emit(AgentEvent::Info(format!(
                                "Switched to session {} ({} · {} · {})",
                                loaded.id,
                                loaded.provider,
                                loaded.model,
                                mode.label()
                            )))
                            .await;
                        agent.emit(AgentEvent::SessionLoaded(loaded.clone())).await;
                        agent
                            .emit(AgentEvent::Settings {
                                provider: Some(loaded.provider.clone()),
                                model: Some(loaded.model.clone()),
                                thinking: Some(loaded.thinking),
                                mode: Some(mode),
                            })
                            .await;
                    }
                    Err(e) => {
                        let agent = agent.lock().await;
                        agent
                            .emit(AgentEvent::Error(format!("failed to load session: {e}")))
                            .await;
                    }
                },
                AgentCmd::Undo => {
                    let mut agent = agent.lock().await;
                    match agent.undo_last().await {
                        Ok(summary) => {
                            agent.emit(AgentEvent::Info(summary)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("undo failed: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::Ledger => {
                    let agent = agent.lock().await;
                    let summary = agent.ledger_summary();
                    if summary.is_empty() {
                        agent
                            .emit(AgentEvent::Info(
                                "No committed edits in this session yet".to_string(),
                            ))
                            .await;
                    } else {
                        agent
                            .emit(AgentEvent::Info(format!(
                                "Recent change ledger:\n{summary}"
                            )))
                            .await;
                    }
                }
                AgentCmd::Pin { path } => {
                    let agent = agent.lock().await;
                    match agent.pin_path(std::path::PathBuf::from(&path)) {
                        Ok(message) => {
                            agent.emit(AgentEvent::Info(message)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("pin failed: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::Unpin { path } => {
                    let agent = agent.lock().await;
                    match agent.unpin_path(std::path::PathBuf::from(&path)) {
                        Ok(message) => {
                            agent.emit(AgentEvent::Info(message)).await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("unpin failed: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::SetProvider(name) => {
                    let mut agent = agent.lock().await;
                    match task_config.build_provider(Some(&name), None) {
                        Ok(new_provider) => {
                            let configured_model = new_provider.model().to_string();
                            agent.set_provider_name(&name);
                            agent.set_provider(new_provider);
                            agent.set_model(configured_model.clone());
                            task_config.default_provider = name.clone();
                            let _ = task_config.save(&task_config_path);
                            let _ = agent.save_session();
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "provider -> {name} · model -> {configured_model} (saved)"
                                )))
                                .await;
                            agent
                                .emit(AgentEvent::Settings {
                                    provider: Some(name),
                                    model: Some(configured_model),
                                    thinking: None,
                                    mode: None,
                                })
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to switch provider: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::SetApiKey { provider, key } => {
                    let mut agent = agent.lock().await;
                    let provider_name =
                        provider.unwrap_or_else(|| agent.session().provider.clone());
                    match task_config.set_api_key(&provider_name, &key) {
                        Ok(()) => {
                            let model = agent.session().model.clone();
                            match task_config.build_provider(Some(&provider_name), Some(&model)) {
                                Ok(new_provider) => {
                                    agent.set_provider(new_provider);
                                    agent
                                        .emit(AgentEvent::Info(format!(
                                            "API key for {provider_name} saved to {} (no \
                                             further setup needed)",
                                            firment_core::auth_path().display()
                                        )))
                                        .await;
                                }
                                Err(e) => {
                                    agent
                                        .emit(AgentEvent::Error(format!(
                                            "rebuilding the provider after saving failed: {e}"
                                        )))
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to save API key: {e}")))
                                .await;
                        }
                    }
                }
                AgentCmd::ListModels => {
                    // Short lock for the provider name, network call outside
                    // the lock (same reason as OpenModelPicker).
                    let provider_name = {
                        let agent = agent.lock().await;
                        agent.session().provider.clone()
                    };
                    match task_config.list_models(&provider_name).await {
                        Ok(models) => {
                            let agent = agent.lock().await;
                            let providers: Vec<String> =
                                task_config.providers.keys().cloned().collect();
                            let mut msg = format!(
                                "Configured providers: {} (current: {})\nAvailable models:",
                                providers.join(", "),
                                provider_name
                            );
                            if models.is_empty() {
                                msg.push_str("\n  (the API returned no models; set one manually with /model <id>)");
                            } else {
                                for model in models {
                                    msg.push_str(&format!("\n  {model}"));
                                }
                            }
                            msg.push_str("\nSwitch: /model <id> or /provider <name>");
                            agent.emit(AgentEvent::Info(msg)).await;
                        }
                        Err(e) => {
                            let agent = agent.lock().await;
                            agent
                                .emit(AgentEvent::Error(format!(
                                    "failed to fetch the model list: {e}"
                                )))
                                .await;
                        }
                    }
                }
                AgentCmd::AddProvider {
                    name,
                    r#type,
                    base_url,
                    model,
                } => {
                    let agent = agent.lock().await;
                    let entry = task_config
                        .providers
                        .entry(name.clone())
                        .or_insert_with(|| ProviderConfig {
                            r#type: r#type.clone(),
                            base_url: Some(base_url.clone()),
                            api_key_env: None,
                            api_key: None,
                            model: model.clone(),
                            max_tokens: None,
                            temperature: None,
                        });
                    entry.r#type = r#type.clone();
                    entry.base_url = Some(base_url.clone());
                    entry.model = model.clone();
                    match task_config.save(&task_config_path) {
                        Ok(()) => {
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "provider {name} saved; next run /apikey {name} sk-xxx to set \
                                     the key"
                                )))
                                .await;
                        }
                        Err(e) => {
                            agent
                                .emit(AgentEvent::Error(format!("failed to save provider: {e}")))
                                .await;
                        }
                    }
                }
            }
        }
    })
}
#[derive(Debug)]
pub(crate) enum AgentCmd {
    User(String),
    Cancel,
    SetModel(String),
    SetThinking(ThinkingLevel),
    SetContextBudget(usize),
    SetMaxOutputTokens(u32),
    ShowContext,
    DeleteSession(String),
    SetMode(SessionMode),
    OpenModelPicker,
    OpenSessionPicker,
    NewSession,
    LoadSession(String),
    Undo,
    Ledger,
    Pin {
        path: String,
    },
    Unpin {
        path: String,
    },
    SetProvider(String),
    SetApiKey {
        provider: Option<String>,
        key: String,
    },
    ListModels,
    AddProvider {
        name: String,
        r#type: String,
        base_url: String,
        model: String,
    },
}
