//! Export commands (plan section 2.8): `export_info` for the pane, `prepare_export` (checks, folder
//! dialog, plan), `start_export`, `cancel_export`, `get_export` / `watch_export`. The `do_*`
//! functions hold the logic and take the managed state as parameters, so tests can call them.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::export_job::{
    ExportEvent, ExportPlan, ExportSnapshot, ExportState, PlanInput, Sink, TargetReq, BUSY_MESSAGE, STALE_MESSAGE,
};
use crate::export_rules::{self, ExportKind, NO_TEMPO_REASON};
use crate::ipc::RepoState;
use crate::picker::{DialogKind, FolderReq};
use crate::repository::{Repository, TrackRecord};
use crate::settings::SettingsStore;
use crate::tempo_store::TempoStore;

/// The first section's count-in, as shown in the pane.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CountInInfo {
    pub bpm: f64,
    pub first_beat_ms: f64,
    /// 4 beats at `bpm`.
    pub lead_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportInfo {
    pub id: String,
    /// Whether the track can be exported at all (a backing track and a tempo map).
    pub available: bool,
    /// Why not (shown in the pane).
    pub reason: Option<String>,
    /// The file that would be exported, relative to the track folder.
    pub source: Option<String>,
    pub count_in: Option<CountInInfo>,
    /// The suggested RC-5 memory.
    pub memory: u8,
}

/// The record of `id` and its folder, read under the repository lock.
fn load(repo: &RepoState, id: &str) -> Result<(TrackRecord, PathBuf), String> {
    let root = repo.lock();
    let r = Repository::new(root.clone());
    let rec = r.load_record(id)?;
    Ok((rec, r.tracks_dir().join(id)))
}

pub fn do_export_info(
    repo: &RepoState,
    settings: &SettingsStore,
    store: &dyn TempoStore,
    id: &str,
) -> Result<ExportInfo, String> {
    let (rec, dir) = load(repo, id)?;
    let memory = export_rules::suggest_memory(settings.get().rc5_memory);
    let mut info =
        ExportInfo { id: id.to_string(), available: false, reason: None, source: None, count_in: None, memory };
    let src = match export_rules::export_source(&rec) {
        Ok(s) => s,
        Err(reason) => {
            info.reason = Some(reason);
            return Ok(info);
        }
    };
    info.source = Some(src.file);
    let tempo = store.load(id, &dir)?;
    match tempo.segments.first() {
        Some(seg) if seg.bpm > 0.0 => {
            info.available = true;
            info.count_in = Some(CountInInfo {
                bpm: seg.bpm,
                first_beat_ms: seg.start_ms,
                lead_ms: 4.0 * 60000.0 / seg.bpm,
            });
        }
        _ => info.reason = Some(NO_TEMPO_REASON.into()),
    }
    Ok(info)
}

fn refused<T>(kind: ExportKind, reason: &str, message: impl Into<String>) -> Result<T, String> {
    eprintln!("calliope: export refused target={} reason={reason}", kind.as_str());
    Err(message.into())
}

/// Everything that can be checked without a folder, then the folder dialog (starting in the
/// remembered folder, else `home`), then `ExportState::plan`. `Ok(None)` = the dialog was
/// cancelled. The picked folder is remembered once it passed the checks.
pub fn do_prepare_export(
    repo: &RepoState,
    settings: &SettingsStore,
    export: &ExportState,
    store: &dyn TempoStore,
    home: Option<PathBuf>,
    id: &str,
    target: TargetReq,
) -> Result<Option<ExportPlan>, String> {
    let kind = target.kind();
    if export.is_running() {
        return refused(kind, "busy", BUSY_MESSAGE);
    }
    let memory = match &target {
        TargetReq::Rc5 { memory } => match export_rules::parse_memory(memory) {
            Ok(n) => Some(n),
            Err(e) => return refused(kind, "memory", e),
        },
        _ => None,
    };
    export.require(kind)?;
    let (rec, dir) = load(repo, id)?;
    let src = match export_rules::export_source(&rec) {
        Ok(s) => s,
        Err(e) => return refused(kind, "no-backing", e),
    };
    let tempo = store.load(id, &dir)?;
    if tempo.is_empty() {
        return refused(kind, "no-tempo", NO_TEMPO_REASON);
    }

    let (dialog, title) = match kind {
        ExportKind::Rc5 => (DialogKind::ExportRc5, "Choose the RC-5 drive (its root folder)"),
        _ => (DialogKind::ExportAudio, "Choose a folder for the MP3/OGG file"),
    };
    let s = settings.get();
    let remembered = if kind == ExportKind::Rc5 { s.rc5_dir } else { s.export_dir };
    let (from, start_dir) = export_rules::start_dir(remembered.as_deref(), home.as_deref());
    eprintln!(
        "calliope: dialog start kind={} from={from} dir={}",
        dialog.as_str(),
        start_dir.as_ref().map_or("none".to_string(), |d| d.display().to_string())
    );
    let Some(chosen) = repo.picker.pick_folder(&FolderReq { kind: dialog, title: title.into(), start_dir }) else {
        return Ok(None);
    };

    let root = repo.current_root();
    let plan = export.plan(PlanInput {
        track: &rec,
        track_dir: dir.clone(),
        source: dir.join(&src.file),
        kind,
        memory,
        chosen: &chosen,
        repo_root: &root,
        tempo: &tempo,
    })?;
    if chosen.is_absolute() {
        let saved = if kind == ExportKind::Rc5 {
            settings.set_rc5_dir(Some(chosen.clone()))
        } else {
            settings.set_export_dir(Some(chosen.clone()))
        };
        match saved {
            Ok(_) => eprintln!(
                "calliope: export remembered kind={} dir={}",
                if kind == ExportKind::Rc5 { "rc5" } else { "file" },
                chosen.display()
            ),
            Err(e) => eprintln!("calliope: could not save settings: {e}"),
        }
    }
    Ok(Some(plan))
}

/// Saves the RC-5 memory of a finished export (a save error is only logged).
pub fn remember_memory(settings: &SettingsStore, n: u8) {
    match settings.set_rc5_memory(Some(n)) {
        Ok(_) => eprintln!("calliope: export remembered memory={n}"),
        Err(e) => eprintln!("calliope: could not save settings: {e}"),
    }
}

/// Starts the planned export with the tempo map as it is now (read from the planned track's directory). `remember` runs with the RC-5
/// memory when an RC-5 export is done (before the `done` event).
pub fn do_start_export(
    _repo: &RepoState,
    export: &ExportState,
    store: &dyn TempoStore,
    token: &str,
    overwrite: bool,
    sink: Sink,
    remember: Box<dyn FnOnce(u8) + Send>,
) -> Result<ExportSnapshot, String> {
    let Some((id, dir, kind, memory)) = export.planned(token) else {
        eprintln!("calliope: export refused target=none reason=stale");
        return Err(STALE_MESSAGE.into());
    };
    // The track directory is the one the plan was made for, not the current repository root.
    let tempo = store.load(&id, &dir)?;
    let on_done: Box<dyn FnOnce() + Send> = match (kind, memory) {
        (ExportKind::Rc5, Some(n)) => Box::new(move || remember(n)),
        _ => Box::new(|| {}),
    };
    export.start(token, overwrite, &tempo, sink, on_done)
}

// ---- Tauri commands ----

fn channel_sink(events: tauri::ipc::Channel<ExportEvent>) -> Sink {
    Arc::new(move |ev| {
        let _ = events.send(ev);
    })
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

#[tauri::command]
pub async fn export_info(app: tauri::AppHandle, id: String) -> Result<ExportInfo, String> {
    blocking(move || {
        do_export_info(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            &id,
        )
    })
    .await
}

#[tauri::command]
pub async fn prepare_export(
    app: tauri::AppHandle,
    id: String,
    target: TargetReq,
) -> Result<Option<ExportPlan>, String> {
    blocking(move || {
        do_prepare_export(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &app.state::<ExportState>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            app.path().home_dir().ok(),
            &id,
            target,
        )
    })
    .await
}

#[tauri::command]
pub async fn start_export(
    app: tauri::AppHandle,
    token: String,
    overwrite: bool,
    events: tauri::ipc::Channel<ExportEvent>,
) -> Result<ExportSnapshot, String> {
    blocking(move || {
        let handle = app.clone();
        do_start_export(
            &app.state::<RepoState>(),
            &app.state::<ExportState>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            &token,
            overwrite,
            channel_sink(events),
            Box::new(move |n| remember_memory(&handle.state::<SettingsStore>(), n)),
        )
    })
    .await
}

#[tauri::command]
pub async fn cancel_export(app: tauri::AppHandle, job: String) -> Result<Option<ExportSnapshot>, String> {
    Ok(app.state::<ExportState>().cancel(&job))
}

#[tauri::command]
pub async fn get_export(app: tauri::AppHandle) -> Result<Option<ExportSnapshot>, String> {
    Ok(app.state::<ExportState>().snapshot())
}

#[tauri::command]
pub async fn watch_export(
    app: tauri::AppHandle,
    events: tauri::ipc::Channel<ExportEvent>,
) -> Result<Option<ExportSnapshot>, String> {
    Ok(app.state::<ExportState>().watch(channel_sink(events)))
}

#[cfg(test)]
#[path = "export_ipc_tests.rs"]
mod tests;
