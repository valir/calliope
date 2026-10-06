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
    /// Absolute path of the tablature repository; `None` = the default location.
    pub repository_root: Option<PathBuf>,
    /// Base URL of the calliope-stems server, `http://host[:port][/path]` (see
    /// `normalize_edge_ai_url`); `None` = not configured.
    pub edge_ai_url: Option<String>,
    /// Keep the original mix (as FLAC) next to the extracted stems.
    pub keep_original: bool,
}

/// Longest accepted `edge_ai_url`.
pub const MAX_EDGE_AI_URL_LEN: usize = 200;

/// Checks and normalises a server URL: only `http://` (the server is on the home LAN), a host,
/// no userinfo, query or fragment, at most 200 characters; trailing slashes are dropped.
/// Blank input means "not configured" (`Ok(None)`).
pub fn normalize_edge_ai_url(input: &str) -> Result<Option<String>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    if input.len() > MAX_EDGE_AI_URL_LEN {
        return Err(format!("The server address is longer than {MAX_EDGE_AI_URL_LEN} characters"));
    }
    // The URL parser is lenient (`http:///x` becomes `http://x/`, `\` becomes `/`): be strict.
    if input.chars().any(|c| c.is_whitespace() || c == '\\' || c.is_control()) {
        return Err("The server address must not contain spaces or backslashes".into());
    }
    if input.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("http://")) && input[7..].starts_with('/') {
        return Err("The server address needs a host name".into());
    }
    let url = url::Url::parse(input).map_err(|e| format!("Not a valid server address: {e}"))?;
    match url.scheme() {
        "http" => {}
        "https" => return Err("Only http:// is supported on the LAN".into()),
        _ => return Err("The server address must start with http://".into()),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The server address needs a host name".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The server address must not contain a user name or password".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The server address must not contain ? or #".into());
    }
    let text = url.as_str().trim_end_matches('/');
    if text.len() > MAX_EDGE_AI_URL_LEN {
        return Err(format!("The server address is longer than {MAX_EDGE_AI_URL_LEN} characters"));
    }
    Ok(Some(text.to_string()))
}

/// The repository root used when the setting is `None`.
pub fn default_repository_root(data_dir: &Path) -> PathBuf {
    data_dir.join("calliope")
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
    match obj.get("repository_root") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(r)) if Path::new(r).is_absolute() => {
            s.repository_root = Some(PathBuf::from(r));
        }
        Some(v) => eprintln!(
            "calliope: invalid repository_root {v} in {} (not an absolute path string); using default",
            path.display()
        ),
    }
    match obj.get("edge_ai_url") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(u)) => match normalize_edge_ai_url(u) {
            Ok(url) => s.edge_ai_url = url,
            Err(e) => eprintln!("calliope: invalid edge_ai_url in {} ({e}); using default", path.display()),
        },
        Some(v) => eprintln!("calliope: invalid edge_ai_url {v} in {} (not a string); using default", path.display()),
    }
    match obj.get("keep_original") {
        None => {}
        Some(serde_json::Value::Bool(b)) => s.keep_original = *b,
        Some(v) => eprintln!("calliope: invalid keep_original {v} in {} (not a boolean); using default", path.display()),
    }
    s
}

/// Creates parent directories and writes the file atomically (see `fsutil::write_atomic`).
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    crate::fsutil::write_atomic(path, json.as_bytes())
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

    pub fn set_repository_root(&self, root: Option<PathBuf>) -> std::io::Result<Settings> {
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = cur.clone();
        next.repository_root = root;
        save(&self.path, &next)?;
        *cur = next.clone();
        Ok(next)
    }

    /// Validates (`normalize_edge_ai_url`), stores and saves the server URL; blank = `None`.
    pub fn set_edge_ai_url(&self, url: &str) -> Result<Settings, String> {
        let url = normalize_edge_ai_url(url)?;
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = cur.clone();
        next.edge_ai_url = url;
        save(&self.path, &next).map_err(|e| format!("Cannot save the settings: {e}"))?;
        *cur = next.clone();
        Ok(next)
    }

    pub fn set_keep_original(&self, keep: bool) -> std::io::Result<Settings> {
        let mut cur = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = cur.clone();
        next.keep_original = keep;
        save(&self.path, &next)?;
        *cur = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light() -> Settings {
        Settings { theme: Theme::Light, ..Settings::default() }
    }

    #[test]
    fn default_is_dark() {
        assert_eq!(Settings::default().theme, Theme::Dark);
    }

    #[test]
    fn json_shape() {
        assert_eq!(serde_json::to_string(&light()).unwrap(), r#"{"theme":"light","repository_root":null,"edge_ai_url":null,"keep_original":false}"#);
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

    #[test]
    fn relative_or_non_string_root_defaults_theme_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        for bad in [r#""rel/path""#, "5", "[]"] {
            std::fs::write(&p, format!(r#"{{"theme":"light","repository_root":{bad}}}"#)).unwrap();
            assert_eq!(load(&p), light());
            assert!(p.exists());
        }
        std::fs::write(&p, r#"{"theme":"nope","repository_root":"/abs/lib"}"#).unwrap();
        let s = load(&p);
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.repository_root, Some(PathBuf::from("/abs/lib")));
    }

    #[test]
    fn root_roundtrip_and_set_reset_persist() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cfg/s.json");
        let s = Settings { repository_root: Some(PathBuf::from("/abs/x")), ..Settings::default() };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
        let store = SettingsStore::open(p.clone());
        let r = store.set_repository_root(None).unwrap();
        assert_eq!(r.repository_root, None);
        assert_eq!(SettingsStore::open(p.clone()).get().repository_root, None);
        store.set_repository_root(Some(PathBuf::from("/abs/y"))).unwrap();
        assert_eq!(SettingsStore::open(p).get().repository_root, Some(PathBuf::from("/abs/y")));
    }

    #[test]
    fn default_root_is_data_dir_calliope() {
        assert_eq!(default_repository_root(Path::new("/d")), PathBuf::from("/d/calliope"));
    }

    #[test]
    fn edge_ai_url_rules() {
        let ok = |i: &str| normalize_edge_ai_url(i).unwrap();
        assert_eq!(ok("http://archserver:8765"), Some("http://archserver:8765".into()));
        assert_eq!(ok("http://192.168.2.20:8765/"), Some("http://192.168.2.20:8765".into()));
        assert_eq!(ok("http://h:8765/prefix"), Some("http://h:8765/prefix".into()));
        assert_eq!(ok("  http://h:8765/prefix/  "), Some("http://h:8765/prefix".into()));
        assert_eq!(ok("http://[::1]:8765"), Some("http://[::1]:8765".into()));
        assert_eq!(ok(""), None);
        assert_eq!(ok("   "), None);
        assert!(normalize_edge_ai_url("https://h:8765").unwrap_err().contains("Only http://"));
        for bad in [
            "ftp://h", "http://u:p@h", "http://u@h", "http://h/?q", "http://h/#f", "http://", "http:///x", "h:8765",
            "archserver", "file:///etc/passwd", "http://h:99999",
        ] {
            assert!(normalize_edge_ai_url(bad).is_err(), "{bad}");
        }
        let long = format!("http://h/{}", "a".repeat(200));
        assert!(normalize_edge_ai_url(&long).is_err());
    }

    #[test]
    fn new_fields_default_and_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        assert_eq!(Settings::default().edge_ai_url, None);
        assert!(!Settings::default().keep_original);
        let s = Settings { edge_ai_url: Some("http://h:1".into()), keep_original: true, ..Settings::default() };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
    }

    #[test]
    fn invalid_new_fields_fall_back_others_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.json");
        for bad in ["\"https://h\"", "\"http://u:p@h\"", "5", "[]"] {
            std::fs::write(&p, format!(r#"{{"theme":"light","keep_original":true,"edge_ai_url":{bad}}}"#)).unwrap();
            let s = load(&p);
            assert_eq!(s, Settings { theme: Theme::Light, keep_original: true, ..Settings::default() }, "{bad}");
            assert!(p.exists() && !d.path().join("s.json.bak").exists());
        }
        std::fs::write(&p, r#"{"keep_original":"yes","edge_ai_url":"http://h:2/"}"#).unwrap();
        let s = load(&p);
        assert!(!s.keep_original);
        assert_eq!(s.edge_ai_url.as_deref(), Some("http://h:2"));
    }

    #[test]
    fn store_setters_persist_and_validate() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cfg/s.json");
        let store = SettingsStore::open(p.clone());
        assert_eq!(store.set_edge_ai_url("http://a:1/").unwrap().edge_ai_url.as_deref(), Some("http://a:1"));
        assert!(store.set_edge_ai_url("https://a").is_err());
        assert_eq!(store.get().edge_ai_url.as_deref(), Some("http://a:1"));
        assert!(store.set_keep_original(true).unwrap().keep_original);
        let again = SettingsStore::open(p).get();
        assert!(again.keep_original);
        assert_eq!(again.edge_ai_url.as_deref(), Some("http://a:1"));
        assert_eq!(store.set_edge_ai_url("").unwrap().edge_ai_url, None);
    }
}
