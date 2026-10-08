//! Tauri shell. The only module that touches Tauri/GTK.

use crate::{
    import_job::ImportState,
    ipc::{self, RepoState},
    picker::{Picker, TauriPicker},
    settings::{self, SettingsStore},
    tools::Tools,
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
            let url = store.get().edge_ai_url;
            app.manage(store);
            let repo = RepoState::new(root, choose_picker(app.handle()));
            let discover = || Tools::discover(&std::env::var_os("PATH").unwrap_or_default());
            app.manage(ImportState::new(discover(), repo.repo_lock(), ipc::IMPORT_POLL).with_rediscovery(discover));
            app.manage(repo);
            let backend = crate::audio_out::select_backend();
            app.manage(ipc::EditorState::new(backend));
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(ipc::EDITOR_TICK);
                handle.state::<ipc::EditorState>().tick(std::time::Instant::now());
            });
            // Start-up health check of the edge-AI server (only when one is configured).
            if url.is_some() {
                std::thread::spawn(move || ipc::startup_health_check(url));
            }
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
            ipc::export_tablature,
            ipc::set_edge_ai_url,
            ipc::set_keep_original,
            ipc::check_edge_ai,
            ipc::check_tools,
            ipc::prepare_url_import,
            ipc::start_url_import,
            ipc::import_file,
            ipc::start_stem_extraction,
            ipc::cancel_import,
            ipc::discard_import,
            ipc::get_import_job,
            ipc::watch_import,
            ipc::open_editor,
            ipc::close_editor,
            ipc::get_editor,
            ipc::watch_editor,
            ipc::editor_play,
            ipc::editor_lane_play,
            ipc::editor_end_solo,
            ipc::editor_pause,
            ipc::editor_stop,
            ipc::editor_seek,
            ipc::editor_nudge,
            ipc::editor_set_stem,
            ipc::save_backing,
            ipc::cancel_backing_save
        ])
        .build(tauri::generate_context!())
        .expect("error while building calliope-gui")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(editor) = app.try_state::<ipc::EditorState>() {
                    editor.shutdown(ipc::SHUTDOWN_WAIT);
                }
                // Stop a running import (child processes, edge-AI job) before the process ends.
                if let Some(import) = app.try_state::<ImportState>() {
                    if !import.shutdown(ipc::SHUTDOWN_WAIT) {
                        eprintln!("calliope: import did not stop within the exit grace period");
                    }
                }
            }
        });
}
