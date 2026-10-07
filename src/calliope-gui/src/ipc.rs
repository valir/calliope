//! Tauri commands: thin wrappers over pure modules, registered in `gui.rs`.
//!
//! The frontend never sends a real path: files picked in native dialogs are registered in the
//! `PickRegistry` and referred to by token, ids and names are validated by `repository`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::import_job::{FileKind, ImportEvent, ImportState, JobSnapshot, RepoLock, Sink, UrlPrep};
use crate::picker::{
    DialogKind, FileReq, FolderReq, PickRegistry, Picker, SaveReq, AUDIO_EXTENSIONS,
    TABLATURE_EXTENSIONS, VIDEO_EXTENSIONS,
};
use crate::tools::{Tools, ToolState, ToolStatus};
use crate::track_meta::TrackEdits;
use calliope_lib::stems_client::{ClientError, StemsClient};
use crate::repository::{self, Library, RepoStatus, Repository, SaveResult, SaveTrackRequest};
use crate::settings::{self, Settings, SettingsStore, Theme};

/// Managed app state: the current repository root (the lock serialises every repository
/// operation), the pick-token registry and the dialog implementation.
pub struct RepoState {
    root: Arc<Mutex<PathBuf>>,
    pub registry: PickRegistry,
    pub picker: Box<dyn Picker>,
}

impl RepoState {
    pub fn new(root: PathBuf, picker: Box<dyn Picker>) -> Self {
        Self { root: Arc::new(Mutex::new(root)), registry: PickRegistry::new(), picker }
    }

    /// The repository lock for the import jobs: the same mutex that serialises every other
    /// repository operation.
    pub fn repo_lock(&self) -> Arc<dyn RepoLock> {
        Arc::new(RootLock(self.root.clone()))
    }

    /// The current root (the lock is not held afterwards).
    fn current_root(&self) -> PathBuf {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, PathBuf> {
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
    Some(PickedRoot { token, root, status })
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
    let setting = if path == default_root { None } else { Some(path.clone()) };
    store.set_repository_root(setting).map_err(|e| format!("could not save settings: {e}"))?;
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
    store.set_repository_root(None).map_err(|e| format!("could not save settings: {e}"))?;
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
    let result = Repository::new(root.clone()).save_track(request, &lookup)?;
    for t in &tokens {
        st.registry.take(DialogKind::AddTablature, t);
        st.registry.take(DialogKind::UpdateTablature, t);
    }
    Ok(result)
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
    ToolsInfo { yt_dlp: tool_info(s.yt_dlp), ffmpeg: tool_info(s.ffmpeg), ffprobe: tool_info(s.ffprobe) }
}

/// Asks the configured server for its health (blocking, up to the client's connect timeout).
pub fn do_check_edge_ai(url: Option<&str>) -> EdgeAiStatus {
    let mk = |state, message: String, models| EdgeAiStatus { state, message, models };
    let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else {
        return mk(EdgeAiState::NotConfigured, "Not configured".into(), vec![]);
    };
    match StemsClient::new(url).health() {
        Ok(h) => mk(
            EdgeAiState::Connected,
            format!("Connected (calliope-stems {}, model {})", h.version, h.default_model),
            h.models,
        ),
        Err(e @ ClientError::Incompatible(_)) => mk(EdgeAiState::Incompatible, e.to_string(), vec![]),
        Err(e) => mk(EdgeAiState::Unreachable, e.to_string(), vec![]),
    }
}

pub fn do_set_edge_ai_url(store: &SettingsStore, url: Option<&str>) -> Result<Settings, String> {
    store.set_edge_ai_url(url.unwrap_or(""))
}

pub fn do_set_keep_original(store: &SettingsStore, keep: bool) -> Result<Settings, String> {
    store.set_keep_original(keep).map_err(|e| format!("could not save settings: {e}"))
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

/// Opens the file dialog for `kind` (the path stays in Rust) and starts the import.
/// `Ok(None)` when the dialog was cancelled.
pub fn do_import_file(
    st: &RepoState,
    import: &ImportState,
    kind: ImportKind,
    home: Option<PathBuf>,
    sink: Sink,
) -> Result<Option<JobSnapshot>, String> {
    if import.is_active() {
        return Err(crate::import_job::BUSY_MESSAGE.into());
    }
    let (dialog, file_kind, title, filter, exts, label) = match kind {
        ImportKind::Audio => {
            (DialogKind::ImportAudio, FileKind::Audio, "Choose an audio file", "Audio files", AUDIO_EXTENSIONS, "an audio")
        }
        ImportKind::Video => {
            (DialogKind::ImportVideo, FileKind::Video, "Choose a video file", "Video files", VIDEO_EXTENSIONS, "a video")
        }
    };
    let req = FileReq {
        kind: dialog,
        title: title.into(),
        filter_name: filter.into(),
        extensions: exts.iter().map(|s| s.to_string()).collect(),
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
    let right = match kind {
        ImportKind::Audio => crate::picker::is_audio_name(&name),
        ImportKind::Video => crate::picker::is_video_name(&name),
    };
    if !right {
        return Err(format!("\"{name}\" is not {label} file"));
    }
    import.start_file(&st.current_root(), file_kind, &path, sink).map(Some)
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
    eprintln!("calliope: edge-ai state={}", serde_json::to_string(&st.state).unwrap_or_default().trim_matches('"'));
}

fn channel_sink(events: tauri::ipc::Channel<ImportEvent>) -> Sink {
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
pub async fn set_edge_ai_url(app: tauri::AppHandle, url: Option<String>) -> Result<Settings, String> {
    blocking(move || do_set_edge_ai_url(&app.state::<SettingsStore>(), url.as_deref())).await
}

#[tauri::command]
pub async fn set_keep_original(app: tauri::AppHandle, keep: bool) -> Result<Settings, String> {
    blocking(move || do_set_keep_original(&app.state::<SettingsStore>(), keep)).await
}

#[tauri::command]
pub async fn check_edge_ai(app: tauri::AppHandle) -> Result<EdgeAiStatus, String> {
    blocking(move || Ok(do_check_edge_ai(app.state::<SettingsStore>().get().edge_ai_url.as_deref())))
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
    blocking(move || Ok(do_prepare_url(&app.state::<RepoState>(), &app.state::<ImportState>(), &url)))
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
pub async fn import_file(
    app: tauri::AppHandle,
    kind: ImportKind,
    events: tauri::ipc::Channel<ImportEvent>,
) -> Result<Option<JobSnapshot>, String> {
    blocking(move || {
        do_import_file(
            &app.state::<RepoState>(),
            &app.state::<ImportState>(),
            kind,
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
        do_start_extraction(&app.state::<ImportState>(), &app.state::<SettingsStore>(), &job, edits)
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
pub fn app_version() -> String {
    crate::VERSION.to_string()
}

#[tauri::command]
pub fn get_settings(store: tauri::State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
pub fn set_theme(
    store: tauri::State<'_, SettingsStore>,
    theme: Theme,
) -> Result<Settings, String> {
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
    }

    fn env(answers: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let answers_path = base.join("answers");
        std::fs::write(&answers_path, answers).unwrap();
        let default = base.join("data/calliope");
        let st = RepoState::new(default.clone(), Box::new(ScriptedPicker::new(answers_path.clone())));
        let store = SettingsStore::open(base.join("settings.json"));
        let import = ImportState::new(Tools::default(), st.repo_lock(), Duration::from_millis(10));
        Env { _dir: dir, base, st, store, import, default, answers: answers_path }
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
        let st = RepoState::new(missing.clone(), Box::new(ScriptedPicker::new(e.answers.clone())));
        let lib = do_list_tracks(&st, &e.default).unwrap();
        assert!(lib.tracks.is_empty());
        assert!(!missing.exists());
        assert_eq!(do_get_repository(&st, &e.default).status, RepoStatus::Missing);
    }

    #[test]
    fn choose_then_set_root_uses_token_once_and_saves_setting() {
        let e = env("");
        let chosen = e.base.join("mylib");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::write(&e.answers, format!("repository-root {}\n", chosen.display())).unwrap();
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
        std::fs::write(chosen.join("calliope-repository.json"), "{\"schema_version\": 9}").unwrap();
        std::fs::write(&e.answers, format!("repository-root {}\n", chosen.display())).unwrap();
        let p = do_choose_root(&e.st, None).unwrap();
        assert_eq!(p.status, RepoStatus::Newer);
        assert!(do_set_root(&e.st, &e.store, &e.import, &e.default, &p.token).unwrap_err().contains("newer"));
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
            format!("add-tablature {}\nadd-tablature {}\nupdate-tablature CANCEL\n", good.display(), bad.display()),
        )
        .unwrap();
        let p = do_pick_tablature(&e.st, PickPurpose::Add, None).unwrap().unwrap();
        assert_eq!(p.name, "riff.gp5");
        assert!(do_pick_tablature(&e.st, PickPurpose::Add, None).is_err());
        assert!(do_pick_tablature(&e.st, PickPurpose::Update, None).unwrap().is_none());
    }

    #[test]
    fn save_with_token_then_export_and_delete() {
        let e = env("");
        let rec = seed(&e);
        let tab = e.base.join("riff.gp5");
        std::fs::write(&tab, b"gp").unwrap();
        std::fs::write(&e.answers, format!("add-tablature {}\n", tab.display())).unwrap();
        let p = do_pick_tablature(&e.st, PickPurpose::Add, None).unwrap().unwrap();
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
            tablatures: vec![TabEntry::Add { token: p.token.clone() }],
        };
        let res = do_save_track(&e.st, req.clone()).unwrap();
        assert_eq!(res.track.tablatures, vec!["riff.gp5"]);
        // the token was consumed
        assert!(e.st.registry.get(DialogKind::AddTablature, &p.token).is_none());

        let out = e.base.join("out");
        std::fs::create_dir(&out).unwrap();
        let tabout = e.base.join("out/riff-copy.gp5");
        std::fs::write(
            &e.answers,
            format!("export-track {}\nexport-tablature {}\nexport-track CANCEL\n", out.display(), tabout.display()),
        )
        .unwrap();
        let dest = do_export_track(&e.st, &rec.id, None).unwrap().unwrap();
        assert!(std::path::Path::new(&dest).join("track.json").is_file());
        assert!(do_export_tablature(&e.st, &rec.id, "riff.gp5", None).unwrap().is_some());
        assert_eq!(std::fs::read(&tabout).unwrap(), b"gp");
        assert!(do_export_track(&e.st, &rec.id, None).unwrap().is_none());

        // stale revision is a conflict, ids are validated
        assert!(do_delete_track(&e.st, &rec.id, &rec.revision).unwrap_err().contains("conflict"));
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
        assert_eq!(do_check_edge_ai(Some("  ")).state, EdgeAiState::NotConfigured);
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
        assert_eq!(do_set_edge_ai_url(&e.store, None).unwrap().edge_ai_url, None);
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
    fn import_file_cancel_and_wrong_extension() {
        let e = env("");
        let bad = e.base.join("notes.txt");
        let video_as_audio = e.base.join("clip.mp4");
        std::fs::write(
            &e.answers,
            format!(
                "import-audio CANCEL\nimport-audio {}\nimport-video {}\n",
                video_as_audio.display(),
                bad.display()
            ),
        )
        .unwrap();
        assert!(do_import_file(&e.st, &e.import, ImportKind::Audio, None, null_sink()).unwrap().is_none());
        let err = do_import_file(&e.st, &e.import, ImportKind::Audio, None, null_sink()).unwrap_err();
        assert!(err.contains("not an audio file"), "{err}");
        let err = do_import_file(&e.st, &e.import, ImportKind::Video, None, null_sink()).unwrap_err();
        assert!(err.contains("not a video file"), "{err}");
    }

    #[test]
    fn import_file_without_tools_is_an_error_not_a_job() {
        let e = env("");
        do_list_tracks(&e.st, &e.default).unwrap();
        let f = e.base.join("song.mp3");
        std::fs::write(&f, b"x").unwrap();
        std::fs::write(&e.answers, format!("import-audio {}\n", f.display())).unwrap();
        let err = do_import_file(&e.st, &e.import, ImportKind::Audio, None, null_sink()).unwrap_err();
        assert!(err.contains("ffmpeg"), "{err}");
        assert!(e.import.snapshot().is_none());
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
}
