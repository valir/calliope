//! Tauri shell. The only module that touches Tauri/GTK.

use crate::{ipc, settings::SettingsStore};
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            app.manage(SettingsStore::open(path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_version,
            ipc::get_settings,
            ipc::set_theme,
            ipc::frontend_log
        ])
        .run(tauri::generate_context!())
        .expect("error while running calliope-gui");
}
