mod agent_core;
mod collab;
mod commands;
mod events;
mod hardware;
mod state;
mod ui;
mod workbench;

use std::sync::Arc;

use firment_core::SessionStore;
use state::Shared;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_path = firment_core::config::config_path();
            let config = firment_core::Config::load_or_create(&config_path)?;

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
                // No global agent: every start_turn builds a per-session
                // agent on demand (parallel chats), so startup has nothing
                // to pre-build.
                agents: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                perm_waiters: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                ask_waiters: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
                collab,
                monitors: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            });

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
