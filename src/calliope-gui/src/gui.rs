//! Tauri shell. The only module that touches Tauri/GTK.

use crate::{
    import_job::ImportState,
    ipc::{self, RepoState},
    picker::{Picker, TauriPicker},
    player_ipc::{self, PlayerHost, PlayerTempoStore},
    settings::{self, SettingsStore},
    tempo_store::TempoStore,
    tools::Tools,
};
use std::sync::Arc;
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
            // One backend for both engines: only one of them sounds at a time.
            let editor = ipc::EditorState::new(backend.clone());
            let h = app.handle().clone();
            editor.set_before_play(std::sync::Arc::new(move || h.state::<Arc<PlayerHost>>().pause_current()));
            app.manage(editor);
            let host = Arc::new(PlayerHost::new(backend, discover().ffmpeg));
            // Editor tempo writes go through the Player: flush on read, release on write.
            let store: Arc<dyn TempoStore> = Arc::new(PlayerTempoStore::new(host.clone()));
            app.manage(store);
            app.manage(host);
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(ipc::EDITOR_TICK);
                let now = std::time::Instant::now();
                handle.state::<ipc::EditorState>().tick(now);
                handle.state::<Arc<PlayerHost>>().tick(now);
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
            ipc::pick_import_file,
            ipc::import_path,
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
            ipc::editor_detect,
            ipc::editor_cancel_detect,
            ipc::editor_tempo,
            ipc::save_backing,
            ipc::cancel_backing_save,
            player_ipc::open_player,
            player_ipc::close_player,
            player_ipc::get_player,
            player_ipc::watch_player,
            player_ipc::player_select_source,
            player_ipc::player_play,
            player_ipc::player_pause,
            player_ipc::player_seek,
            player_ipc::player_nudge,
            player_ipc::player_skip,
            player_ipc::player_leave,
            player_ipc::player_tempo,
            player_ipc::player_loop
        ])
        .build(tauri::generate_context!())
        .expect("error while building calliope-gui")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(player) = app.try_state::<Arc<PlayerHost>>() {
                    player.shutdown(ipc::SHUTDOWN_WAIT);
                }
                if let Some(editor) = app.try_state::<ipc::EditorState>() {
                    editor.shutdown(ipc::SHUTDOWN_WAIT);
                    editor.with(|m| m.shutdown_detect(ipc::SHUTDOWN_WAIT));
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
