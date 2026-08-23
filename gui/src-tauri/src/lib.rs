mod agent_core;
mod collab;
mod commands;
mod events;
mod hardware;
mod state;
mod ui;
mod workbench;

use std::sync::Arc;

use firment_core::Session;
use firment_core::SessionStore;
use state::Shared;
use tauri::Manager;

/// Build the initial session (latest saved, or a fresh one) so the app opens
/// with something loadable.
fn initial_session(config: &firment_core::Config) -> Session {
    let store = SessionStore::new(firment_core::config::config_dir().join("sessions"));
    if let Some(summary) = store.latest().ok().flatten() {
        if let Ok(session) = store.load(&summary.id) {
            return session;
        }
    }
    let (provider, model) = {
        let provider = config.default_provider.clone();
        let model = config
            .providers
            .get(&provider)
            .map(|p| p.model.clone())
            .unwrap_or_default();
        (provider, model)
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    Session::new(cwd, provider, model)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = firment_core::config::config_path();
            let config = firment_core::Config::load_or_create(&config_path)?;
            let session = initial_session(&config);

            let app_handle = app.handle().clone();
            let collab: Arc<dyn collab::CollabBackend> = Arc::new(collab::NoopBackend);
            let shared = Arc::new(Shared {
                app: app_handle,
                config_path,
                config: Arc::new(std::sync::Mutex::new(config)),
                store: Arc::new(std::sync::Mutex::new(SessionStore::new(
                    firment_core::config::config_dir().join("sessions"),
                ))),
                registry: firment_tools::default_registry(),
                agent: Arc::new(tokio::sync::Mutex::new(None)),
                cancel: Arc::new(std::sync::Mutex::new(None)),
                perm_waiters: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                ask_waiters: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                collab,
                running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                monitors: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            });

            let agent = match agent_core::build_agent(&shared, session) {
                Ok(agent) => Some(agent),
                Err(e) => {
                    eprintln!("failed to build agent: {e}");
                    None
                }
            };
            *shared.agent.blocking_lock() = agent;

            app.manage(shared);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_turn,
            commands::cancel_turn,
            commands::list_sessions,
            commands::new_session,
            commands::load_session,
            commands::delete_session,
            commands::session_transcript,
            commands::respond_permission,
            commands::respond_ask,
            commands::fetch_models,
            commands::set_api_key,
            commands::get_settings,
            commands::save_settings,
            commands::set_provider,
            commands::remove_provider,
            commands::list_ports,
            commands::monitor_start,
            commands::monitor_stop,
            commands::monitor_send,
            commands::active_monitors,
            commands::flash,
            commands::firm_run,
            workbench::workbench_state,
            workbench::workbench_set_mainline,
            workbench::workbench_branch_create,
            workbench::workbench_elf,
            workbench::workbench_quality,
            workbench::workbench_timeline,
        ]);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
