//! Tauri commands: thin wrappers over pure modules, registered in `gui.rs`.
//!
//! The frontend never sends a real path: files picked in native dialogs are registered in the
//! `PickRegistry` and referred to by token, ids and names are validated by `repository`.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::picker::{
    DialogKind, FileReq, FolderReq, PickRegistry, Picker, SaveReq, TABLATURE_EXTENSIONS,
};
use crate::repository::{self, Library, RepoStatus, Repository, SaveResult, SaveTrackRequest};
use crate::settings::{self, Settings, SettingsStore, Theme};

/// Managed app state: the current repository root (the lock serialises every repository
/// operation), the pick-token registry and the dialog implementation.
pub struct RepoState {
    root: Mutex<PathBuf>,
    pub registry: PickRegistry,
    pub picker: Box<dyn Picker>,
}

impl RepoState {
    pub fn new(root: PathBuf, picker: Box<dyn Picker>) -> Self {
        Self { root: Mutex::new(root), registry: PickRegistry::new(), picker }
    }

    fn lock(&self) -> MutexGuard<'_, PathBuf> {
        self.root.lock().unwrap_or_else(|e| e.into_inner())
    }
}

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
    default_root: &std::path::Path,
    token: &str,
) -> Result<RepoInfo, String> {
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
    default_root: &std::path::Path,
) -> Result<RepoInfo, String> {
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
        do_set_root(&app.state::<RepoState>(), &app.state::<SettingsStore>(), &d, &token)
    })
    .await
}

#[tauri::command]
pub async fn reset_repository_root(app: tauri::AppHandle) -> Result<RepoInfo, String> {
    let d = default_root(&app)?;
    blocking(move || do_reset_root(&app.state::<RepoState>(), &app.state::<SettingsStore>(), &d))
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
        Env { _dir: dir, base, st, store, default, answers: answers_path }
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
        assert!(do_set_root(&e.st, &e.store, &e.default, "bogus").is_err());
        let info = do_set_root(&e.st, &e.store, &e.default, &p.token).unwrap();
        assert_eq!(info.status, RepoStatus::Ok);
        assert!(!info.is_default);
        assert_eq!(e.store.get().repository_root, Some(chosen.clone()));
        assert!(chosen.join("tracks").is_dir());
        assert!(do_set_root(&e.st, &e.store, &e.default, &p.token).is_err());
        let info = do_reset_root(&e.st, &e.store, &e.default).unwrap();
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
        assert!(do_set_root(&e.st, &e.store, &e.default, &p.token).unwrap_err().contains("newer"));
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
}
