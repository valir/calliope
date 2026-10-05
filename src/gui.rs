//! Tauri shell. The only module that touches Tauri/GTK.

use crate::{
    ipc::{self, RepoState},
    picker::{Picker, TauriPicker},
    settings::{self, SettingsStore},
};
use tauri::Manager;

/// `ScriptedPicker` only when built with `e2e-hooks` and the answers env var is set.
#[cfg(feature = "e2e-hooks")]
fn choose_picker(app: &tauri::AppHandle) -> Box<dyn Picker> {
    match crate::picker::ScriptedPicker::from_env() {
        Some(p) => Box::new(p),
        None => Box::new(TauriPicker::new(app.clone())),
    }
}

#[cfg(not(feature = "e2e-hooks"))]
fn choose_picker(app: &tauri::AppHandle) -> Box<dyn Picker> {
    Box::new(TauriPicker::new(app.clone()))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            let store = SettingsStore::open(path);
            let root = store
                .get()
                .repository_root
                .unwrap_or_else(|| settings::default_repository_root(&app.path().data_dir().expect("data dir")));
            app.manage(store);
            app.manage(RepoState::new(root, choose_picker(app.handle())));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_version,
            ipc::get_settings,
            ipc::set_theme,
            ipc::frontend_log,
            ipc::get_repository,
            ipc::choose_repository_root,
            ipc::set_repository_root,
            ipc::reset_repository_root,
            ipc::list_tracks,
            ipc::pick_tablature,
            ipc::save_track,
            ipc::delete_track,
            ipc::export_track,
            ipc::export_tablature
        ])
        .run(tauri::generate_context!())
        .expect("error while running calliope-gui");
}
