use std::sync::atomic::Ordering;
use std::sync::Arc;

use firment_core::Session;
use firment_core::SessionMode;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::agent_core::{build_agent, default_provider_model};
use crate::events::{session_dto, session_summary_dto, FrontendEvent};
use crate::hardware;
use crate::state::Shared;

// ---------- session lifecycle ----------

#[tauri::command]
pub async fn start_turn(
    shared: tauri::State<'_, Arc<Shared>>,
    input: String,
) -> Result<(), String> {
    let shared = shared.inner().clone();
    if shared.running.load(Ordering::SeqCst) {
        return Err("agent is already running - cancel it first".to_string());
    }
    shared.running.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let result = {
            let mut guard = shared.agent.lock().await;
            match guard.as_mut() {
                Some(agent) => {
                    // A previous cancel leaves the watch channel armed (true),
                    // which would make this new turn abort instantly at the
                    // first checkpoint. Clear it before every turn.
                    agent.reset_cancel();
                    agent.run_turn(&input).await
                }
                None => Err(firment_core::AgentError::NoProvider),
            }
        };
        shared.running.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            let _ = shared
                .app
                .emit("agent-event", FrontendEvent::Error { message: e.to_string() });
            // No TurnEnd on error: the frontend's error handler already
            // resets `running` and keeps the error text visible in the turn
            // until the next turn_start. Success paths emit TurnEnd inside
            // run_turn, so we must not emit a duplicate here either.
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn cancel_turn(shared: tauri::State<'_, Arc<Shared>>) -> Result<(), String> {
    let shared = shared.inner().clone();
    // Must NOT go through the agent lock: run_turn holds it for the whole
    // turn, so a cancel waiting on the lock would block until the turn
    // finishes and never take effect. Fire the pre-extracted handles instead.
    let handles = shared.cancel.lock().unwrap().clone();
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
    let mut session = Session::new(
        std::path::PathBuf::from(&cwd),
        provider,
        model.clone(),
    );
    session.mode = session_mode;
    let store = shared.store.lock().unwrap().clone();
    store.save(&session).map_err(|e| e.to_string())?;
    let agent = build_agent(&shared, session.clone()).map_err(|e| e.to_string())?;
    *shared.agent.lock().await = Some(agent);
    let dto = session_dto(&session);
    let _ = shared.app.emit("agent-event", FrontendEvent::SessionLoaded {
        session: dto.clone(),
    });
    Ok(dto)
}

#[tauri::command]
pub async fn load_session(
    shared: tauri::State<'_, Arc<Shared>>,
    id: String,
) -> Result<crate::events::SessionDto, String> {
    let shared = shared.inner().clone();
    let store = shared.store.lock().unwrap().clone();
    let session = store.load(&id).map_err(|e| e.to_string())?;
    let agent = build_agent(&shared, session.clone()).map_err(|e| e.to_string())?;
    *shared.agent.lock().await = Some(agent);
    let dto = session_dto(&session);
    let _ = shared.app.emit("agent-event", FrontendEvent::SessionLoaded {
        session: dto.clone(),
    });
    Ok(dto)
}

#[tauri::command]
pub async fn delete_session(
    shared: tauri::State<'_, Arc<Shared>>,
    id: String,
) -> Result<(), String> {
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
    config.list_models(&provider).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_api_key(
    shared: tauri::State<'_, Arc<Shared>>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let config = shared.config.lock().unwrap();
    config.set_api_key(&provider, &key).map_err(|e| e.to_string())
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
}

#[tauri::command]
pub async fn get_settings(
    shared: tauri::State<'_, Arc<Shared>>,
) -> Result<SettingsDto, String> {
    let config = shared.config.lock().unwrap().clone();
    let (_, model) = default_provider_model(&config);
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
    })
}

#[tauri::command]
pub async fn save_settings(
    shared: tauri::State<'_, Arc<Shared>>,
    settings: SettingsDto,
) -> Result<(), String> {
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
    Ok(())
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
pub async fn active_monitors(
    shared: tauri::State<'_, Arc<Shared>>,
) -> Result<Vec<String>, String> {
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
    // firm flash <FILE> --chip <CHIP> [--probe <PROBE>]
    // NOTE: file is a POSITIONAL arg in the firm CLI (not --file), so it must
    // not be passed as a named option.
    let mut args = vec!["flash".to_string(), file];
    if let Some(chip) = chip {
        args.push("--chip".to_string());
        args.push(chip);
    }
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe);
    }
    hardware::run_hardware_command(shared.inner().clone(), "flash".to_string(), args, cwd, 120).await
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
    // firm run <FILE> --chip <CHIP> [--probe <PROBE>] --timeout <SECS>
    // NOTE: file is a POSITIONAL arg in the firm CLI (not --file).
    let mut args = vec!["run".to_string(), file];
    if let Some(chip) = chip {
        args.push("--chip".to_string());
        args.push(chip);
    }
    if let Some(probe) = probe {
        args.push("--probe".to_string());
        args.push(probe);
    }
    args.push("--timeout".to_string());
    args.push(timeout_secs.to_string());
    hardware::run_hardware_command(shared.inner().clone(), "run".to_string(), args, cwd, timeout_secs.max(60)).await
}