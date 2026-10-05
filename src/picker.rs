//! Native file dialogs (opened from Rust only) and the opaque token registry.
//!
//! The frontend never sees or sends a real path for a picked file: a pick registers the path
//! here and returns a token; later commands accept the token only.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Which dialog a pick or a token belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DialogKind {
    AddTablature,
    UpdateTablature,
    RepositoryRoot,
    ExportTrack,
    ExportTablature,
}

impl DialogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DialogKind::AddTablature => "add-tablature",
            DialogKind::UpdateTablature => "update-tablature",
            DialogKind::RepositoryRoot => "repository-root",
            DialogKind::ExportTrack => "export-track",
            DialogKind::ExportTablature => "export-tablature",
        }
    }

    #[cfg(any(test, feature = "e2e-hooks"))]
    fn parse(s: &str) -> Option<Self> {
        [
            DialogKind::AddTablature,
            DialogKind::UpdateTablature,
            DialogKind::RepositoryRoot,
            DialogKind::ExportTrack,
            DialogKind::ExportTablature,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

/// Extensions accepted for tablature files (lowercase, no dot).
pub const TABLATURE_EXTENSIONS: &[&str] =
    &["gp", "gp3", "gp4", "gp5", "gpx", "tg", "ptb", "musicxml", "mxl", "xml"];

/// True when the file name has a tablature extension (case-insensitive).
pub fn is_tablature_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            let ext = ext.to_ascii_lowercase();
            TABLATURE_EXTENSIONS.contains(&ext.as_str())
        }
        _ => false,
    }
}

/// Logs the line the e2e tests wait on.
pub fn log_dialog(kind: DialogKind, picked: bool) {
    eprintln!(
        "calliope: dialog kind={} result={}",
        kind.as_str(),
        if picked { "picked" } else { "cancelled" }
    );
}

#[derive(Debug, Clone)]
pub struct FileReq {
    pub kind: DialogKind,
    pub title: String,
    /// Filter name and extensions; empty extensions means no filter.
    pub filter_name: String,
    pub extensions: Vec<String>,
    pub start_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FolderReq {
    pub kind: DialogKind,
    pub title: String,
    pub start_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SaveReq {
    pub kind: DialogKind,
    pub title: String,
    pub default_name: String,
    pub filter_name: String,
    pub extensions: Vec<String>,
    pub start_dir: Option<PathBuf>,
}

/// Blocking dialogs. Call from `spawn_blocking`, never from the main thread.
pub trait Picker: Send + Sync {
    fn pick_file(&self, req: &FileReq) -> Option<PathBuf>;
    fn pick_folder(&self, req: &FolderReq) -> Option<PathBuf>;
    fn save_file(&self, req: &SaveReq) -> Option<PathBuf>;
}

/// Token -> (kind, path). Tokens are `p<counter>` and live until taken or the app exits.
#[derive(Default)]
pub struct PickRegistry {
    inner: Mutex<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    next: u64,
    map: HashMap<String, (DialogKind, PathBuf)>,
}

impl PickRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert(&self, kind: DialogKind, path: PathBuf) -> String {
        let mut g = self.lock();
        g.next += 1;
        let token = format!("p{}", g.next);
        g.map.insert(token.clone(), (kind, path));
        token
    }

    /// The path for a token of this kind; `None` for an unknown token or another kind.
    pub fn get(&self, kind: DialogKind, token: &str) -> Option<PathBuf> {
        match self.lock().map.get(token) {
            Some((k, p)) if *k == kind => Some(p.clone()),
            _ => None,
        }
    }

    /// Like `get`, but removes the token (single use). A token of another kind stays.
    pub fn take(&self, kind: DialogKind, token: &str) -> Option<PathBuf> {
        let mut g = self.lock();
        match g.map.get(token) {
            Some((k, _)) if *k == kind => g.map.remove(token).map(|(_, p)| p),
            _ => None,
        }
    }
}

/// The real dialogs via tauri-plugin-dialog.
pub struct TauriPicker {
    app: tauri::AppHandle,
}

impl TauriPicker {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn builder(&self) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
        use tauri::Manager;
        use tauri_plugin_dialog::DialogExt;
        let b = self.app.dialog().file();
        match self.app.get_webview_window("main") {
            Some(w) => b.set_parent(&w),
            None => b,
        }
    }
}

fn into_path(fp: Option<tauri_plugin_dialog::FilePath>) -> Option<PathBuf> {
    fp.and_then(|f| f.into_path().ok())
}

impl Picker for TauriPicker {
    fn pick_file(&self, req: &FileReq) -> Option<PathBuf> {
        let mut b = self.builder().set_title(&req.title);
        if !req.extensions.is_empty() {
            let exts: Vec<&str> = req.extensions.iter().map(String::as_str).collect();
            b = b.add_filter(&req.filter_name, &exts);
        }
        if let Some(d) = &req.start_dir {
            b = b.set_directory(d);
        }
        let r = into_path(b.blocking_pick_file());
        log_dialog(req.kind, r.is_some());
        r
    }

    fn pick_folder(&self, req: &FolderReq) -> Option<PathBuf> {
        let mut b = self.builder().set_title(&req.title);
        if let Some(d) = &req.start_dir {
            b = b.set_directory(d);
        }
        let r = into_path(b.blocking_pick_folder());
        log_dialog(req.kind, r.is_some());
        r
    }

    fn save_file(&self, req: &SaveReq) -> Option<PathBuf> {
        let mut b = self.builder().set_title(&req.title).set_file_name(&req.default_name);
        if !req.extensions.is_empty() {
            let exts: Vec<&str> = req.extensions.iter().map(String::as_str).collect();
            b = b.add_filter(&req.filter_name, &exts);
        }
        if let Some(d) = &req.start_dir {
            b = b.set_directory(d);
        }
        let r = into_path(b.blocking_save_file());
        log_dialog(req.kind, r.is_some());
        r
    }
}

/// Test-only picker driven by an answers file. Compiled only with the `e2e-hooks` feature
/// (and in unit tests).
#[cfg(any(test, feature = "e2e-hooks"))]
pub struct ScriptedPicker {
    answers: std::path::PathBuf,
    lock: Mutex<()>,
}

#[cfg(any(test, feature = "e2e-hooks"))]
impl ScriptedPicker {
    pub fn new(answers: PathBuf) -> Self {
        Self { answers, lock: Mutex::new(()) }
    }

    /// Reads `CALLIOPE_E2E_DIALOG_ANSWERS`; `None` when it is unset or empty.
    pub fn from_env() -> Option<Self> {
        let v = std::env::var_os("CALLIOPE_E2E_DIALOG_ANSWERS")?;
        if v.is_empty() {
            return None;
        }
        Some(Self::new(PathBuf::from(v)))
    }

    fn answer(&self, kind: DialogKind) -> Option<PathBuf> {
        let r = self.pop(kind);
        log_dialog(kind, r.is_some());
        r
    }

    fn pop(&self, kind: DialogKind) -> Option<PathBuf> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mismatch = |why: &str| {
            eprintln!("calliope: e2e dialog script mismatch kind={} ({why})", kind.as_str());
            None
        };
        let Ok(text) = std::fs::read_to_string(&self.answers) else {
            return mismatch("answers file missing");
        };
        let mut lines = text.lines();
        let Some(first) = lines.next() else {
            return mismatch("no answers left");
        };
        let rest: Vec<&str> = lines.collect();
        let mut new = rest.join("\n");
        if !rest.is_empty() {
            new.push('\n');
        }
        if crate::fsutil::write_atomic(&self.answers, new.as_bytes()).is_err() {
            return mismatch("cannot rewrite answers file");
        }
        let (k, arg) = first.split_once(' ').unwrap_or((first, ""));
        if DialogKind::parse(k) != Some(kind) {
            return mismatch(&format!("expected {k}"));
        }
        if arg == "CANCEL" {
            return None;
        }
        let p = PathBuf::from(arg);
        if !p.is_absolute() {
            return mismatch("path is not absolute");
        }
        Some(p)
    }
}

#[cfg(any(test, feature = "e2e-hooks"))]
impl Picker for ScriptedPicker {
    fn pick_file(&self, req: &FileReq) -> Option<PathBuf> {
        self.answer(req.kind)
    }
    fn pick_folder(&self, req: &FolderReq) -> Option<PathBuf> {
        self.answer(req.kind)
    }
    fn save_file(&self, req: &SaveReq) -> Option<PathBuf> {
        self.answer(req.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_other_kind_and_unknown() {
        let r = PickRegistry::new();
        let t = r.insert(DialogKind::AddTablature, "/x/a.gp5".into());
        assert_eq!(r.get(DialogKind::RepositoryRoot, &t), None);
        assert_eq!(r.take(DialogKind::RepositoryRoot, &t), None);
        assert_eq!(r.get(DialogKind::AddTablature, "nope"), None);
        assert_eq!(r.get(DialogKind::AddTablature, &t), Some("/x/a.gp5".into()));
    }

    #[test]
    fn take_is_single_use_and_tokens_are_unique() {
        let r = PickRegistry::new();
        let a = r.insert(DialogKind::RepositoryRoot, "/a".into());
        let b = r.insert(DialogKind::RepositoryRoot, "/b".into());
        assert_ne!(a, b);
        assert_eq!(r.take(DialogKind::RepositoryRoot, &a), Some("/a".into()));
        assert_eq!(r.take(DialogKind::RepositoryRoot, &a), None);
        assert_eq!(r.get(DialogKind::RepositoryRoot, &b), Some("/b".into()));
    }

    #[test]
    fn tablature_names() {
        assert!(is_tablature_name("riff.gp5"));
        assert!(is_tablature_name("Riff.GPX"));
        assert!(is_tablature_name("a.b.musicxml"));
        assert!(!is_tablature_name("notes.txt"));
        assert!(!is_tablature_name("gp5"));
        assert!(!is_tablature_name(".gp5"));
        assert!(!is_tablature_name("riff"));
    }

    fn file_req(kind: DialogKind) -> FileReq {
        FileReq {
            kind,
            title: "t".into(),
            filter_name: "f".into(),
            extensions: vec![],
            start_dir: None,
        }
    }

    #[test]
    fn scripted_pops_in_order_and_handles_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("answers");
        std::fs::write(
            &f,
            "add-tablature /tmp/a.gp5\nrepository-root CANCEL\nexport-track /tmp/out\nexport-track /tmp/x\n",
        )
        .unwrap();
        let p = ScriptedPicker::new(f.clone());
        assert_eq!(p.pick_file(&file_req(DialogKind::AddTablature)), Some("/tmp/a.gp5".into()));
        let folder = |kind| FolderReq { kind, title: "t".into(), start_dir: None };
        assert_eq!(p.pick_folder(&folder(DialogKind::RepositoryRoot)), None);
        // mismatch: the line is consumed, result is a cancel
        assert_eq!(p.pick_folder(&folder(DialogKind::RepositoryRoot)), None);
        assert_eq!(p.pick_folder(&folder(DialogKind::ExportTrack)), Some("/tmp/x".into()));
        // empty file
        assert_eq!(p.pick_folder(&folder(DialogKind::ExportTrack)), None);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "");
    }

    #[test]
    fn scripted_missing_file_and_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = ScriptedPicker::new(dir.path().join("none"));
        assert_eq!(p.pick_file(&file_req(DialogKind::AddTablature)), None);
        let f = dir.path().join("a");
        std::fs::write(&f, "add-tablature rel/x.gp5\n").unwrap();
        let p = ScriptedPicker::new(f);
        assert_eq!(p.pick_file(&file_req(DialogKind::AddTablature)), None);
    }
}
