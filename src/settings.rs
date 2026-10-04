//! User settings persisted as JSON. Pure logic, no Tauri.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
}

/// Missing file gives the default. Each field is read leniently: an invalid value falls back
/// to that field's default (with a warning) and the other fields are kept. A file that is not
/// a JSON object is moved to `<path>.bak` (replacing an older one) before defaults are used.
pub fn load(path: &Path) -> Settings {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
        Err(e) => {
            eprintln!("calliope: cannot read settings file {}: {e}", path.display());
            return Settings::default();
        }
    };
    let obj = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(o)) => o,
        other => {
            let why = match other {
                Ok(_) => "not a JSON object".to_string(),
                Err(e) => e.to_string(),
            };
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            let bak = PathBuf::from(bak);
            match std::fs::rename(path, &bak) {
                Ok(()) => eprintln!(
                    "calliope: invalid settings file {} ({why}); moved to {}, using defaults",
                    path.display(),
                    bak.display()
                ),
                Err(e) => eprintln!(
                    "calliope: invalid settings file {} ({why}); could not back it up to {}: {e}",
                    path.display(),
                    bak.display()
                ),
            }
            return Settings::default();
        }
    };
    let mut s = Settings::default();
    if let Some(v) = obj.get("theme") {
        match serde_json::from_value::<Theme>(v.clone()) {
            Ok(t) => s.theme = t,
            Err(e) => eprintln!(
                "calliope: invalid theme in {}: {e}; using default",
                path.display()
            ),
        }
    }
    s
}

/// Creates parent directories, writes `<path>.tmp` (fsynced), then renames it (atomic) and
/// best-effort fsyncs the directory on Unix.
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        let dir = parent.unwrap_or_else(|| Path::new("."));
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn open(path: PathBuf) -> Self {
        let current = Mutex::new(load(&path));
        Self { path, current }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_theme(&self, theme: Theme) -> std::io::Result<Settings> {
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = cur.clone();
        next.theme = theme;
        save(&self.path, &next)?;
        *cur = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light() -> Settings {
        Settings { theme: Theme::Light }
    }

    #[test]
    fn default_is_dark() {
        assert_eq!(Settings::default().theme, Theme::Dark);
    }

    #[test]
    fn json_shape() {
        assert_eq!(serde_json::to_string(&light()).unwrap(), r#"{"theme":"light"}"#);
        let s: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
        assert_eq!(s, light());
    }

    #[test]
    fn unknown_ignored_missing_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme":"light","x":1}"#).unwrap();
        assert_eq!(s, light());
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn load_missing_is_default() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(&d.path().join("nope.json")), Settings::default());
    }

    #[test]
    fn load_garbage_is_default_and_backed_up() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        let bak = d.path().join("s.json.bak");
        std::fs::write(&bak, "older").unwrap();
        std::fs::write(&p, "not json {{").unwrap();
        assert_eq!(load(&p), Settings::default());
        assert_eq!(std::fs::read(&bak).unwrap(), b"not json {{");
        assert!(!p.exists());
        save(&p, &light()).unwrap();
        assert_eq!(std::fs::read(&bak).unwrap(), b"not json {{");
        assert_eq!(load(&p), light());
        assert!(!d.path().join("s.json.tmp").exists());
    }

    #[test]
    fn non_object_is_backed_up() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        std::fs::write(&p, "[1,2]").unwrap();
        assert_eq!(load(&p), Settings::default());
        assert_eq!(std::fs::read(d.path().join("s.json.bak")).unwrap(), b"[1,2]");
    }

    #[test]
    fn invalid_theme_falls_back_file_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        std::fs::write(&p, r#"{"theme":"purple","extra":true}"#).unwrap();
        assert_eq!(load(&p), Settings::default());
        assert!(p.exists());
        assert!(!d.path().join("s.json.bak").exists());
        std::fs::write(&p, r#"{"theme":7}"#).unwrap();
        assert_eq!(load(&p).theme, Theme::Dark);
    }

    #[test]
    fn save_load_roundtrip_creates_parents() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/b/s.json");
        save(&p, &light()).unwrap();
        assert_eq!(load(&p), light());
    }

    #[test]
    fn save_leaves_no_tmp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        save(&p, &light()).unwrap();
        assert!(!d.path().join("s.json.tmp").exists());
    }

    #[test]
    fn store_set_theme_persists() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cfg/s.json");
        let store = SettingsStore::open(p.clone());
        assert_eq!(store.get().theme, Theme::Dark);
        assert_eq!(store.set_theme(Theme::Light).unwrap(), light());
        assert_eq!(store.get(), light());
        assert_eq!(SettingsStore::open(p).get(), light());
    }
}
