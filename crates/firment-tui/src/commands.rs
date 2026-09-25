//! The agent command loop: a dedicated task that owns the `Agent` behind its
//! lock and services `AgentCmd` requests from the UI thread. Turns run on
//! their own task (so the loop stays responsive) and `Cancel` fires the
//! pre-extracted cancellation handles directly instead of going through the
//! agent lock — together that is what makes Esc interrupt a running turn.

use firment_core::{
    Agent, AgentEvent, Cancellable, Config, PermissionChecker, ProviderConfig, Session,
    SessionMode, SessionStore, ThinkingLevel, ToolVerbosity,
};
use futures::FutureExt;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
/// Install the tool set for `mode` and report what it refused.
///
/// Built here, per switch, through the same door the CLI and the GUI use. It used to be two
/// registries chosen once at startup from the built-in sets — `plan_registry()` and
/// `default_registry()` — which contain no plugins at all, so `/plan`, `/new` and `/session` each
/// dropped every plugin the user had configured, without a message, and the plugin came back only
/// on restart. A plugin's declared path is also relative to the session's directory, which a
/// registry built before `/session` moved the session would resolve against the wrong checkout.
async fn apply_mode(
    agent: &mut Agent,
    config: &Config,
    mode: SessionMode,
    planning_permission: Arc<dyn PermissionChecker>,
    normal_permission: Arc<dyn PermissionChecker>,
) -> Vec<String> {
    let planning = mode == SessionMode::Plan;
    let (registry, refusals) = firment_tools::session_registry(
        planning,
        &config.plugins,
        &firment_core::plugin::plugin_command_base(),
    );
    agent.set_mode(
        mode,
        registry,
        if planning {
            planning_permission
        } else {
            normal_permission
        },
    );
    refusals
}

/// Say what the registry refused. A plugin that is absent without an explanation is a plugin the
/// user believes is working, and the CLI and the GUI already print these.
async fn emit_refusals(agent: &mut Agent, refusals: Vec<String>) {
    for refusal in refusals {
        agent.emit(AgentEvent::Info(refusal)).await;
    }
}

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
    plan_permission: Arc<dyn PermissionChecker>,
    tui_permission: Arc<dyn PermissionChecker>,
) -> tokio::task::JoinHandle<()> {
    // One turn body, shared by a fresh prompt and by a retry: the two differ only in
    // where the text comes from. A retry resolves its prompt INSIDE the turn task,
    // under the same lock that runs it, so the rewind and the re-run cannot be
    // reordered by anything else that arrives on the channel -- and the loop needs no
    // sender of its own, which is what keeps a dropped command channel meaning "the UI
    // is gone" rather than "one task still holds a clone".
    let spawn_turn = {
        let agent = agent.clone();
        let turn_lock = turn_lock.clone();
        move |prompt: Option<String>| {
            // Cloned per call: the closure has to stay callable for the next command,
            // so the turn takes its own handles rather than the closure's.
            let agent = agent.clone();
            let turn_lock = turn_lock.clone();
            // Run the turn on its own task so the command loop stays
            // responsive and Cancel can interrupt mid-turn.
            tokio::spawn(async move {
                let _turn = turn_lock.lock().await;
                let mut agent = agent.lock().await;
                let text = match prompt {
                    Some(text) => text,
                    None => match agent.retry_last() {
                        Some(text) => text,
                        None => {
                            // Nothing to repeat. The turn that would have carried
                            // `TurnEnd` never starts, so it is emitted here: a UI left
                            // busy with no turn coming is the failure this loop exists
                            // to prevent.
                            agent
                                .emit(AgentEvent::Info("Nothing to retry yet.".to_string()))
                                .await;
                            agent
                                .emit(AgentEvent::TurnEnd {
                                    text: String::new(),
                                })
                                .await;
                            return;
                        }
                    },
                };
                agent.reset_cancel();
                // A panic inside run_turn (provider serde, a tool bug,
                // ...) would unwind the spawned task silently: no
                // TurnEnd ever fires and the TUI stays busy forever.
                // Catch it, close the turn, keep the app alive.
                let result = std::panic::AssertUnwindSafe(agent.run_turn(&text))
                    .catch_unwind()
                    .await;
                match result {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
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
                    Err(panic_payload) => {
                        let detail = panic_payload
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown panic".to_string());
                        agent
                            .emit(AgentEvent::Error(format!(
                                "internal error (turn panicked): {detail}"
                            )))
                            .await;
                        agent
                            .emit(AgentEvent::TurnEnd {
                                text: String::new(),
                            })
                            .await;
                        let _ = agent.save_session();
                    }
                }
            });
        }
    };

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                AgentCmd::User(text) => {
                    spawn_turn(Some(text));
                }
                // Spawned rather than awaited: §16.2-1 makes this the manual path of a
                // capability whose whole point is not to add waiting, and this loop has to
                // keep servicing `Cancel` while a model call is in flight. The agent lock
                // is taken briefly three times -- read the diff, announce, report -- and
                // never held across the call.
                AgentCmd::ReviewLast => {
                    let agent = agent.clone();
                    let config = task_config.clone();
                    tokio::spawn(async move {
                        use firment_core::review::self_review;
                        let (change, provider) = {
                            let agent = agent.lock().await;
                            let session = agent.session();
                            (session.last_change(), session.provider.clone())
                        };
                        let Some((tool, diff)) = change else {
                            let agent = agent.lock().await;
                            agent
                                .emit(AgentEvent::Info(
                                    "No edit with a diff in this session yet - nothing to review."
                                        .to_string(),
                                ))
                                .await;
                            return;
                        };
                        let path = self_review::path_from_diff(&diff).unwrap_or(tool);
                        {
                            let agent = agent.lock().await;
                            agent
                                .emit(AgentEvent::Info(format!(
                                    "Reviewing the last change to {path}..."
                                )))
                                .await;
                        }
                        let lines =
                            match self_review::review_diff(&config, &provider, &path, &diff, None)
                                .await
                            {
                                Ok(report) => review_lines(&report),
                                // A review that could not run is a failure of the command,
                                // and saying so is the point: the alternative is silence
                                // that reads like "nothing wrong with your change".
                                Err(e) => vec![format!("Review failed: {e}")],
                            };
                        let agent = agent.lock().await;
                        for line in lines {
                            agent.emit(AgentEvent::Info(line)).await;
                        }
                    });
                }
                // The rules are pure and read files, so this runs off the UI thread but
                // needs neither the agent nor a provider — no lock is taken at all.
                AgentCmd::ReviewPath { path } => {
                    let sink = agent.lock().await.sink_handle();
                    tokio::spawn(async move {
                        let lines = review_path_lines(&path);
                        for line in lines {
                            sink.event(AgentEvent::Info(line)).await;
                        }
                    });
                }
                AgentCmd::RetryLast => {
                    spawn_turn(None);
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
                AgentCmd::SetToolVerbosity(level) => {
                    // A display preference: no agent lock, no session save. The
                    // agent never sees it, so there is nothing to reload.
                    task_config.ui.tool_verbosity = level;
                    let saved = match task_config.save(&task_config_path) {
                        Ok(()) => "saved".to_string(),
                        Err(e) => format!("NOT saved ({e})"),
                    };
                    let _ = agent
                        .lock()
                        .await
                        .emit(AgentEvent::Info(format!(
                            "tool output -> {} ({saved})",
                            level.label()
                        )))
                        .await;
                }
                AgentCmd::SetContextBudget(chars) => {
                    let mut agent = agent.lock().await;
                    agent.set_context_budget_chars(chars);
                    // Through the setter: a `--context-length` from launch would otherwise win
                    // the next merge and quietly undo what was just typed.
                    task_config.set_context_budget(chars);
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
                    task_config.set_max_output_tokens(tokens);
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
                    let refusals = apply_mode(
                        &mut agent,
                        &task_config,
                        mode,
                        plan_permission.clone(),
                        tui_permission.clone(),
                    )
                    .await;
                    emit_refusals(&mut agent, refusals).await;
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
                    agent.replace_session(fresh.clone());
                    let refusals = apply_mode(
                        &mut agent,
                        &task_config,
                        SessionMode::Agent,
                        plan_permission.clone(),
                        tui_permission.clone(),
                    )
                    .await;
                    emit_refusals(&mut agent, refusals).await;
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
                        let refusals = apply_mode(
                            &mut agent,
                            &task_config,
                            mode,
                            plan_permission.clone(),
                            tui_permission.clone(),
                        )
                        .await;
                        emit_refusals(&mut agent, refusals).await;
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
                AgentCmd::UndoBefore { seq } => {
                    let mut agent = agent.lock().await;
                    match agent.undo_to_before(seq).await {
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
                AgentCmd::Undo { turns } => {
                    let mut agent = agent.lock().await;
                    match agent.undo_turns(turns).await {
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
                AgentCmd::Ledger { export: None } => {
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
                AgentCmd::Ledger { export: Some(dest) } => {
                    // The ledger's paths are relative to the session's cwd, and the export
                    // resolves them against it. Both are read under one lock and the file
                    // is written outside it: the write is the slow part and the agent lock
                    // is what a cancel needs.
                    let (diff, truncated) = {
                        let agent = agent.lock().await;
                        agent.export_ledger(std::path::Path::new(&agent.session().cwd))
                    };
                    let lines = match std::fs::write(&dest, &diff) {
                        Ok(()) if diff.is_empty() => vec![
                            "No committed edits in this session yet — nothing to export"
                                .to_string(),
                        ],
                        Ok(()) => {
                            let mut lines = vec![format!(
                                "Exported {} byte(s) of changes to {}",
                                diff.len(),
                                dest.display()
                            )];
                            if !truncated.is_empty() {
                                // The warning the core API exists to make possible: a
                                // truncated patch applies PARTIALLY and does not fail.
                                lines.push(format!(
                                    "⚠ {} file(s) had their hunks capped, so applying this patch applies a PARTIAL change: {}",
                                    truncated.len(),
                                    truncated
                                        .iter()
                                        .map(|p| p.display().to_string())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ));
                            }
                            lines
                        }
                        Err(e) => vec![format!("Could not write {}: {e}", dest.display())],
                    };
                    let agent = agent.lock().await;
                    for line in lines {
                        agent.emit(AgentEvent::Info(line)).await;
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

/// The deterministic rules over a path, as transcript lines.
///
/// Synchronous file reads inside a spawn: bounded by the file limit the rules module
/// applies, and a rules pass over this repository's own `crates/` takes milliseconds —
/// unlike the model-backed commands, there is nothing here to wait for.
fn review_path_lines(path: &str) -> Vec<String> {
    use firment_tools::review::rules;
    let root = std::path::Path::new(path);
    if !root.exists() {
        return vec![format!("Nothing at {path}.")];
    }
    // The walk is the shared one (which files count as source is one decision, not one
    // per surface) and its limit is already applied; this only takes the list.
    let mut files = firment_tools::review::walk::collect(root).files;
    files.truncate(firment_tools::review::walk::FILE_LIMIT);

    let mut report = firment_core::review::ReviewReport::new(format!("static: {path}"));
    report.detail(format!("{} file(s) read", files.len()));
    let mut skipped_tests = 0usize;
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let label = firment_tools::review::walk::label_for(root, file);
        let reviewed = rules::review_source(&label, &text);
        skipped_tests += reviewed.skipped_test_lines;
        for finding in reviewed.findings {
            report.push(finding);
        }
    }
    if skipped_tests > 0 {
        report.detail(format!("{skipped_tests} test-module line(s) skipped"));
    }
    report.note("rules only — this pass has not asked a model to look");
    review_lines(&report)
}

/// The review report as transcript lines.
///
/// The rule lives in core (`review::self_review::report_lines`) because the automatic
/// trigger reports through the same shape; this is the TUI's name for it.
fn review_lines(report: &firment_core::review::ReviewReport) -> Vec<String> {
    firment_core::review::self_review::report_lines(report)
}

#[derive(Debug)]
pub(crate) enum AgentCmd {
    User(String),
    /// Re-send the last request, discarding what the turn recorded after it. Resolves
    /// inside the loop into a `User` with the recovered prompt.
    RetryLast,
    /// Review the newest change in this session (plan §4-A's `/review-last`).
    ReviewLast,
    /// Review a path with the built-in rules (plan §4-C's `/review [path]`). No provider:
    /// this half costs nothing and works with nothing configured.
    ReviewPath {
        path: String,
    },
    Cancel,
    SetModel(String),
    SetThinking(ThinkingLevel),
    SetContextBudget(usize),
    SetMaxOutputTokens(u32),
    SetToolVerbosity(ToolVerbosity),
    ShowContext,
    DeleteSession(String),
    SetMode(SessionMode),
    OpenModelPicker,
    OpenSessionPicker,
    NewSession,
    LoadSession(String),
    Undo {
        /// How many committed turns to walk back. `/undo` is one; `/undo 3` is three.
        turns: usize,
    },
    /// `/undo --before <seq>`: walk back past the turn that contained that tool call. What the
    /// review cards need — the seq is printed on the card (`#12`), so a finding's own step is
    /// something the user can name.
    UndoBefore {
        seq: u64,
    },
    Ledger {
        /// Where to write the changes as a unified diff (plan §8's `/ledger --export`).
        /// `None` reads the summary instead.
        export: Option<std::path::PathBuf>,
    },
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
