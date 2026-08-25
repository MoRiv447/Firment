use std::sync::atomic::Ordering;
use std::sync::Arc;

use firment_core::Session;
use firment_core::SessionMode;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::agent_core::{build_agent, default_provider_model};
use crate::events::{session_dto, session_summary_dto, FrontendEvent};
use crate::hardware;
use crate::state::{AgentSlot, Shared};

// ---------- session lifecycle ----------

/// Clears a slot's running flag when the owning task is dropped — including
/// on panic. Without this, one panic inside the turn task leaves running=true
/// forever and the session refuses every future turn until app restart.
struct RunningGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn start_turn(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
    input: String,
) -> Result<(), String> {
    let shared = shared.inner().clone();
    // Reserve the slot FIRST (under the map lock) so a concurrent
    // start_turn AND a concurrent delete_session both observe the
    // reservation atomically; the agent is built after the reservation and
    // the slot released if the build fails.
    let slot = {
        let mut map = shared.agents.lock().unwrap();
        let slot = map.entry(session_id.clone()).or_insert_with(AgentSlot::new);
        if slot.running.swap(true, Ordering::SeqCst) {
            return Err("this session already has a turn running - cancel it first".to_string());
        }
        slot.clone()
    };
    let _reservation = RunningGuard(slot.running.clone());

    // Build the agent fresh from the CURRENT session snapshot and config:
    // settings/provider changes therefore apply on the very next turn
    // without any explicit reload step.
    let build = (|| -> Result<(firment_core::Agent, crate::state::CancelHandles), String> {
        let store = shared
            .store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let session = store.load(&session_id).map_err(|e| e.to_string())?;
        let budget = session.context_budget_chars;
        let (mut agent, handles) =
            build_agent(&shared, session).map_err(|e| e.to_string())?;
        if budget > 0 {
            agent.set_context_budget_chars(budget);
        }
        Ok((agent, handles))
    })();
    let (mut agent, handles) = match build {
        Ok(v) => v,
        Err(e) => {
            drop(_reservation); // release before returning
            return Err(e);
        }
    };

    {
        let map = shared.agents.lock().unwrap();
        if let Some(s) = map.get(&session_id) {
            *s.cancel.lock().unwrap() = Some(handles);
        }
    }
    tauri::async_runtime::spawn(async move {
        let mut agent = agent;
        let result = agent.run_turn(&input).await;
        drop(agent);
        drop(_reservation); // clears running on success AND on panic unwind
        if let Err(e) = result {
            let _ = shared.app.emit(
                "agent-event",
                FrontendEvent::Error {
                    session_id: Some(session_id),
                    message: e.to_string(),
                },
            );
            // No TurnEnd on error: the frontend's error handler already
            // resets `running` and keeps the error text visible in the turn
            // until the next turn_start. Success paths emit TurnEnd inside
            // run_turn, so we must not emit a duplicate here either.
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_turn(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
) -> Result<(), String> {
    let shared = shared.inner().clone();
    // Must NOT go through the agent lock: run_turn holds it for the whole
    // turn, so a cancel waiting on the lock would block until the turn
    // finishes and never take effect. Fire the pre-extracted handles instead.
    let handles = {
        let map = shared.agents.lock().unwrap();
        map.get(&session_id)
            .and_then(|slot| slot.cancel.lock().unwrap().clone())
    };
    if let Some((tx, signal)) = handles {
        let _ = tx.send(true);
        signal.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn list_sessions(
    shared: tauri::State<'_, Arc<Shared>>,
) -> Result<Vec<crate::events::SessionSummaryDto>, String> {
    let store = shared.store.lock().unwrap();
    let sessions = store.list().map_err(|e| e.to_string())?;
    Ok(sessions.iter().map(session_summary_dto).collect())
}

#[tauri::command]
pub async fn new_session(
    shared: tauri::State<'_, Arc<Shared>>,
    cwd: String,
    mode: String,
) -> Result<crate::events::SessionDto, String> {
    let shared = shared.inner().clone();
    let (provider, model) = {
        let config = shared.config.lock().unwrap().clone();
        default_provider_model(&config)
    };
    let session_mode = if mode.eq_ignore_ascii_case("plan") {
        SessionMode::Plan
    } else {
        SessionMode::Agent
    };
    let mut session = Session::new(std::path::PathBuf::from(&cwd), provider, model.clone());
    session.mode = session_mode;
    let store = shared.store.lock().unwrap().clone();
    store.save(&session).map_err(|e| e.to_string())?;
    // No global agent to swap anymore: each start_turn builds its own agent,
    // so creating a session never disturbs turns running in other chats.
    let dto = session_dto(&session);
    let _ = shared.app.emit(
        "agent-event",
        FrontendEvent::SessionLoaded {
            session: dto.clone(),
        },
    );
    Ok(dto)
}

#[tauri::command]
pub async fn load_session(
    shared: tauri::State<'_, Arc<Shared>>,
    id: String,
) -> Result<crate::events::SessionDto, String> {
    let shared = shared.inner().clone();
    // Switching chats never disturbs turns running elsewhere — parallel
    // sessions each own their agent, so there is nothing global to guard.
    let store = shared.store.lock().unwrap().clone();
    let session = store.load(&id).map_err(|e| e.to_string())?;
    let dto = session_dto(&session);
    let _ = shared.app.emit(
        "agent-event",
        FrontendEvent::SessionLoaded {
            session: dto.clone(),
        },
    );
    Ok(dto)
}

#[tauri::command]
pub async fn delete_session(
    shared: tauri::State<'_, Arc<Shared>>,
    id: String,
) -> Result<(), String> {
    // Deleting while a turn is running would let run_turn's final save
    // resurrect the session file — refuse until the turn ends or is cancelled.
    {
        let map = shared.agents.lock().unwrap();
        if let Some(slot) = map.get(&id) {
            if slot.running.load(Ordering::SeqCst) {
                return Err(
                    "cannot delete: a turn is running in this session - cancel it first"
                        .to_string(),
                );
            }
        }
    }
    let store = shared.store.lock().unwrap();
    store.delete(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_transcript(
    shared: tauri::State<'_, Arc<Shared>>,
    id: String,
) -> Result<crate::events::SessionDto, String> {
    let store = shared.store.lock().unwrap().clone();
    let session = store.load(&id).map_err(|e| e.to_string())?;
    Ok(session_dto(&session))
}

// ---------- per-session chat knobs (thinking / mode / context budget) ----
// The GUI rebuilds the agent from the saved session on every start_turn,
// so persisting these to the session file is all it takes for the change
// to apply from the next message onwards.

#[tauri::command]
/// Guard shared by the per-session knob commands: a running turn holds its
/// own session snapshot and saves it on EVERY exit path, so a knob written
/// mid-turn would be silently reverted when the turn finishes. Refuse
/// instead — the UI disables the chips, this is the backend backstop.
fn ensure_not_running(shared: &Shared, session_id: &str) -> Result<(), String> {
    let map = shared.agents.lock().unwrap();
    if let Some(slot) = map.get(session_id) {
        if slot.running.load(Ordering::SeqCst) {
            return Err("a turn is running in this session — the change would be overwritten; try again after it finishes".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_session_thinking(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
    level: String,
) -> Result<crate::events::SessionDto, String> {
    ensure_not_running(&shared, &session_id)?;
    let level = level
        .parse::<firment_core::ThinkingLevel>()
        .map_err(|e: std::io::Error| e.to_string())?;
    let shared = shared.inner().clone();
    let store = shared.store.lock().unwrap().clone();
    let mut session = store.load(&session_id).map_err(|e| e.to_string())?;
    session.thinking = level;
    store.save(&session).map_err(|e| e.to_string())?;
    Ok(session_dto(&session))
}

#[tauri::command]
pub async fn set_session_mode(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
    mode: String,
) -> Result<crate::events::SessionDto, String> {
    ensure_not_running(&shared, &session_id)?;
    let mode = match mode.to_ascii_lowercase().as_str() {
        "agent" => SessionMode::Agent,
        "plan" => SessionMode::Plan,
        other => return Err(format!("invalid mode '{other}' (expected agent|plan)")),
    };
    let shared = shared.inner().clone();
    let store = shared.store.lock().unwrap().clone();
    let mut session = store.load(&session_id).map_err(|e| e.to_string())?;
    session.mode = mode;
    store.save(&session).map_err(|e| e.to_string())?;
    Ok(session_dto(&session))
}

#[tauri::command]
pub async fn set_session_budget(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
    chars: usize,
) -> Result<crate::events::SessionDto, String> {
    ensure_not_running(&shared, &session_id)?;
    if chars != 0 && !(16_384..=4_194_304).contains(&chars) {
        return Err("budget must be 0 (default) or between 16384 and 4194304 chars".to_string());
    }
    let shared = shared.inner().clone();
    let store = shared.store.lock().unwrap().clone();
    let mut session = store.load(&session_id).map_err(|e| e.to_string())?;
    session.context_budget_chars = chars;
    store.save(&session).map_err(|e| e.to_string())?;
    Ok(session_dto(&session))
}

#[derive(Serialize)]
pub struct ContextUsageDto {
    pub system_chars: u64,
    pub messages_chars: u64,
    /// Budget in effect (session override or the agent default).
    pub budget: u64,
    pub total_chars: u64,
    pub pct: f64,
}

/// Last known MQTT link status (the GuardStatus JSON the card renders).
/// Pull-on-mount companion to the push events: anything emitted before the
/// webview attached its listeners is recoverable here.
#[tauri::command]
pub async fn mqtt_status(shared: tauri::State<'_, Arc<Shared>>) -> Result<String, String> {
    Ok(shared.mqtt_status.lock().unwrap().clone())
}

/// Rough context usage matching the TUI's `/context` estimate: system
/// prompt + transcript chars against the compaction budget. Tool-schema
/// chars are omitted (registry-dependent); treat the number as a lower
/// bound that trends correctly.
#[tauri::command]
pub async fn session_context_usage(
    shared: tauri::State<'_, Arc<Shared>>,
    session_id: String,
) -> Result<ContextUsageDto, String> {
    const DEFAULT_BUDGET: u64 = 256 * 1024;
    let shared = shared.inner().clone();
    let store = shared.store.lock().unwrap().clone();
    let session = store.load(&session_id).map_err(|e| e.to_string())?;
    let system_chars = firment_core::context::system_prompt_for(&session.cwd, session.mode)
        .chars()
        .count() as u64;
    let messages_chars: u64 = session
        .messages
        .iter()
        .map(|m| match m {
            firment_core::types::ChatMessage::System { content }
            | firment_core::types::ChatMessage::User { content }
            | firment_core::types::ChatMessage::Assistant { content, .. }
            | firment_core::types::ChatMessage::Tool { content, .. } => content.chars().count(),
        })
        .sum::<usize>() as u64;
    let budget = if session.context_budget_chars > 0 {
        session.context_budget_chars as u64
    } else {
        DEFAULT_BUDGET
    };
    let total_chars = system_chars + messages_chars;
    Ok(ContextUsageDto {
        system_chars,
        messages_chars,
        budget,
        total_chars,
        pct: (total_chars as f64 / budget.max(1) as f64) * 100.0,
    })
}

// ---------- permission / ask responses ----------

#[tauri::command]
pub async fn respond_permission(
    shared: tauri::State<'_, Arc<Shared>>,
    id: u64,
    allowed: bool,
) -> Result<(), String> {
    if let Some(tx) = shared.perm_waiters.lock().unwrap().remove(&id) {
        let _ = tx.send(allowed);
    }
    Ok(())
}

#[tauri::command]
pub async fn respond_ask(
    shared: tauri::State<'_, Arc<Shared>>,
    id: u64,
    answer: Option<String>,
) -> Result<(), String> {
    if let Some(tx) = shared.ask_waiters.lock().unwrap().remove(&id) {
        let _ = tx.send(answer);
    }
    Ok(())
}

// ---------- models / settings ----------

#[tauri::command]
pub async fn fetch_models(
    shared: tauri::State<'_, Arc<Shared>>,
    provider: String,
) -> Result<Vec<String>, String> {
    let config = shared.config.lock().unwrap().clone();
    config
        .list_models(&provider)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_api_key(
    shared: tauri::State<'_, Arc<Shared>>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let config = shared.config.lock().unwrap();
    config
        .set_api_key(&provider, &key)
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
pub struct SettingsDto {
    pub default_provider: String,
    pub default_model: String,
    pub auto_approve: Vec<String>,
    pub max_iterations: usize,
    pub context_budget_chars: usize,
    pub build_command: Option<String>,
    pub default_chip: Option<String>,
    pub monitor_port: Option<String>,
    pub monitor_baud: u32,
    pub web_search: Option<String>,
    pub thinking: String,
    /// All configured providers (name + resolved connection info) so the IDE
    /// can edit base_url / model without hand-editing config.toml.
    /// Defaulted so save_settings from older clients (which don't round-trip
    /// the provider list) still deserialize.
    #[serde(default)]
    pub providers: Vec<ProviderEntryDto>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProviderEntryDto {
    pub name: String,
    pub r#type: String,
    pub base_url: Option<String>,
    pub model: String,
    pub is_default: bool,
    /// Resolved API key (from auth.json / inline / env). Sent back so the
    /// settings UI can show per-provider keys; it never leaves the local app.
    pub api_key: Option<String>,
}

#[tauri::command]
pub async fn get_settings(shared: tauri::State<'_, Arc<Shared>>) -> Result<SettingsDto, String> {
    let config = shared.config.lock().unwrap().clone();
    let (_, model) = default_provider_model(&config);
    let mut providers: Vec<ProviderEntryDto> = config
        .providers
        .iter()
        .map(|(name, p)| ProviderEntryDto {
            name: name.clone(),
            r#type: p.r#type.clone(),
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            is_default: name == &config.default_provider,
            api_key: config.api_key_for(p, name),
        })
        .collect();
    providers.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(SettingsDto {
        default_provider: config.default_provider.clone(),
        default_model: model,
        auto_approve: config.auto_approve.clone(),
        max_iterations: config.max_iterations,
        context_budget_chars: config.context_budget_chars,
        build_command: config.tools.build_command.clone(),
        default_chip: config.tools.default_chip.clone(),
        monitor_port: config.tools.monitor_port.clone(),
        monitor_baud: config.tools.monitor_baud,
        web_search: config.tools.web_search.clone(),
        thinking: config.thinking.label().to_string(),
        providers,
    })
}

#[tauri::command]
pub async fn save_settings(
    shared: tauri::State<'_, Arc<Shared>>,
    settings: SettingsDto,
) -> Result<(), String> {
    {
        let mut config = shared.config.lock().unwrap();
        config.default_provider = settings.default_provider.clone();
        if !settings.default_model.is_empty() {
            let provider_name = config.default_provider.clone();
            if let Some(p) = config.providers.get_mut(&provider_name) {
                p.model = settings.default_model.clone();
            }
        }
        config.auto_approve = settings.auto_approve;
        config.max_iterations = settings.max_iterations;
        config.context_budget_chars = settings.context_budget_chars;
        config.tools.build_command = settings.build_command;
        config.tools.default_chip = settings.default_chip;
        config.tools.monitor_port = settings.monitor_port;
        config.tools.monitor_baud = settings.monitor_baud;
        config.tools.web_search = settings.web_search;
        if let Ok(level) = settings.thinking.parse::<firment_core::ThinkingLevel>() {
            config.thinking = level;
        }
        config
            .save(&shared.config_path)
            .map_err(|e| e.to_string())?;
    }

    // No agent rebuild needed: start_turn builds a fresh agent from the
    // current session snapshot + config, so saved settings apply on the
    // very next turn in every chat automatically.
    Ok(())
}

/// Add or update a provider (type / base URL / model) in config.toml.
#[tauri::command]
pub async fn set_provider(
    shared: tauri::State<'_, Arc<Shared>>,
    name: String,
    provider_type: String,
    base_url: Option<String>,
    model: String,
) -> Result<(), String> {
    let mut config = shared.config.lock().unwrap();
    config
        .set_provider(&name, &provider_type, base_url, &model)
        .map_err(|e| e.to_string())
}

/// Remove a provider definition (except the default one).
#[tauri::command]
pub async fn remove_provider(
    shared: tauri::State<'_, Arc<Shared>>,
    name: String,
) -> Result<(), String> {
    let mut config = shared.config.lock().unwrap();
    config.remove_provider(&name).map_err(|e| e.to_string())
}

// ---------- hardware ----------

#[tauri::command]
pub async fn list_ports() -> Vec<String> {
    hardware::list_serial_ports()
}

#[tauri::command]
pub async fn monitor_start(
    shared: tauri::State<'_, Arc<Shared>>,
    port: String,
    baud: u32,
    elf: Option<String>,
) -> Result<(), String> {
    hardware::monitor_start(shared.inner().clone(), port, baud, elf).await
}

#[tauri::command]
pub async fn monitor_stop(
    shared: tauri::State<'_, Arc<Shared>>,
    port: String,
) -> Result<(), String> {
    hardware::monitor_stop(shared.inner().clone(), &port).await;
    Ok(())
}

#[tauri::command]
pub async fn monitor_send(
    shared: tauri::State<'_, Arc<Shared>>,
    port: String,
    data: String,
) -> Result<(), String> {
    hardware::monitor_send(shared.inner().clone(), &port, &data).await
}

#[tauri::command]
pub async fn active_monitors(shared: tauri::State<'_, Arc<Shared>>) -> Result<Vec<String>, String> {
    Ok(hardware::active_monitors(shared.inner()))
}

#[tauri::command]
pub async fn flash(
    shared: tauri::State<'_, Arc<Shared>>,
    file: String,
    chip: Option<String>,
    probe: Option<String>,
    cwd: Option<String>,
) -> Result<(), String> {
    hardware::run_hardware_command(
        shared.inner().clone(),
        "flash".to_string(),
        file,
        chip,
        probe,
        cwd,
        120,
    )
    .await
}

#[tauri::command]
pub async fn firm_run(
    shared: tauri::State<'_, Arc<Shared>>,
    file: String,
    chip: Option<String>,
    probe: Option<String>,
    cwd: Option<String>,
    timeout_secs: u64,
) -> Result<(), String> {
    // Wait at most as long as the UI's requested timeout (clamped to a
    // minimum so a 0/accidental value can't spawn a forever-wait).
    hardware::run_hardware_command(
        shared.inner().clone(),
        "run".to_string(),
        file,
        chip,
        probe,
        cwd,
        timeout_secs.max(5),
    )
    .await
}
