//! Export commands (plan section 2.8): `export_info` for the pane, `prepare_export` (checks, folder
//! dialog, plan), `start_export`, `cancel_export`, `get_export` / `watch_export`. The `do_*`
//! functions hold the logic and take the managed state as parameters, so tests can call them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::export_audio::{LEAD_BEATS, SLOW_SPACING};
use crate::export_job::{
    check_looper_base, ExportEvent, ExportPlan, ExportSnapshot, ExportState, Finished, OnDone,
    PlanInput, Recorded, Sink, TargetReq, BUSY_MESSAGE, STALE_MESSAGE,
};
use crate::export_record::{ExportEntry, TrackExports};
use crate::export_rules::{self, ExportKind, NO_TEMPO_REASON};
use crate::import_job::{locked, RepoLock};
use crate::ipc::RepoState;
use crate::picker::{DialogKind, FolderReq};
use crate::repository::{Repository, TrackRecord};
use crate::settings::SettingsStore;
use crate::tempo_store::TempoStore;
use crate::track_meta;

/// The first section's count-in, as shown in the pane.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CountInInfo {
    pub bpm: f64,
    pub first_beat_ms: f64,
    /// The track's slow count-in switch.
    pub slow: bool,
    /// `bpm`, or `bpm / 2` with the slow count-in.
    pub count_in_bpm: f64,
    /// The count-in: `LEAD_BEATS` beats at `count_in_bpm`.
    pub lead_ms: f64,
    /// The silence before the count-in (as long as it).
    pub silence_ms: f64,
    /// The audio before the first beat that the export leaves out (0 unless the first beat is set).
    pub skip_ms: f64,
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
    /// The memory recorded for the track, else the suggested RC-5 memory.
    pub memory: u8,
    /// The RC-500 memory: the recorded one, else the suggestion after the last one used.
    pub rc500_memory: u8,
    /// The RC-500 track: the recorded one, else 1.
    pub rc500_track: u8,
    /// The track's recorded exports, as read now.
    pub last: TrackExports,
}

/// The record of `id` and its folder, read under the repository lock.
fn load(repo: &RepoState, id: &str) -> Result<(TrackRecord, PathBuf), String> {
    let root = repo.lock();
    let r = Repository::new(root.clone());
    let rec = r.load_record(id)?;
    Ok((rec, r.tracks_dir().join(id)))
}

fn count_in_bpm(bpm: f64, slow: bool) -> f64 {
    if slow {
        bpm / SLOW_SPACING as f64
    } else {
        bpm
    }
}

/// Sets the track's slow count-in switch and returns its fresh export info.
pub fn do_set_slow_count_in(
    repo: &RepoState,
    settings: &SettingsStore,
    store: &dyn TempoStore,
    id: &str,
    on: bool,
) -> Result<ExportInfo, String> {
    {
        let root = repo.lock();
        Repository::new(root.clone()).set_slow_count_in(id, on)?;
    }
    do_export_info(repo, settings, store, id)
}

pub fn do_export_info(
    repo: &RepoState,
    settings: &SettingsStore,
    store: &dyn TempoStore,
    id: &str,
) -> Result<ExportInfo, String> {
    let (rec, dir) = load(repo, id)?;
    let memory = export_rules::memory_for(
        rec.exports.rc5.as_ref().map(|e| e.memory),
        settings.get().rc5_memory,
    );
    let (rc500_memory, rc500_track) = export_rules::rc500_slot_for(
        rec.exports.rc500.as_ref().map(|e| (e.memory, e.track)),
        settings.get().rc500_memory,
    );
    let mut info = ExportInfo {
        id: id.to_string(),
        available: false,
        reason: None,
        source: None,
        count_in: None,
        memory,
        rc500_memory,
        rc500_track,
        last: rec.exports.clone(),
    };
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
            let lead_ms = LEAD_BEATS as f64 * 60000.0 / count_in_bpm(seg.bpm, rec.slow_count_in);
            info.count_in = Some(CountInInfo {
                bpm: seg.bpm,
                first_beat_ms: seg.start_ms,
                slow: rec.slow_count_in,
                count_in_bpm: count_in_bpm(seg.bpm, rec.slow_count_in),
                lead_ms,
                silence_ms: lead_ms,
                skip_ms: tempo.play_start_ms(),
            });
        }
        _ => info.reason = Some(NO_TEMPO_REASON.into()),
    }
    Ok(info)
}

const RC5_TITLE: &str = "Choose the RC-5 drive (its root folder)";
const RC500_TITLE: &str = "Choose the RC-500 drive (its root folder)";

/// Logs the dialog start and opens the folder dialog.
fn pick(
    repo: &RepoState,
    kind: DialogKind,
    title: &str,
    from: &str,
    start_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    eprintln!(
        "calliope: dialog start kind={} from={from} dir={}",
        kind.as_str(),
        start_dir
            .as_ref()
            .map_or("none".to_string(), |d| d.display().to_string())
    );
    repo.picker.pick_folder(&FolderReq {
        kind,
        title: title.into(),
        start_dir,
    })
}

fn refused<T>(kind: ExportKind, reason: &str, message: impl Into<String>) -> Result<T, String> {
    eprintln!(
        "calliope: export refused target={} reason={reason}",
        kind.as_str()
    );
    Err(message.into())
}

/// Everything that can be checked without a folder, then the folder dialog (starting in the
/// recorded or remembered folder, else `home`), then `ExportState::plan`. `Ok(None)` = the dialog was
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
        TargetReq::Rc5 { memory, .. } => match export_rules::parse_memory(memory) {
            Ok(n) => Some(n),
            Err(e) => return refused(kind, "memory", e),
        },
        TargetReq::Rc500 { memory, track, .. } => {
            let n = match export_rules::parse_memory(memory) {
                Ok(n) => n,
                Err(e) => return refused(kind, "memory", e),
            };
            if let Err(e) = export_rules::parse_track(*track) {
                return refused(kind, "track", e);
            }
            Some(n)
        }
        _ => None,
    };
    let rc500_track = match &target {
        TargetReq::Rc500 { track, .. } => Some(*track),
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

    // RC-5 and RC-500 only: the folder of the last success, when the frontend asks for it and it
    // is still a usable folder; otherwise the dialog, as the first time.
    let mut quick: Option<PathBuf> = None;
    let reuse = match &target {
        TargetReq::Rc5 { reuse, .. } | TargetReq::Rc500 { reuse, .. } => reuse.as_deref(),
        _ => None,
    };
    if let Some(reuse) = reuse {
        match export_rules::quick_rc5_dir(Some(reuse), export.quick_dir(kind).as_deref()) {
            Ok(dir) => {
                eprintln!(
                    "calliope: dialog skipped kind={} dir={}",
                    if kind == ExportKind::Rc500 {
                        "export-rc500"
                    } else {
                        "export-rc5"
                    },
                    dir.display()
                );
                quick = Some(dir);
            }
            Err(reason) => {
                if kind == ExportKind::Rc500 {
                    eprintln!(
                        "calliope: export quick unavailable kind=rc500 reason={reason} dir={reuse}"
                    );
                } else {
                    eprintln!("calliope: export quick unavailable reason={reason} dir={reuse}");
                }
            }
        }
    }
    let skipped = quick.is_some();
    let chosen = match quick {
        Some(dir) => dir,
        None => {
            let (dialog, title) = match kind {
                ExportKind::Rc5 => (DialogKind::ExportRc5, RC5_TITLE),
                ExportKind::Rc500 => (DialogKind::ExportRc500, RC500_TITLE),
                _ => (
                    DialogKind::ExportAudio,
                    "Choose a folder for the MP3/OGG file",
                ),
            };
            let s = settings.get();
            let remembered = match kind {
                ExportKind::Rc5 => s.rc5_dir,
                ExportKind::Rc500 => s.rc500_dir,
                _ => s.export_dir,
            };
            let (from, start_dir) = match kind {
                ExportKind::Rc5 | ExportKind::Rc500 => {
                    export_rules::start_dir(remembered.as_deref(), home.as_deref())
                }
                _ => {
                    let recorded = match kind {
                        ExportKind::Mp3 => rec.exports.mp3.as_ref(),
                        _ => rec.exports.ogg.as_ref(),
                    };
                    let recorded = recorded.map(|e| Path::new(&e.path));
                    export_rules::file_start_dir(recorded, remembered.as_deref(), home.as_deref())
                }
            };
            let Some(chosen) = pick(repo, dialog, title, from, start_dir) else {
                return Ok(None);
            };
            chosen
        }
    };

    let root = repo.current_root();
    let plan = export.plan(PlanInput {
        track: &rec,
        track_dir: dir.clone(),
        source: dir.join(&src.file),
        kind,
        memory,
        rc500_track,
        chosen: &chosen,
        repo_root: &root,
        tempo: &tempo,
    })?;
    if !skipped && chosen.is_absolute() {
        let saved = match kind {
            ExportKind::Rc5 => settings.set_rc5_dir(Some(chosen.clone())),
            ExportKind::Rc500 => settings.set_rc500_dir(Some(chosen.clone())),
            _ => settings.set_export_dir(Some(chosen.clone())),
        };
        match saved {
            Ok(_) => eprintln!(
                "calliope: export remembered kind={} dir={}",
                match kind {
                    ExportKind::Rc5 => "rc5",
                    ExportKind::Rc500 => "rc500",
                    _ => "file",
                },
                chosen.display()
            ),
            Err(e) => eprintln!("calliope: could not save settings: {e}"),
        }
    }
    Ok(Some(plan))
}

/// "Change..." on the RC-5 row: asks for a folder (starting in the quick folder when there is
/// one), checks it like an export would, and remembers it for the next exports. Never exports.
/// `Ok(None)` = the dialog was cancelled.
pub fn do_choose_rc5_folder(
    repo: &RepoState,
    settings: &SettingsStore,
    export: &ExportState,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    do_choose_folder(ExportKind::Rc5, repo, settings, export, home)
}

/// "Change..." on the RC-500 row: the same for the RC-500 drive.
pub fn do_choose_rc500_folder(
    repo: &RepoState,
    settings: &SettingsStore,
    export: &ExportState,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    do_choose_folder(ExportKind::Rc500, repo, settings, export, home)
}

fn do_choose_folder(
    kind: ExportKind,
    repo: &RepoState,
    settings: &SettingsStore,
    export: &ExportState,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let rc500 = kind == ExportKind::Rc500;
    if export.is_running() {
        return refused(kind, "busy", BUSY_MESSAGE);
    }
    let (from, start_dir) = match export.quick_dir(kind).filter(|q| q.is_dir()) {
        Some(q) => ("quick", Some(q)),
        None => {
            let s = settings.get();
            let remembered = if rc500 { s.rc500_dir } else { s.rc5_dir };
            export_rules::start_dir(remembered.as_deref(), home.as_deref())
        }
    };
    let (dialog, title) = if rc500 {
        (DialogKind::ExportRc500, RC500_TITLE)
    } else {
        (DialogKind::ExportRc5, RC5_TITLE)
    };
    let Some(chosen) = pick(repo, dialog, title, from, start_dir) else {
        return Ok(None);
    };
    if let Err((reason, message)) = check_looper_base(kind, &chosen, &repo.current_root()) {
        return refused(kind, reason, message);
    }
    export.set_quick_dir(kind, chosen.clone());
    if chosen.is_absolute() {
        let saved = if rc500 {
            settings.set_rc500_dir(Some(chosen.clone()))
        } else {
            settings.set_rc5_dir(Some(chosen.clone()))
        };
        match saved {
            Ok(_) => eprintln!(
                "calliope: export remembered kind={} dir={}",
                kind.as_str(),
                chosen.display()
            ),
            Err(e) => eprintln!("calliope: could not save settings: {e}"),
        }
    }
    if rc500 {
        eprintln!("calliope: export quick kind=rc500 dir={}", chosen.display());
    } else {
        eprintln!("calliope: export quick dir={}", chosen.display());
    }
    Ok(Some(chosen.to_string_lossy().into_owned()))
}

/// Saves the memory of a finished RC-5 or RC-500 export (a save error is only logged).
pub fn remember_memory(settings: &SettingsStore, kind: ExportKind, n: u8) {
    let (saved, line) = if kind == ExportKind::Rc500 {
        (
            settings.set_rc500_memory(Some(n)),
            format!("export remembered kind=rc500 memory={n}"),
        )
    } else {
        (
            settings.set_rc5_memory(Some(n)),
            format!("export remembered memory={n}"),
        )
    };
    match saved {
        Ok(_) => eprintln!("calliope: {line}"),
        Err(e) => eprintln!("calliope: could not save settings: {e}"),
    }
}

/// Writes the export info of a finished export under the repository lock (plan 2.5): the
/// track's entry, and for the RC-5 the clearing of every other holder of the memory.
pub fn record_finished(lock: &dyn RepoLock, root: &Path, f: &Finished, at: &str) -> Recorded {
    locked(lock, || {
        let repo = Repository::new(root);
        let mut out = Recorded {
            root: root.to_string_lossy().into_owned(),
            ..Recorded::default()
        };
        let mut errors: Vec<String> = Vec::new();
        let entry = match (f.kind, f.memory, f.path.to_str()) {
            (ExportKind::Rc5, Some(memory), _) => Some(ExportEntry::Rc5 { memory }),
            (ExportKind::Rc5, None, _) => None,
            (ExportKind::Rc500, Some(memory), _) => {
                f.track.map(|track| ExportEntry::Rc500 { memory, track })
            }
            (ExportKind::Rc500, ..) => None,
            (_, _, None) => {
                errors.push(
                    "The export info wasn't saved: the file's path is not valid UTF-8".into(),
                );
                None
            }
            (ExportKind::Mp3, _, Some(p)) => Some(ExportEntry::Mp3 { path: p.into() }),
            (ExportKind::Ogg, _, Some(p)) => Some(ExportEntry::Ogg { path: p.into() }),
        };
        if let Some(entry) = entry {
            match repo.record_export(&f.id, &entry, at) {
                Ok(r) => out.records.push(r),
                Err(m) => {
                    eprintln!("calliope: export record failed id={} message={m}", f.id);
                    errors.push(format!("The export info wasn't saved: {m}"));
                }
            }
        }
        if let (ExportKind::Rc5, Some(memory)) = (f.kind, f.memory) {
            let cleared = repo.clear_rc5_holders(memory, &f.id);
            out.records.extend(cleared.cleared);
            for b in cleared.failed {
                errors.push(format!(
                    "The RC-5 info of \"{}\" (memory {memory}) wasn't cleared: {}",
                    b.title, b.message
                ));
            }
        }
        if let (ExportKind::Rc500, Some(memory), Some(track)) = (f.kind, f.memory, f.track) {
            let cleared = repo.clear_rc500_holders(memory, track, &f.id);
            out.records.extend(cleared.cleared);
            for b in cleared.failed {
                errors.push(format!(
                    "The RC-500 info of \"{}\" (memory {memory}, track {track}) wasn't cleared: {}",
                    b.title, b.message
                ));
            }
        }
        out.info_error = (!errors.is_empty()).then(|| errors.join(" "));
        out
    })
}

/// Starts the planned export with the tempo map as it is now (read from the planned track's directory). When it is done, `remember`
/// runs with the memory (RC-5 and RC-500 only), then the export info is recorded in the plan's own
/// repository; both before the `done` event.
pub fn do_start_export(
    repo: &RepoState,
    export: &ExportState,
    store: &dyn TempoStore,
    token: &str,
    overwrite: bool,
    sink: Sink,
    remember: Box<dyn FnOnce(ExportKind, u8) + Send>,
) -> Result<ExportSnapshot, String> {
    let Some(plan) = export.planned(token) else {
        eprintln!("calliope: export refused target=none reason=stale");
        return Err(STALE_MESSAGE.into());
    };
    // The track directory is the one the plan was made for, not the current repository root.
    let tempo = store.load(&plan.id, &plan.track_dir)?;
    // The switch is read now, like the tempo map, so the file matches what the pane shows.
    let slow = Repository::new(plan.repo_root.clone())
        .load_record(&plan.id)?
        .slow_count_in;
    let lock = repo.repo_lock();
    let root = plan.repo_root.clone();
    let on_done: OnDone = Box::new(move |f| {
        if let (true, Some(n)) = (f.kind.is_looper(), f.memory) {
            remember(f.kind, n);
        }
        record_finished(&*lock, &root, f, &track_meta::now_rfc3339())
    });
    export.start(token, overwrite, &tempo, slow, sink, on_done)
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
pub async fn set_slow_count_in(
    app: tauri::AppHandle,
    id: String,
    on: bool,
) -> Result<ExportInfo, String> {
    blocking(move || {
        do_set_slow_count_in(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            &id,
            on,
        )
    })
    .await
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
pub async fn choose_rc5_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    blocking(move || {
        do_choose_rc5_folder(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &app.state::<ExportState>(),
            app.path().home_dir().ok(),
        )
    })
    .await
}

#[tauri::command]
pub async fn choose_rc500_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    blocking(move || {
        do_choose_rc500_folder(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &app.state::<ExportState>(),
            app.path().home_dir().ok(),
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
            Box::new(move |k, n| remember_memory(&handle.state::<SettingsStore>(), k, n)),
        )
    })
    .await
}

#[tauri::command]
pub async fn cancel_export(
    app: tauri::AppHandle,
    job: String,
) -> Result<Option<ExportSnapshot>, String> {
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
