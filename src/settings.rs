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

/// Missing file gives the default; unreadable or invalid gives the default plus a warning.
pub fn load(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("calliope: invalid settings file {}: {e}", path.display());
            Settings::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(e) => {
            eprintln!("calliope: cannot read settings file {}: {e}", path.display());
            Settings::default()
        }
    }
}

/// Creates parent directories, writes `<path>.tmp`, then renames it (atomic).
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
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
    fn load_garbage_is_default() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        std::fs::write(&p, "not json {{").unwrap();
        assert_eq!(load(&p), Settings::default());
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
