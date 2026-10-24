//! Tauri commands: thin wrappers over pure modules, registered in `gui.rs`.
//!
//! The frontend never sends a real path: files picked in native dialogs are registered in the
//! `PickRegistry` and referred to by token, ids and names are validated by `repository`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::editor::{
    self, EditorEvent, EditorManager, EditorSnapshot, EventSink, LaneState, OpenSpec,
    TransportState,
};
use crate::import_job::{FileKind, ImportEvent, ImportState, JobSnapshot, RepoLock, Sink, UrlPrep};
use crate::picker::{
    DialogKind, FileReq, FolderReq, PickRegistry, Picker, SaveReq, AUDIO_EXTENSIONS,
    TABLATURE_EXTENSIONS, VIDEO_EXTENSIONS,
};
use crate::repository::{
    self, DeleteResult, Library, RepoStatus, Repository, SaveResult, SaveTrackRequest, TrackPatch,
    TrackRef, UpdateResult, MAX_BATCH,
};
use crate::settings::{self, Settings, SettingsStore, Theme};
#[cfg(test)]
use crate::tempo_store::FileTempoStore;
use crate::tempo_store::TempoStore;
use crate::tools::{ToolState, ToolStatus, Tools};
use crate::track_meta::TrackEdits;
use calliope_lib::stems_client::{ClientError, StemsClient};

/// Managed app state: the current repository root (the lock serialises every repository
/// operation), the pick-token registry and the dialog implementation.
pub struct RepoState {
    root: Arc<Mutex<PathBuf>>,
    pub registry: PickRegistry,
    pub picker: Box<dyn Picker>,
}

impl RepoState {
    pub fn new(root: PathBuf, picker: Box<dyn Picker>) -> Self {
        Self {
            root: Arc::new(Mutex::new(root)),
            registry: PickRegistry::new(),
            picker,
        }
    }

    /// The repository lock for the import jobs: the same mutex that serialises every other
    /// repository operation.
    pub fn repo_lock(&self) -> Arc<dyn RepoLock> {
        Arc::new(RootLock(self.root.clone()))
    }

    /// The current root (the lock is not held afterwards).
    pub(crate) fn current_root(&self) -> PathBuf {
        self.lock().clone()
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, PathBuf> {
        self.root.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct RootLock(Arc<Mutex<PathBuf>>);

impl RepoLock for RootLock {
    fn run(&self, f: &mut dyn FnMut()) {
        let _g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f();
    }
}

pub const IMPORT_RUNNING_MESSAGE: &str = "An import is running";
/// How often the app polls the edge-AI job.
pub const IMPORT_POLL: Duration = Duration::from_secs(1);
/// How long the app waits for a running import to stop when it exits.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
pub struct RepoInfo {
    pub root: String,
    pub is_default: bool,
    pub status: RepoStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct PickedRoot {
    pub token: String,
    pub root: String,
    pub status: RepoStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct Picked {
    pub token: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PickPurpose {
    Add,
    Update,
}

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}

fn info(root: &std::path::Path, default_root: &std::path::Path) -> RepoInfo {
    RepoInfo {
        root: path_str(root),
        is_default: root == default_root,
        status: repository::status(root),
    }
}

// ---- command bodies (blocking; no Tauri types, so they are unit-testable) ----

pub fn do_get_repository(st: &RepoState, default_root: &std::path::Path) -> RepoInfo {
    let root = st.lock();
    info(&root, default_root)
}

pub fn do_choose_root(st: &RepoState, home: Option<PathBuf>) -> Option<PickedRoot> {
    let req = FolderReq {
        kind: DialogKind::RepositoryRoot,
        title: "Choose track repository folder".into(),
        start_dir: home,
    };
    let path = st.picker.pick_folder(&req)?;
    let status = repository::status(&path);
    let root = path_str(&path);
    let token = st.registry.insert(DialogKind::RepositoryRoot, path);
    Some(PickedRoot {
        token,
        root,
        status,
    })
}

pub fn do_set_root(
    st: &RepoState,
    store: &SettingsStore,
    import: &ImportState,
    default_root: &std::path::Path,
    token: &str,
) -> Result<RepoInfo, String> {
    // Checked before the root lock is taken: a starting job holds its own lock first.
    if import.is_active() {
        return Err(IMPORT_RUNNING_MESSAGE.into());
    }
    let mut root = st.lock();
    let path = st
        .registry
        .get(DialogKind::RepositoryRoot, token)
        .ok_or("unknown or expired folder pick")?;
    if !path.is_absolute() {
        return Err("the repository folder must be an absolute path".into());
    }
    if repository::status(&path) == RepoStatus::Newer {
        return Err("that repository was written by a newer Calliope".into());
    }
    Repository::new(&path).ensure_layout()?;
    // Store `None` for the default location so that it follows the data directory.
    let setting = if path == default_root {
        None
    } else {
        Some(path.clone())
    };
    store
        .set_repository_root(setting)
        .map_err(|e| format!("could not save settings: {e}"))?;
    st.registry.take(DialogKind::RepositoryRoot, token);
    *root = path;
    Ok(info(&root, default_root))
}

pub fn do_reset_root(
    st: &RepoState,
    store: &SettingsStore,
    import: &ImportState,
    default_root: &std::path::Path,
) -> Result<RepoInfo, String> {
    if import.is_active() {
        return Err(IMPORT_RUNNING_MESSAGE.into());
    }
    let mut root = st.lock();
    store
        .set_repository_root(None)
        .map_err(|e| format!("could not save settings: {e}"))?;
    *root = default_root.to_path_buf();
    Ok(info(&root, default_root))
}

pub fn do_list_tracks(st: &RepoState, default_root: &std::path::Path) -> Result<Library, String> {
    let root = st.lock();
    let repo = Repository::new(root.clone());
    if *root == default_root {
        repo.ensure_layout()?;
    }
    Ok(repo.scan())
}

pub fn do_pick_tablature(
    st: &RepoState,
    purpose: PickPurpose,
    home: Option<PathBuf>,
) -> Result<Option<Picked>, String> {
    let (kind, title) = match purpose {
        PickPurpose::Add => (DialogKind::AddTablature, "Add tablature"),
        PickPurpose::Update => (DialogKind::UpdateTablature, "Update tablature"),
    };
    let req = FileReq {
        kind,
        title: title.into(),
        filter_name: "Tablature files".into(),
        extensions: TABLATURE_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        start_dir: home,
    };
    let Some(path) = st.picker.pick_file(&req) else {
        return Ok(None);
    };
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the picked file has no usable name")?
        .to_string();
    crate::fsutil::validate_file_name(&name).map_err(|e| format!("picked file: {e}"))?;
    if !crate::picker::is_tablature_name(&name) {
        return Err(format!("\"{name}\" is not a tablature file"));
    }
    let token = st.registry.insert(kind, path);
    Ok(Some(Picked { token, name }))
}

pub fn do_save_track(st: &RepoState, request: SaveTrackRequest) -> Result<SaveResult, String> {
    use crate::repository::TabEntry;
    let root = st.lock();
    let tokens: Vec<String> = request
        .tablatures
        .iter()
        .filter_map(|t| match t {
            TabEntry::Add { token } | TabEntry::Replace { token, .. } => Some(token.clone()),
            TabEntry::Keep { .. } => None,
        })
        .collect();
    let lookup = |token: &str| {
        st.registry
            .get(DialogKind::AddTablature, token)
            .or_else(|| st.registry.get(DialogKind::UpdateTablature, token))
    };
    let repo = Repository::new(root.clone());
    if request.group.iter().any(|p| p.id == request.id) {
        return Err("the group of a save cannot contain the saved track".into());
    }
    // check the whole group first: a problem refuses the save and nothing is written
    let prepared = repo.prepare_patches(&request.group)?;
    let mut result = repo.save_track(request, &lookup)?;
    for t in &tokens {
        st.registry.take(DialogKind::AddTablature, t);
        st.registry.take(DialogKind::UpdateTablature, t);
    }
    let group = repo.commit_patches(prepared);
    result.updated = group.tracks;
    result.failed = group.failed;
    Ok(result)
}

pub fn do_update_tracks(st: &RepoState, patches: Vec<TrackPatch>) -> Result<UpdateResult, String> {
    let root = st.lock();
    Repository::new(root.clone()).update_tracks(&patches)
}

pub fn do_delete_tracks(st: &RepoState, items: Vec<TrackRef>) -> Result<DeleteResult, String> {
    if items.is_empty() {
        return Err("no tracks to delete".into());
    }
    if items.len() > MAX_BATCH {
        return Err("too many tracks in one change".into());
    }
    let root = st.lock();
    Ok(Repository::new(root.clone()).delete_tracks(&items))
}

pub fn do_delete_track(st: &RepoState, id: &str, revision: &str) -> Result<(), String> {
    let root = st.lock();
    Repository::new(root.clone()).delete_track(id, revision)
}

pub fn do_export_track(
    st: &RepoState,
    id: &str,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let req = FolderReq {
        kind: DialogKind::ExportTrack,
        title: "Export track to\u{2026}".into(),
        start_dir: home,
    };
    let Some(dest) = st.picker.pick_folder(&req) else {
        return Ok(None);
    };
    let root = st.lock();
    let created = Repository::new(root.clone()).export_track(id, &dest)?;
    Ok(Some(path_str(&created)))
}

pub fn do_export_tablature(
    st: &RepoState,
    id: &str,
    name: &str,
    home: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let req = SaveReq {
        kind: DialogKind::ExportTablature,
        title: "Export tablature".into(),
        default_name: name.to_string(),
        filter_name: "Tablature files".into(),
        extensions: TABLATURE_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        start_dir: home,
    };
    let Some(dest) = st.picker.save_file(&req) else {
        return Ok(None);
    };
    let root = st.lock();
    Repository::new(root.clone()).export_tablature(id, name, &dest)?;
    Ok(Some(path_str(&dest)))
}

// ---- import: DTOs and command bodies ----

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportKind {
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeAiState {
    NotConfigured,
    Connected,
    Unreachable,
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeAiStatus {
    pub state: EdgeAiState,
    pub message: String,
    pub models: Vec<String>,
}

/// One external tool as the Settings card shows it (no paths).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub found: bool,
    pub version: Option<String>,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolsInfo {
    pub yt_dlp: ToolInfo,
    pub ffmpeg: ToolInfo,
    pub ffprobe: ToolInfo,
}

fn tool_info(t: ToolStatus) -> ToolInfo {
    eprintln!(
        "calliope: tool {} found={} version={}",
        t.name.exe(),
        t.state != ToolState::Missing,
        t.version.as_deref().unwrap_or("unknown")
    );
    ToolInfo {
        found: t.state != ToolState::Missing,
        ok: t.usable(),
        version: t.version,
        message: t.message.unwrap_or_default(),
    }
}

pub fn do_check_tools(tools: &Tools) -> ToolsInfo {
    let s = tools.check();
    ToolsInfo {
        yt_dlp: tool_info(s.yt_dlp),
        ffmpeg: tool_info(s.ffmpeg),
        ffprobe: tool_info(s.ffprobe),
    }
}

/// Asks the configured server for its health (blocking, up to the client's connect timeout).
pub fn do_check_edge_ai(url: Option<&str>) -> EdgeAiStatus {
    let mk = |state, message: String, models| EdgeAiStatus {
        state,
        message,
        models,
    };
    let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else {
        return mk(EdgeAiState::NotConfigured, "Not configured".into(), vec![]);
    };
    match StemsClient::new(url).health() {
        Ok(h) => mk(
            EdgeAiState::Connected,
            format!(
                "Connected (calliope-stems {}, model {})",
                h.version, h.default_model
            ),
            h.models,
        ),
        Err(e @ ClientError::Incompatible(_)) => {
            mk(EdgeAiState::Incompatible, e.to_string(), vec![])
        }
        Err(e) => mk(EdgeAiState::Unreachable, e.to_string(), vec![]),
    }
}

pub fn do_set_edge_ai_url(store: &SettingsStore, url: Option<&str>) -> Result<Settings, String> {
    store.set_edge_ai_url(url.unwrap_or(""))
}

pub fn do_set_keep_original(store: &SettingsStore, keep: bool) -> Result<Settings, String> {
    store
        .set_keep_original(keep)
        .map_err(|e| format!("could not save settings: {e}"))
}

pub fn do_prepare_url(st: &RepoState, import: &ImportState, url: &str) -> UrlPrep {
    import.prepare_url(&st.current_root(), url)
}

pub fn do_start_url(
    st: &RepoState,
    import: &ImportState,
    url: &str,
    resume: bool,
    sink: Sink,
) -> Result<JobSnapshot, String> {
    import.start_url(&st.current_root(), url, resume, sink)
}

/// What the file dialog returned: the full path (UTF-8) and its file name.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PickedFile {
    pub path: String,
    pub name: String,
}

/// Which dialog `pick_import_file` opens.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PickKind {
    Audio,
    Video,
    Directory,
}

/// Opens the dialog for `kind` and returns the chosen path. `Ok(None)` when cancelled. The
/// dialog starts in the folder of `typed`, else the remembered folder, else home. No validation
/// here: `do_import_path` and `do_import_directory` do it.
pub fn do_pick_import_file(
    st: &RepoState,
    store: &SettingsStore,
    kind: PickKind,
    typed: &str,
    home: Option<PathBuf>,
) -> Result<Option<PickedFile>, String> {
    let start = crate::local_source::browse_start(
        typed,
        store.get().last_import_dir.as_deref(),
        home.as_deref(),
    );
    if start.forget_last {
        eprintln!("calliope: import last-location forgotten");
        if let Err(e) = store.set_last_import_dir(None) {
            eprintln!("calliope: could not save settings: {e}");
        }
    }
    let start_dir = start.dir;
    let (dialog, title, filter, exts) = match kind {
        PickKind::Audio => (
            DialogKind::ImportAudio,
            "Choose an audio file",
            "Audio files",
            AUDIO_EXTENSIONS,
        ),
        PickKind::Video => (
            DialogKind::ImportVideo,
            "Choose a video file",
            "Video files",
            VIDEO_EXTENSIONS,
        ),
        PickKind::Directory => (
            DialogKind::ImportDirectory,
            "Choose a folder of songs",
            "",
            &[][..],
        ),
    };
    eprintln!(
        "calliope: dialog start kind={} from={} dir={}",
        dialog.as_str(),
        start.from.as_str(),
        start_dir
            .as_ref()
            .map_or("none".to_string(), |d| d.display().to_string())
    );
    let picked = if kind == PickKind::Directory {
        st.picker.pick_folder(&FolderReq {
            kind: dialog,
            title: title.into(),
            start_dir,
        })
    } else {
        st.picker.pick_file(&FileReq {
            kind: dialog,
            title: title.into(),
            filter_name: filter.into(),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
            start_dir,
        })
    };
    let Some(path) = picked else {
        return Ok(None);
    };
    let what = if kind == PickKind::Directory {
        "folder"
    } else {
        "file"
    };
    let not_utf8 = format!(
        "The chosen {what}'s path has characters Calliope cannot show; rename the {what} or its folder"
    );
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(not_utf8.as_str())?
        .to_string();
    let path = path.to_str().ok_or(not_utf8.as_str())?.to_string();
    Ok(Some(PickedFile { path, name }))
}

/// Saves `folder` as the starting folder of the next Browse. A save error is only logged.
fn remember_location(store: &SettingsStore, folder: &std::path::Path) {
    match store.set_last_import_dir(Some(folder.to_path_buf())) {
        Ok(_) => eprintln!("calliope: import last-location dir={}", folder.display()),
        Err(e) => eprintln!("calliope: could not save settings: {e}"),
    }
}

/// Starts the import of a typed or picked path. The only command that takes a path; it only
/// ever reads it (see "Local import by typed path" in docs/architecture.md).
pub fn do_import_path(
    st: &RepoState,
    import: &ImportState,
    store: &SettingsStore,
    kind: ImportKind,
    path: &str,
    home: Option<PathBuf>,
    sink: Sink,
) -> Result<JobSnapshot, String> {
    if import.is_active() {
        return Err(crate::import_job::BUSY_MESSAGE.into());
    }
    let (local, file_kind, k) = match kind {
        ImportKind::Audio => (
            crate::local_source::LocalKind::Audio,
            FileKind::Audio,
            "audio",
        ),
        ImportKind::Video => (
            crate::local_source::LocalKind::Video,
            FileKind::Video,
            "video",
        ),
    };
    let abs = match crate::local_source::resolve(path, local, home.as_deref()) {
        Ok(p) => p,
        Err(p) => {
            eprintln!(
                "calliope: import path kind={k} result=rejected reason={}",
                p.reason
            );
            return Err(p.message);
        }
    };
    eprintln!("calliope: import path kind={k} result=accepted");
    let snap = import.start_file(&st.current_root(), file_kind, &abs, sink)?;
    if let Some(folder) = abs.parent().filter(|p| !p.as_os_str().is_empty()) {
        remember_location(store, folder);
    }
    Ok(snap)
}

/// Starts the import of every audio file directly inside the folder `path`. Read-only, like
/// `do_import_path`. The folder itself is remembered once the job started.
pub fn do_import_directory(
    st: &RepoState,
    import: &ImportState,
    store: &SettingsStore,
    path: &str,
    home: Option<PathBuf>,
    sink: Sink,
) -> Result<JobSnapshot, String> {
    if import.is_active() {
        return Err(crate::import_job::BUSY_MESSAGE.into());
    }
    let reject = |reason: &str| {
        eprintln!("calliope: import path kind=directory result=rejected reason={reason}")
    };
    let dir = crate::local_source::resolve_dir(path, home.as_deref()).map_err(|p| {
        reject(p.reason);
        p.message
    })?;
    let files = crate::local_source::list_audio_files(&dir).map_err(|p| {
        reject(p.reason);
        p.message
    })?;
    if files.is_empty() {
        reject("no-audio");
        return Err(crate::local_source::no_audio_message(&dir));
    }
    let s = store.get();
    let n = files.len();
    let snap = import.start_batch(
        &st.current_root(),
        &dir,
        files,
        s.edge_ai_url.as_deref(),
        s.keep_original,
        sink,
    )?;
    eprintln!("calliope: import path kind=directory result=accepted files={n}");
    remember_location(store, &dir);
    Ok(snap)
}

pub fn do_start_extraction(
    import: &ImportState,
    store: &SettingsStore,
    job: &str,
    edits: TrackEdits,
) -> Result<JobSnapshot, String> {
    let s = store.get();
    import.start_extraction(job, edits, s.edge_ai_url.as_deref(), s.keep_original)
}

/// Checks the configured server once at start-up (only when configured) and logs the result.
pub fn startup_health_check(url: Option<String>) {
    if url.is_none() {
        return;
    }
    let st = do_check_edge_ai(url.as_deref());
    eprintln!(
        "calliope: edge-ai state={}",
        serde_json::to_string(&st.state)
            .unwrap_or_default()
            .trim_matches('"')
    );
}

fn channel_sink(events: tauri::ipc::Channel<ImportEvent>) -> Sink {
    Arc::new(move |ev| {
        let _ = events.send(ev);
    })
}

// ---- editor: state and command bodies ----

/// Managed app state of the backing-track editor: the session manager plus the slot holding
/// the webview's current event channel (replaced by `open_editor` and `watch_editor`).
pub struct EditorState {
    mgr: Mutex<EditorManager>,
    slot: Arc<Mutex<Option<EventSink>>>,
    /// The running save's flag, reachable without the manager lock.
    live_save: Arc<Mutex<Option<Arc<crate::backing_render::SaveShared>>>>,
    /// Called before the Editor starts to sound (the app pauses the Player there).
    before_play: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl EditorState {
    pub fn new(backend: Arc<dyn crate::audio_out::OutputBackend>) -> Self {
        let slot: Arc<Mutex<Option<EventSink>>> = Arc::new(Mutex::new(None));
        let forward = slot.clone();
        let sink: EventSink = Arc::new(move |ev| {
            let target = forward.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(t) = target {
                t(ev);
            }
        });
        let mut mgr = EditorManager::new(backend, sink);
        // Closing a real device can block: handles are dropped after the lock is released.
        mgr.defer_drops();
        let live_save = mgr.live_save();
        Self {
            mgr: Mutex::new(mgr),
            slot,
            live_save,
            before_play: Mutex::new(None),
        }
    }

    /// Runs `f` on the manager. Output handles it retired are dropped after the lock is
    /// released, so a stalled audio device never blocks other commands or the ticker.
    pub fn with<R>(&self, f: impl FnOnce(&mut EditorManager) -> R) -> R {
        let (r, trash) = {
            let mut m = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
            let r = f(&mut m);
            (r, m.take_trash())
        };
        drop(trash);
        r
    }

    #[cfg(test)]
    fn mgr(&self) -> MutexGuard<'_, EditorManager> {
        self.mgr.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Installs the hook that runs before the Editor starts to sound.
    pub fn set_before_play(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.before_play.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn run_before_play(&self) {
        let hook = self
            .before_play
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(h) = hook {
            h();
        }
    }

    fn set_sink(&self, sink: EventSink) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    /// Ticker body (every 20 ms).
    pub fn tick(&self, now: std::time::Instant) {
        self.with(|m| m.tick(now));
    }

    /// App exit: stops playback and ends a running save, within about `cap`. When the
    /// manager stays locked (a stuck command) the save is still cancelled through its flag.
    pub fn shutdown(&self, cap: Duration) {
        let end = std::time::Instant::now() + cap;
        let lock_end = std::time::Instant::now() + cap / 2;
        let guard = loop {
            match self.mgr.try_lock() {
                Ok(g) => break Some(g),
                Err(std::sync::TryLockError::Poisoned(e)) => break Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= lock_end {
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };
        match guard {
            Some(mut m) => {
                m.close();
                m.shutdown_save(end.saturating_duration_since(std::time::Instant::now()));
                let trash = m.take_trash();
                drop(m);
                // Closing the device may hang: do it on a helper thread and wait a little.
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    drop(trash);
                    let _ = tx.send(());
                });
                let _ = rx.recv_timeout(end.saturating_duration_since(std::time::Instant::now()));
            }
            None => {
                eprintln!("calliope: editor busy at exit; cancelling the save without the lock");
                let shared = self
                    .live_save
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(sh) = shared {
                    sh.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    while !sh.is_finished() && std::time::Instant::now() < end {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
    }
}

/// How often the editor ticker runs.
pub const EDITOR_TICK: Duration = Duration::from_millis(20);

pub fn do_open_editor(
    st: &EditorState,
    repo: &RepoState,
    tempo: &dyn crate::tempo_store::TempoStore,
    id: &str,
    sink: EventSink,
) -> Result<EditorSnapshot, String> {
    // The open is registered before anything slow, so overlapping opens are ordered by
    // their arrival here and the newest one owns the event sink.
    let (ticket, events) = st.with(|m| {
        st.set_sink(sink);
        (m.begin_open(), m.sink())
    });
    let result = (|| {
        let spec = {
            let root = repo.lock();
            let r = Repository::new(root.clone());
            let rec = r.load_record(id)?;
            let dir = r.tracks_dir().join(&rec.id);
            OpenSpec {
                // Through the shared store: the Player's pending state is flushed first.
                tempo: tempo.load(&rec.id, &dir).unwrap_or_default(),
                dir,
                id: rec.id,
                title: rec.title,
                stems: rec.stems,
                missing: rec.missing,
                variant: rec.backings.into_iter().next(),
            }
        };
        let loaded = editor::load(&spec, &ticket, &*events)?;
        st.with(|m| m.finish_open(&ticket, &spec, loaded))
    })();
    if result.is_err() {
        st.with(|m| m.abort_open(&ticket));
    }
    result
}

pub fn do_close_editor(st: &EditorState) {
    st.with(|m| m.close());
}

/// Re-attach after a webview reload. It does not take the sink from an open in progress.
pub fn do_watch_editor(st: &EditorState, sink: EventSink) -> Option<EditorSnapshot> {
    st.with(|m| {
        if !m.is_opening() {
            st.set_sink(sink);
        }
        m.current()
    })
}

/// What `editor_tempo` does (plan 2.11).
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TempoEdit {
    /// Absolute offset in steps from the detected position of change `index`.
    Move {
        index: usize,
        steps: i64,
    },
    Reload,
}

pub fn do_editor_play(st: &EditorState, id: &str) -> Result<TransportState, String> {
    let req = st.with(|m| m.play_request(id))?;
    st.run_before_play();
    // The device is opened without the manager lock.
    let opened = req.map(editor::OutputRequest::open).transpose()?;
    st.with(|m| m.play_with(id, opened))
}

pub fn do_editor_lane_play(
    st: &EditorState,
    id: &str,
    name: &str,
) -> Result<TransportState, String> {
    let req = st.with(|m| m.lane_play_request(id, name))?;
    st.run_before_play();
    let opened = req.map(editor::OutputRequest::open).transpose()?;
    st.with(|m| m.lane_play_with(id, name, opened))
}

pub fn do_editor_seek(st: &EditorState, id: &str, position: u64) -> Result<TransportState, String> {
    st.with(|m| m.seek(id, position, std::time::Instant::now()))
}

pub fn do_editor_nudge(st: &EditorState, id: &str, delta: i64) -> Result<TransportState, String> {
    st.with(|m| m.nudge(id, delta, std::time::Instant::now()))
}

/// Starts the beat detection of the checked stems (plan 2.11). `url` is the edge-AI server
/// from the settings (empty = not set).
pub fn do_editor_detect(
    st: &EditorState,
    store: Arc<dyn TempoStore>,
    url: String,
    id: &str,
) -> Result<EditorSnapshot, String> {
    let params = crate::beat_detect::DetectParams {
        url,
        store,
        poll: IMPORT_POLL,
        stall: calliope_lib::stems_client::STALL_TIMEOUT,
    };
    st.with(|m| m.start_detect(id, params))
}

/// Edits the tempo map of the open track through the store and returns the new snapshot.
pub fn do_editor_tempo(
    st: &EditorState,
    store: &dyn TempoStore,
    id: &str,
    action: TempoEdit,
) -> Result<EditorSnapshot, String> {
    let dir = st.with(|m| m.dir_of(id))?;
    let tempo = match action {
        TempoEdit::Move { index, steps } => {
            if st.with(|m| m.detect_running(id)) {
                return Err("Beat detection is running; wait for it to finish".into());
            }
            store.replace(id, &dir, &|old| {
                old.set_change_offset(index, steps)
                    .map(|(map, _applied)| map)
            })?
        }
        TempoEdit::Reload => store.load(id, &dir)?,
    };
    st.with(|m| m.set_tempo(id, tempo))
}

pub fn do_save_backing(
    st: &EditorState,
    repo: &RepoState,
    id: &str,
) -> Result<EditorSnapshot, String> {
    let r = Repository::new(repo.current_root());
    st.with(|m| m.start_save(id, r, repo.repo_lock()))
}

fn event_sink(events: tauri::ipc::Channel<EditorEvent>) -> EventSink {
    Arc::new(move |ev| {
        let _ = events.send(ev);
    })
}

// ---- Tauri commands ----

fn home(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().home_dir().ok()
}

fn default_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .data_dir()
        .map(|d| settings::default_repository_root(&d))
        .map_err(|e| format!("no data directory: {e}"))
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
pub async fn get_repository(app: tauri::AppHandle) -> Result<RepoInfo, String> {
    let d = default_root(&app)?;
    blocking(move || Ok(do_get_repository(&app.state::<RepoState>(), &d))).await
}

#[tauri::command]
pub async fn choose_repository_root(app: tauri::AppHandle) -> Result<Option<PickedRoot>, String> {
    blocking(move || Ok(do_choose_root(&app.state::<RepoState>(), home(&app)))).await
}

#[tauri::command]
pub async fn set_repository_root(app: tauri::AppHandle, token: String) -> Result<RepoInfo, String> {
    let d = default_root(&app)?;
    blocking(move || {
        do_set_root(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &app.state::<ImportState>(),
            &d,
            &token,
        )
    })
    .await
}

#[tauri::command]
pub async fn reset_repository_root(app: tauri::AppHandle) -> Result<RepoInfo, String> {
    let d = default_root(&app)?;
    blocking(move || {
        do_reset_root(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            &app.state::<ImportState>(),
            &d,
        )
    })
    .await
}

#[tauri::command]
pub async fn list_tracks(app: tauri::AppHandle) -> Result<Library, String> {
    let d = default_root(&app)?;
    blocking(move || do_list_tracks(&app.state::<RepoState>(), &d)).await
}

#[tauri::command]
pub async fn pick_tablature(
    app: tauri::AppHandle,
    purpose: PickPurpose,
) -> Result<Option<Picked>, String> {
    blocking(move || do_pick_tablature(&app.state::<RepoState>(), purpose, home(&app))).await
}

#[tauri::command]
pub async fn save_track(
    app: tauri::AppHandle,
    request: SaveTrackRequest,
) -> Result<SaveResult, String> {
    blocking(move || do_save_track(&app.state::<RepoState>(), request)).await
}

#[tauri::command]
pub async fn delete_track(
    app: tauri::AppHandle,
    id: String,
    revision: String,
) -> Result<(), String> {
    blocking(move || do_delete_track(&app.state::<RepoState>(), &id, &revision)).await
}

#[tauri::command]
pub async fn update_tracks(
    app: tauri::AppHandle,
    patches: Vec<TrackPatch>,
) -> Result<UpdateResult, String> {
    blocking(move || do_update_tracks(&app.state::<RepoState>(), patches)).await
}

#[tauri::command]
pub async fn delete_tracks(
    app: tauri::AppHandle,
    items: Vec<TrackRef>,
) -> Result<DeleteResult, String> {
    blocking(move || do_delete_tracks(&app.state::<RepoState>(), items)).await
}

#[tauri::command]
pub async fn export_track(app: tauri::AppHandle, id: String) -> Result<Option<String>, String> {
    blocking(move || do_export_track(&app.state::<RepoState>(), &id, home(&app))).await
}

#[tauri::command]
pub async fn export_tablature(
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<Option<String>, String> {
    blocking(move || do_export_tablature(&app.state::<RepoState>(), &id, &name, home(&app))).await
}

#[tauri::command]
pub async fn set_edge_ai_url(
    app: tauri::AppHandle,
    url: Option<String>,
) -> Result<Settings, String> {
    blocking(move || do_set_edge_ai_url(&app.state::<SettingsStore>(), url.as_deref())).await
}

#[tauri::command]
pub async fn set_keep_original(app: tauri::AppHandle, keep: bool) -> Result<Settings, String> {
    blocking(move || do_set_keep_original(&app.state::<SettingsStore>(), keep)).await
}

#[tauri::command]
pub async fn check_edge_ai(app: tauri::AppHandle) -> Result<EdgeAiStatus, String> {
    blocking(move || {
        Ok(do_check_edge_ai(
            app.state::<SettingsStore>().get().edge_ai_url.as_deref(),
        ))
    })
    .await
}

#[tauri::command]
pub async fn check_tools(app: tauri::AppHandle) -> Result<ToolsInfo, String> {
    blocking(move || {
        let tools = app.state::<ImportState>().tools();
        Ok(do_check_tools(&tools))
    })
    .await
}

#[tauri::command]
pub async fn prepare_url_import(app: tauri::AppHandle, url: String) -> Result<UrlPrep, String> {
    blocking(move || {
        Ok(do_prepare_url(
            &app.state::<RepoState>(),
            &app.state::<ImportState>(),
            &url,
        ))
    })
    .await
}

#[tauri::command]
pub async fn start_url_import(
    app: tauri::AppHandle,
    url: String,
    resume: bool,
    events: tauri::ipc::Channel<ImportEvent>,
) -> Result<JobSnapshot, String> {
    blocking(move || {
        do_start_url(
            &app.state::<RepoState>(),
            &app.state::<ImportState>(),
            &url,
            resume,
            channel_sink(events),
        )
    })
    .await
}

#[tauri::command]
pub async fn pick_import_file(
    app: tauri::AppHandle,
    kind: PickKind,
    typed: Option<String>,
) -> Result<Option<PickedFile>, String> {
    blocking(move || {
        do_pick_import_file(
            &app.state::<RepoState>(),
            &app.state::<SettingsStore>(),
            kind,
            typed.as_deref().unwrap_or(""),
            home(&app),
        )
    })
    .await
}

#[tauri::command]
pub async fn import_path(
    app: tauri::AppHandle,
    kind: ImportKind,
    path: String,
    events: tauri::ipc::Channel<ImportEvent>,
) -> Result<JobSnapshot, String> {
    blocking(move || {
        do_import_path(
            &app.state::<RepoState>(),
            &app.state::<ImportState>(),
            &app.state::<SettingsStore>(),
            kind,
            &path,
            home(&app),
            channel_sink(events),
        )
    })
    .await
}

#[tauri::command]
pub async fn import_directory(
    app: tauri::AppHandle,
    path: String,
    events: tauri::ipc::Channel<ImportEvent>,
) -> Result<JobSnapshot, String> {
    blocking(move || {
        do_import_directory(
            &app.state::<RepoState>(),
            &app.state::<ImportState>(),
            &app.state::<SettingsStore>(),
            &path,
            home(&app),
            channel_sink(events),
        )
    })
    .await
}

#[tauri::command]
pub async fn start_stem_extraction(
    app: tauri::AppHandle,
    job: String,
    edits: TrackEdits,
) -> Result<JobSnapshot, String> {
    blocking(move || {
        do_start_extraction(
            &app.state::<ImportState>(),
            &app.state::<SettingsStore>(),
            &job,
            edits,
        )
    })
    .await
}

#[tauri::command]
pub async fn cancel_import(app: tauri::AppHandle, job: String) -> Result<JobSnapshot, String> {
    blocking(move || app.state::<ImportState>().cancel(&job)).await
}

#[tauri::command]
pub async fn discard_import(app: tauri::AppHandle, job: String) -> Result<(), String> {
    blocking(move || app.state::<ImportState>().discard(&job)).await
}

#[tauri::command]
pub async fn get_import_job(app: tauri::AppHandle) -> Result<Option<JobSnapshot>, String> {
    Ok(app.state::<ImportState>().snapshot())
}

#[tauri::command]
pub async fn watch_import(
    app: tauri::AppHandle,
    events: tauri::ipc::Channel<ImportEvent>,
) -> Result<Option<JobSnapshot>, String> {
    let import = app.state::<ImportState>();
    import.replace_sink(channel_sink(events));
    Ok(import.snapshot())
}

#[tauri::command]
pub async fn open_editor(
    app: tauri::AppHandle,
    id: String,
    events: tauri::ipc::Channel<EditorEvent>,
) -> Result<EditorSnapshot, String> {
    blocking(move || {
        do_open_editor(
            &app.state::<EditorState>(),
            &app.state::<RepoState>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            &id,
            event_sink(events),
        )
    })
    .await
}

#[tauri::command]
pub async fn close_editor(app: tauri::AppHandle) -> Result<(), String> {
    blocking(move || {
        do_close_editor(&app.state::<EditorState>());
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn get_editor(app: tauri::AppHandle) -> Result<Option<EditorSnapshot>, String> {
    blocking(move || Ok(app.state::<EditorState>().with(|m| m.current()))).await
}

#[tauri::command]
pub async fn watch_editor(
    app: tauri::AppHandle,
    events: tauri::ipc::Channel<EditorEvent>,
) -> Result<Option<EditorSnapshot>, String> {
    blocking(move || {
        Ok(do_watch_editor(
            &app.state::<EditorState>(),
            event_sink(events),
        ))
    })
    .await
}

#[tauri::command]
pub async fn editor_play(app: tauri::AppHandle, id: String) -> Result<TransportState, String> {
    blocking(move || do_editor_play(&app.state::<EditorState>(), &id)).await
}

#[tauri::command]
pub async fn editor_lane_play(
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<TransportState, String> {
    blocking(move || do_editor_lane_play(&app.state::<EditorState>(), &id, &name)).await
}

#[tauri::command]
pub async fn editor_end_solo(app: tauri::AppHandle, id: String) -> Result<TransportState, String> {
    blocking(move || app.state::<EditorState>().with(|m| m.end_solo(&id))).await
}

#[tauri::command]
pub async fn editor_pause(app: tauri::AppHandle, id: String) -> Result<TransportState, String> {
    blocking(move || app.state::<EditorState>().with(|m| m.pause(&id))).await
}

#[tauri::command]
pub async fn editor_stop(app: tauri::AppHandle, id: String) -> Result<TransportState, String> {
    blocking(move || app.state::<EditorState>().with(|m| m.stop(&id))).await
}

#[tauri::command]
pub async fn editor_seek(
    app: tauri::AppHandle,
    id: String,
    position: u64,
) -> Result<TransportState, String> {
    blocking(move || do_editor_seek(&app.state::<EditorState>(), &id, position)).await
}

#[tauri::command]
pub async fn editor_nudge(
    app: tauri::AppHandle,
    id: String,
    delta: i64,
) -> Result<TransportState, String> {
    blocking(move || do_editor_nudge(&app.state::<EditorState>(), &id, delta)).await
}

#[tauri::command]
pub async fn editor_set_stem(
    app: tauri::AppHandle,
    id: String,
    name: String,
    gain: Option<f32>,
    unmuted: bool,
) -> Result<LaneState, String> {
    blocking(move || {
        app.state::<EditorState>()
            .with(|m| m.set_stem(&id, &name, gain, unmuted, std::time::Instant::now()))
    })
    .await
}

#[tauri::command]
pub async fn editor_detect(app: tauri::AppHandle, id: String) -> Result<EditorSnapshot, String> {
    blocking(move || {
        let url = app
            .state::<SettingsStore>()
            .get()
            .edge_ai_url
            .unwrap_or_default();
        do_editor_detect(
            &app.state::<EditorState>(),
            app.state::<Arc<dyn TempoStore>>().inner().clone(),
            url,
            &id,
        )
    })
    .await
}

#[tauri::command]
pub async fn editor_cancel_detect(app: tauri::AppHandle, id: String) -> Result<(), String> {
    blocking(move || app.state::<EditorState>().with(|m| m.cancel_detect(&id))).await
}

#[tauri::command]
pub async fn editor_tempo(
    app: tauri::AppHandle,
    id: String,
    action: TempoEdit,
) -> Result<EditorSnapshot, String> {
    blocking(move || {
        do_editor_tempo(
            &app.state::<EditorState>(),
            &**app.state::<Arc<dyn TempoStore>>(),
            &id,
            action,
        )
    })
    .await
}

#[tauri::command]
pub async fn save_backing(app: tauri::AppHandle, id: String) -> Result<EditorSnapshot, String> {
    blocking(move || do_save_backing(&app.state::<EditorState>(), &app.state::<RepoState>(), &id))
        .await
}

#[tauri::command]
pub async fn cancel_backing_save(app: tauri::AppHandle, id: String) -> Result<(), String> {
    blocking(move || app.state::<EditorState>().with(|m| m.cancel_save(&id))).await
}

#[tauri::command]
pub fn app_version() -> String {
    crate::VERSION.to_string()
}

#[tauri::command]
pub fn get_settings(store: tauri::State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
pub fn set_theme(store: tauri::State<'_, SettingsStore>, theme: Theme) -> Result<Settings, String> {
    store
        .set_theme(theme)
        .map_err(|e| format!("could not save settings: {e}"))
}

#[tauri::command]
pub fn frontend_log(message: String) {
    eprintln!("calliope-ui: {}", sanitize_log(&message));
}

/// Replaces control characters with a space and truncates to 500 chars.
pub fn sanitize_log(msg: &str) -> String {
    msg.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_matches_build_version() {
        let v = app_version();
        assert_eq!(v, env!("CALLIOPE_VERSION"));
        let parts: Vec<&str> = v.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts
            .iter()
            .all(|p| p.len() >= 2 && p.chars().all(|c| c.is_ascii_digit())));
        assert_eq!(parts[0].len(), 2);
        assert_eq!(parts[1].len(), 2);
        assert_eq!(parts[2].len(), 4);
    }

    #[test]
    fn sanitize_replaces_control_chars() {
        assert_eq!(sanitize_log("a\nb\rc\td\u{7}e\u{1b}f"), "a b c d e f");
        assert_eq!(sanitize_log("plain text"), "plain text");
    }

    #[test]
    fn sanitize_truncates_to_500_chars() {
        let long = "é".repeat(600);
        assert_eq!(sanitize_log(&long).chars().count(), 500);
    }

    // ---- repository command bodies with a scripted picker ----

    use crate::picker::ScriptedPicker;
    use crate::repository::{NewTrack, TabEntry};
    use crate::track_meta::TrackEdits;

    struct Env {
        _dir: tempfile::TempDir,
        base: PathBuf,
        st: RepoState,
        store: SettingsStore,
        import: ImportState,
        default: PathBuf,
        answers: PathBuf,
        picker: Arc<ScriptedPicker>,
    }

    fn env(answers: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let answers_path = base.join("answers");
        std::fs::write(&answers_path, answers).unwrap();
        let default = base.join("data/calliope");
        let picker = Arc::new(ScriptedPicker::new(answers_path.clone()));
        let st = RepoState::new(default.clone(), Box::new(picker.clone()));
        let store = SettingsStore::open(base.join("settings.json"));
        let import = ImportState::new(Tools::default(), st.repo_lock(), Duration::from_millis(10));
        Env {
            _dir: dir,
            base,
            st,
            store,
            import,
            default,
            answers: answers_path,
            picker,
        }
    }

    fn seed(e: &Env) -> crate::repository::TrackRecord {
        let repo = Repository::new(&e.default);
        repo.ensure_layout().unwrap();
        let audio = e.base.join("a.mp3");
        std::fs::write(&audio, b"mp3").unwrap();
        repo.create_track(
            NewTrack {
                id: None,
                edits: TrackEdits {
                    band: "B".into(),
                    album: "A".into(),
                    title: "T".into(),
                    composers: vec![],
                    year: None,
                    source_url: None,
                    copyright: None,
                },
                audio_name: "a.mp3".into(),
                tablatures: vec![],
            },
            &audio,
        )
        .unwrap()
    }

    fn patch_of(r: &crate::repository::TrackRecord) -> TrackPatch {
        TrackPatch {
            id: r.id.clone(),
            revision: r.revision.clone(),
            band: None,
            album: Some("A2".into()),
            position: None,
            album_position: None,
        }
    }

    fn save_request(
        r: &crate::repository::TrackRecord,
        group: Vec<TrackPatch>,
    ) -> SaveTrackRequest {
        SaveTrackRequest {
            id: r.id.clone(),
            revision: r.revision.clone(),
            edits: TrackEdits {
                band: r.band.clone(),
                album: "A2".into(),
                title: r.title.clone(),
                composers: vec![],
                year: None,
                source_url: None,
                copyright: None,
            },
            tablatures: vec![],
            position: Some(Some(1)),
            album_position: None,
            group,
        }
    }

    #[test]
    fn a_save_with_a_group_renames_all_and_a_stale_member_refuses_it() {
        let e = env("");
        let (a, b, c) = (seed(&e), seed(&e), seed(&e));
        let files = |ids: [&str; 3]| -> Vec<Vec<u8>> {
            ids.iter()
                .map(|i| {
                    std::fs::read(e.default.join("tracks").join(i).join("track.json")).unwrap()
                })
                .collect()
        };
        let ids = [a.id.as_str(), b.id.as_str(), c.id.as_str()];
        let before = files(ids);
        let mut stale = patch_of(&c);
        stale.revision = "stale".into();
        let err = do_save_track(&e.st, save_request(&a, vec![patch_of(&b), stale])).unwrap_err();
        assert!(err.starts_with("conflict:"), "{err}");
        assert_eq!(files(ids), before, "nothing was written");
        // the main track cannot be in its own group
        assert!(do_save_track(&e.st, save_request(&a, vec![patch_of(&a)])).is_err());
        assert_eq!(files(ids), before);

        let res = do_save_track(&e.st, save_request(&a, vec![patch_of(&b), patch_of(&c)])).unwrap();
        assert_eq!(
            (res.track.album.as_str(), res.track.position),
            ("A2", Some(1))
        );
        assert_eq!(res.updated.len(), 2);
        assert!(res.failed.is_empty() && res.updated.iter().all(|t| t.album == "A2"));
        let lib = do_list_tracks(&e.st, &e.default).unwrap();
        assert!(lib.tracks.iter().all(|t| t.album == "A2"));
    }

    #[test]
    fn update_and_delete_tracks_commands() {
        let e = env("");
        let (a, b) = (seed(&e), seed(&e));
        let res = do_update_tracks(&e.st, vec![patch_of(&a)]).unwrap();
        assert_eq!(res.tracks[0].album, "A2");
        let err = do_delete_tracks(&e.st, vec![]).unwrap_err();
        assert!(!err.is_empty());
        let items = vec![
            TrackRef {
                id: res.tracks[0].id.clone(),
                revision: res.tracks[0].revision.clone(),
            },
            TrackRef {
                id: b.id.clone(),
                revision: "stale".into(),
            },
        ];
        let out = do_delete_tracks(&e.st, items).unwrap();
        assert_eq!(out.deleted, vec![a.id]);
        assert_eq!(out.failed.len(), 1);
    }

    #[test]
    fn list_tracks_creates_default_layout_only() {
        let e = env("");
        let lib = do_list_tracks(&e.st, &e.default).unwrap();
        assert!(lib.tracks.is_empty());
        assert!(e.default.join("tracks").is_dir());
        let info = do_get_repository(&e.st, &e.default);
        assert!(info.is_default);
        assert_eq!(info.status, RepoStatus::Ok);
    }

    #[test]
    fn configured_missing_root_is_not_created() {
        let e = env("");
        let missing = e.base.join("usb/lib");
        let st = RepoState::new(
            missing.clone(),
            Box::new(ScriptedPicker::new(e.answers.clone())),
        );
        let lib = do_list_tracks(&st, &e.default).unwrap();
        assert!(lib.tracks.is_empty());
        assert!(!missing.exists());
        assert_eq!(
            do_get_repository(&st, &e.default).status,
            RepoStatus::Missing
        );
    }

    #[test]
    fn choose_then_set_root_uses_token_once_and_saves_setting() {
        let e = env("");
        let chosen = e.base.join("mylib");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::write(
            &e.answers,
            format!("repository-root {}\n", chosen.display()),
        )
        .unwrap();
        let p = do_choose_root(&e.st, None).unwrap();
        assert_eq!(p.status, RepoStatus::Empty);
        assert!(!p.token.contains('/'));
        assert!(do_set_root(&e.st, &e.store, &e.import, &e.default, "bogus").is_err());
        let info = do_set_root(&e.st, &e.store, &e.import, &e.default, &p.token).unwrap();
        assert_eq!(info.status, RepoStatus::Ok);
        assert!(!info.is_default);
        assert_eq!(e.store.get().repository_root, Some(chosen.clone()));
        assert!(chosen.join("tracks").is_dir());
        assert!(do_set_root(&e.st, &e.store, &e.import, &e.default, &p.token).is_err());
        let info = do_reset_root(&e.st, &e.store, &e.import, &e.default).unwrap();
        assert!(info.is_default);
        assert_eq!(e.store.get().repository_root, None);
    }

    #[test]
    fn set_root_refuses_newer_repository() {
        let e = env("");
        let chosen = e.base.join("new");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::write(
            chosen.join("calliope-repository.json"),
            "{\"schema_version\": 9}",
        )
        .unwrap();
        std::fs::write(
            &e.answers,
            format!("repository-root {}\n", chosen.display()),
        )
        .unwrap();
        let p = do_choose_root(&e.st, None).unwrap();
        assert_eq!(p.status, RepoStatus::Newer);
        assert!(
            do_set_root(&e.st, &e.store, &e.import, &e.default, &p.token)
                .unwrap_err()
                .contains("newer")
        );
        assert_eq!(e.store.get().repository_root, None);
    }

    #[test]
    fn choose_cancel_gives_none() {
        let e = env("repository-root CANCEL\n");
        assert!(do_choose_root(&e.st, None).is_none());
    }

    #[test]
    fn pick_tablature_checks_extension_and_returns_bare_name() {
        let e = env("");
        let good = e.base.join("riff.gp5");
        let bad = e.base.join("notes.txt");
        std::fs::write(
            &e.answers,
            format!(
                "add-tablature {}\nadd-tablature {}\nupdate-tablature CANCEL\n",
                good.display(),
                bad.display()
            ),
        )
        .unwrap();
        let p = do_pick_tablature(&e.st, PickPurpose::Add, None)
            .unwrap()
            .unwrap();
        assert_eq!(p.name, "riff.gp5");
        assert!(do_pick_tablature(&e.st, PickPurpose::Add, None).is_err());
        assert!(do_pick_tablature(&e.st, PickPurpose::Update, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn save_with_token_then_export_and_delete() {
        let e = env("");
        let rec = seed(&e);
        let tab = e.base.join("riff.gp5");
        std::fs::write(&tab, b"gp").unwrap();
        std::fs::write(&e.answers, format!("add-tablature {}\n", tab.display())).unwrap();
        let p = do_pick_tablature(&e.st, PickPurpose::Add, None)
            .unwrap()
            .unwrap();
        let req = SaveTrackRequest {
            id: rec.id.clone(),
            revision: rec.revision.clone(),
            edits: TrackEdits {
                band: "B".into(),
                album: "A".into(),
                title: "T2".into(),
                composers: vec![],
                year: None,
                source_url: None,
                copyright: None,
            },
            tablatures: vec![TabEntry::Add {
                token: p.token.clone(),
            }],
            position: None,
            album_position: None,
            group: vec![],
        };
        let res = do_save_track(&e.st, req.clone()).unwrap();
        assert_eq!(res.track.tablatures, vec!["riff.gp5"]);
        // the token was consumed
        assert!(e
            .st
            .registry
            .get(DialogKind::AddTablature, &p.token)
            .is_none());

        let out = e.base.join("out");
        std::fs::create_dir(&out).unwrap();
        let tabout = e.base.join("out/riff-copy.gp5");
        std::fs::write(
            &e.answers,
            format!(
                "export-track {}\nexport-tablature {}\nexport-track CANCEL\n",
                out.display(),
                tabout.display()
            ),
        )
        .unwrap();
        let dest = do_export_track(&e.st, &rec.id, None).unwrap().unwrap();
        assert!(std::path::Path::new(&dest).join("track.json").is_file());
        assert!(do_export_tablature(&e.st, &rec.id, "riff.gp5", None)
            .unwrap()
            .is_some());
        assert_eq!(std::fs::read(&tabout).unwrap(), b"gp");
        assert!(do_export_track(&e.st, &rec.id, None).unwrap().is_none());

        // stale revision is a conflict, ids are validated
        assert!(do_delete_track(&e.st, &rec.id, &rec.revision)
            .unwrap_err()
            .contains("conflict"));
        assert!(do_delete_track(&e.st, "../x", "r").is_err());
        do_delete_track(&e.st, &rec.id, &res.track.revision).unwrap();
        assert!(do_list_tracks(&e.st, &e.default).unwrap().tracks.is_empty());
        assert!(e.default.join("trash").is_dir());
    }

    // ---- import command bodies ----

    fn null_sink() -> Sink {
        Arc::new(|_| {})
    }

    #[test]
    fn edge_ai_status_not_configured_and_unreachable() {
        assert_eq!(do_check_edge_ai(None).state, EdgeAiState::NotConfigured);
        assert_eq!(
            do_check_edge_ai(Some("  ")).state,
            EdgeAiState::NotConfigured
        );
        // nothing listens on port 1 of the loopback
        let st = do_check_edge_ai(Some("http://127.0.0.1:1"));
        assert_eq!(st.state, EdgeAiState::Unreachable);
        assert!(!st.message.is_empty());
    }

    #[test]
    fn edge_ai_and_keep_original_settings() {
        let e = env("");
        let s = do_set_edge_ai_url(&e.store, Some("http://archserver:8765")).unwrap();
        assert_eq!(s.edge_ai_url.as_deref(), Some("http://archserver:8765"));
        assert!(do_set_edge_ai_url(&e.store, Some("ftp://x")).is_err());
        assert_eq!(
            do_set_edge_ai_url(&e.store, None).unwrap().edge_ai_url,
            None
        );
        assert!(do_set_keep_original(&e.store, true).unwrap().keep_original);
        assert!(!do_set_keep_original(&e.store, false).unwrap().keep_original);
    }

    #[test]
    fn tools_info_has_no_paths() {
        let info = do_check_tools(&Tools::default());
        assert!(!info.yt_dlp.found && !info.yt_dlp.ok && info.yt_dlp.version.is_none());
        assert!(info.ffmpeg.message.contains("ffmpeg"));
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"yt_dlp\"") && !json.contains("path"));
    }

    #[test]
    fn pick_import_file_cancel_and_pick() {
        let e = env("");
        let f = e.base.join("My Song.mp3");
        std::fs::write(
            &e.answers,
            format!(
                "import-audio CANCEL\nimport-audio {}\nimport-video {}\n",
                f.display(),
                f.display()
            ),
        )
        .unwrap();
        assert!(
            do_pick_import_file(&e.st, &e.store, PickKind::Audio, "", None)
                .unwrap()
                .is_none()
        );
        let p = do_pick_import_file(&e.st, &e.store, PickKind::Audio, "", None)
            .unwrap()
            .unwrap();
        assert_eq!(p.path, f.to_str().unwrap());
        assert_eq!(p.name, "My Song.mp3");
        // no validation at pick time
        assert!(
            do_pick_import_file(&e.st, &e.store, PickKind::Video, "", None)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn pick_start_dir_rules() {
        use crate::picker::DialogKind as D;
        let e = env("");
        let music = e.base.join("music");
        let album = music.join("album");
        std::fs::create_dir_all(&album).unwrap();
        let song = album.join("s.mp3");
        std::fs::write(&song, b"x").unwrap();
        let home = e.base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let ans = |lines: &[String]| std::fs::write(&e.answers, lines.join("\n") + "\n").unwrap();
        ans(&vec!["import-audio CANCEL".to_string(); 5]);
        let pick = |k, typed: &str| {
            do_pick_import_file(&e.st, &e.store, k, typed, Some(home.clone())).unwrap()
        };
        // typed file -> its folder; typed dir -> itself
        pick(PickKind::Audio, song.to_str().unwrap());
        pick(PickKind::Audio, album.to_str().unwrap());
        // empty, with a remembered folder -> last
        e.store.set_last_import_dir(Some(music.clone())).unwrap();
        pick(PickKind::Audio, "");
        // remembered folder deleted -> home, and the setting is cleared
        let gone = e.base.join("gone");
        e.store.set_last_import_dir(Some(gone)).unwrap();
        pick(PickKind::Audio, "");
        assert_eq!(e.store.get().last_import_dir, None);
        // directory kind uses pick_folder
        ans(&[format!("import-directory {}", album.display())]);
        let got = pick(PickKind::Directory, "").unwrap();
        assert_eq!(
            (got.path.as_str(), got.name.as_str()),
            (album.to_str().unwrap(), "album")
        );
        assert_eq!(
            e.picker.starts(),
            vec![
                (D::ImportAudio, Some(album.clone())),
                (D::ImportAudio, Some(album.clone())),
                (D::ImportAudio, Some(music)),
                (D::ImportAudio, Some(home.clone())),
                (D::ImportDirectory, Some(home)),
            ]
        );
    }

    /// An import state whose ffprobe hangs, so an accepted job stays running until cancelled.
    fn slow_import(e: &Env) -> ImportState {
        use std::os::unix::fs::PermissionsExt;
        let bin = e.base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (n, body) in [
            ("ffmpeg", "echo ffmpeg version 7.1"),
            (
                "ffprobe",
                "if [ \"$1\" = -version ]; then echo ffprobe version 7.1; else sleep 30; fi",
            ),
        ] {
            let f = bin.join(n);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        ImportState::new(
            Tools::discover(bin.as_os_str()),
            e.st.repo_lock(),
            Duration::from_millis(10),
        )
    }

    #[test]
    fn accepted_imports_remember_the_folder() {
        let e = env("");
        do_list_tracks(&e.st, &e.default).unwrap();
        let import = slow_import(&e);
        let album = e.base.join("album");
        std::fs::create_dir_all(&album).unwrap();
        let song = album.join("song.mp3");
        std::fs::write(&song, b"x").unwrap();
        let notes = e.base.join("notes.txt");
        std::fs::write(&notes, b"x").unwrap();
        // a rejected path changes nothing
        do_import_path(
            &e.st,
            &import,
            &e.store,
            ImportKind::Audio,
            notes.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap_err();
        assert_eq!(e.store.get().last_import_dir, None);
        // an accepted file remembers its folder
        let snap = do_import_path(
            &e.st,
            &import,
            &e.store,
            ImportKind::Audio,
            song.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap();
        assert_eq!(e.store.get().last_import_dir, Some(album.clone()));
        let _ = import.cancel(&snap.job);
        // an accepted directory remembers the directory itself
        e.store.set_last_import_dir(None).unwrap();
        e.store.set_edge_ai_url("http://127.0.0.1:1").unwrap();
        let import = slow_import(&e);
        let snap = do_import_directory(
            &e.st,
            &import,
            &e.store,
            album.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap();
        assert_eq!(e.store.get().last_import_dir, Some(album));
        let _ = import.cancel(&snap.job);
    }

    #[test]
    fn import_directory_rejections() {
        let e = env("");
        let file = e.base.join("a.mp3");
        std::fs::write(&file, b"x").unwrap();
        let empty = e.base.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let go = |p: &str| {
            do_import_directory(&e.st, &e.import, &e.store, p, None, null_sink()).unwrap_err()
        };
        let missing = e.base.join("nope");
        assert_eq!(
            go(missing.to_str().unwrap()),
            format!("No folder found at {}", missing.display())
        );
        assert_eq!(
            go(file.to_str().unwrap()),
            format!("{} is not a folder", file.display())
        );
        assert_eq!(
            go(empty.to_str().unwrap()),
            format!("Directory {} has no audio files", empty.display())
        );
        assert_eq!(
            go(""),
            "Enter the path of a folder or choose one with Browse"
        );
        assert!(e.import.snapshot().is_none());
        assert_eq!(e.store.get().last_import_dir, None);
    }

    #[test]
    fn import_path_rejections_start_no_job() {
        let e = env("");
        let notes = e.base.join("notes.txt");
        let clip = e.base.join("clip.mp4");
        std::fs::write(&notes, b"x").unwrap();
        std::fs::write(&clip, b"x").unwrap();
        let go = |k, p: &str| {
            do_import_path(&e.st, &e.import, &e.store, k, p, None, null_sink()).unwrap_err()
        };
        let err = go(ImportKind::Audio, clip.to_str().unwrap());
        assert!(err.contains("not an audio file"), "{err}");
        let err = go(ImportKind::Video, notes.to_str().unwrap());
        assert!(err.contains("not a video file"), "{err}");
        let missing = e.base.join("nope.mp3");
        let err = go(ImportKind::Audio, missing.to_str().unwrap());
        assert_eq!(err, format!("No file found at {}", missing.display()));
        let err = go(ImportKind::Audio, "");
        assert_eq!(err, "Enter the path of a file or choose one with Browse");
        assert!(e.import.snapshot().is_none());
    }

    #[test]
    fn import_path_without_tools_is_an_error_not_a_job() {
        let e = env("");
        do_list_tracks(&e.st, &e.default).unwrap();
        let f = e.base.join("song.mp3");
        std::fs::write(&f, b"x").unwrap();
        let err = do_import_path(
            &e.st,
            &e.import,
            &e.store,
            ImportKind::Audio,
            f.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap_err();
        assert!(err.contains("ffmpeg"), "{err}");
        assert!(e.import.snapshot().is_none());
    }

    #[test]
    fn import_path_while_a_job_runs_is_busy() {
        use std::os::unix::fs::PermissionsExt;
        let e = env("");
        do_list_tracks(&e.st, &e.default).unwrap();
        let bin = e.base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (n, body) in [
            ("ffmpeg", "echo ffmpeg version 7.1"),
            (
                "ffprobe",
                "if [ \"$1\" = -version ]; then echo ffprobe version 7.1; else sleep 30; fi",
            ),
        ] {
            let f = bin.join(n);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tools = Tools::discover(bin.as_os_str());
        let import = ImportState::new(tools, e.st.repo_lock(), Duration::from_millis(10));
        let f = e.base.join("song.mp3");
        std::fs::write(&f, b"x").unwrap();
        let snap = do_import_path(
            &e.st,
            &import,
            &e.store,
            ImportKind::Audio,
            f.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap();
        let err = do_import_path(
            &e.st,
            &import,
            &e.store,
            ImportKind::Audio,
            f.to_str().unwrap(),
            None,
            null_sink(),
        )
        .unwrap_err();
        assert_eq!(err, crate::import_job::BUSY_MESSAGE);
        let _ = import.cancel(&snap.job);
    }

    #[test]
    fn import_path_expands_home() {
        let e = env("");
        std::fs::write(e.base.join("song.mp3"), b"x").unwrap();
        let err = do_import_path(
            &e.st,
            &e.import,
            &e.store,
            ImportKind::Audio,
            "~/song.mp3",
            Some(e.base.clone()),
            null_sink(),
        )
        .unwrap_err();
        assert!(err.contains("ffmpeg"), "{err}");
        let err = do_import_path(
            &e.st,
            &e.import,
            &e.store,
            ImportKind::Audio,
            "~/other.mp3",
            Some(e.base.clone()),
            null_sink(),
        )
        .unwrap_err();
        assert!(err.starts_with("No file found at "), "{err}");
    }

    #[test]
    fn unknown_job_ids_are_errors() {
        let e = env("");
        assert!(e.import.cancel("nope").is_err());
        assert!(e.import.discard("nope").is_err());
        assert!(e.import.snapshot().is_none());
        let edits = TrackEdits {
            band: "B".into(),
            album: "A".into(),
            title: "T".into(),
            composers: vec![],
            year: None,
            source_url: None,
            copyright: None,
        };
        assert!(do_start_extraction(&e.import, &e.store, "nope", edits).is_err());
    }

    #[test]
    fn prepare_url_rejects_bad_urls() {
        let e = env("");
        let p = do_prepare_url(&e.st, &e.import, "not a url");
        assert_eq!(p.status, crate::import_job::UrlPrepStatus::Invalid);
    }

    #[test]
    fn repo_lock_is_the_root_mutex() {
        let e = env("");
        let lock = e.st.repo_lock();
        let mut ran = false;
        lock.run(&mut || ran = true);
        assert!(ran);
    }

    // ---- editor command bodies (null audio backend; never a device) ----

    fn editor_state() -> EditorState {
        EditorState::new(Arc::new(crate::audio_out::NullBackend::new(None)))
    }

    fn null_editor_sink() -> EventSink {
        Arc::new(|_| {})
    }

    #[test]
    fn editor_commands_reject_unknown_sessions() {
        let st = editor_state();
        assert!(st.mgr().current().is_none());
        assert!(do_watch_editor(&st, null_editor_sink()).is_none());
        assert_eq!(st.mgr().play("x").unwrap_err(), "no editor session for x");
        assert!(do_editor_seek(&st, "x", 1000)
            .unwrap_err()
            .contains("no editor session"));
        assert!(do_editor_nudge(&st, "x", -100)
            .unwrap_err()
            .contains("no editor session"));
        assert!(st
            .mgr()
            .set_stem("x", "drums", None, true, std::time::Instant::now())
            .is_err());
        do_close_editor(&st);
        st.tick(std::time::Instant::now());
    }

    #[test]
    fn open_editor_of_unknown_track_fails_and_save_needs_a_session() {
        let e = env("");
        let st = editor_state();
        assert!(do_open_editor(
            &st,
            &e.st,
            &FileTempoStore,
            "0199b0a0-0000-7000-8000-00000000dead",
            null_editor_sink()
        )
        .is_err());
        assert!(do_save_backing(&st, &e.st, "x")
            .unwrap_err()
            .contains("no editor session"));
    }

    #[test]
    fn events_follow_the_latest_sink() {
        let st = editor_state();
        let got = Arc::new(Mutex::new(0usize));
        let g = got.clone();
        do_watch_editor(&st, Arc::new(move |_| *g.lock().unwrap() += 1));
        let sink = st.mgr().sink();
        sink(EditorEvent::SaveCancelled { id: "x".into() });
        assert_eq!(*got.lock().unwrap(), 1);
        let g2 = got.clone();
        do_watch_editor(&st, Arc::new(move |_| *g2.lock().unwrap() += 10));
        sink(EditorEvent::SaveCancelled { id: "x".into() });
        assert_eq!(*got.lock().unwrap(), 11);
    }
}

#[cfg(test)]
#[path = "editor_ipc_tests.rs"]
mod editor_ipc_tests;
