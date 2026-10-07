//! `<root>/import-tmp/`: Calliope's own working space for imports (plan section 2.5). Pure file
//! logic, no Tauri.
//!
//! Data safety: only folders directly inside `import-tmp/` whose name matches
//! `url-<16 hex>` / `file-<uuid>` AND that contain `job.json` are ever removed; each is checked
//! with `symlink_metadata` to be a real folder. A symlinked `import-tmp` is refused. Anything
//! else in there (a user's own file or folder) is never touched.
#![allow(dead_code)] // used by the import job (a later task) and the tests

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::fsutil;
use crate::track_meta;

pub const DIR_NAME: &str = "import-tmp";
pub const JOB_FILE: &str = "job.json";
pub const AUDIO_FLAC: &str = "audio.flac";
/// `url-*` folders older than this are removed by `clean_stale`.
pub const URL_KEEP_SECS: u64 = 14 * 24 * 3600;

fn io_err(what: &str, e: io::Error) -> String {
    format!("{what}: {e}")
}

/// `url-<16 lower-case hex>`
fn is_url_name(name: &str) -> bool {
    name.strip_prefix("url-")
        .is_some_and(|k| k.len() == 16 && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// `file-[0-9a-z-]{36}`
fn is_file_name(name: &str) -> bool {
    name.strip_prefix("file-")
        .is_some_and(|k| k.len() == 36 && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'-')))
}

fn is_real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn is_real_file(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

/// What a URL's folder holds (decides whether the UI offers "Resume").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partial {
    pub dir: PathBuf,
    /// A yt-dlp `.part` file.
    pub has_part: bool,
    /// A complete `download.<ext>`.
    pub has_download: bool,
    /// A finished `audio.flac`.
    pub has_audio: bool,
}

impl Partial {
    pub fn is_resumable(&self) -> bool {
        self.has_part || self.has_download || self.has_audio
    }
}

#[derive(Debug, Clone)]
pub struct ImportTmp {
    dir: PathBuf,
}

/// The 16-hex key of a (normalised) URL.
pub fn url_key(normalised_url: &str) -> String {
    fsutil::fnv1a64_hex(normalised_url.as_bytes())
}

/// True for yt-dlp's finished `download.<ext>`; false for `.part` / `.ytdl` / `.temp` and the
/// per-format fragments of a merge (`download.f251.webm`).
pub fn is_finished_download(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("download.") else { return false };
    if [".part", ".ytdl", ".temp"].iter().any(|x| name.ends_with(x)) || name.contains(".part-Frag") {
        return false;
    }
    let fragment = rest
        .split_once('.')
        .is_some_and(|(f, _)| f.len() > 1 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    !fragment && !rest.is_empty()
}

impl ImportTmp {
    /// Opens (creating it if absent) `<root>/import-tmp/`. The root must be an existing
    /// folder; a symlinked or non-folder `import-tmp` is refused.
    pub fn open(root: &Path) -> Result<Self, String> {
        if !root.is_dir() {
            return Err("The track repository folder is missing. Check Settings > Track repository.".into());
        }
        let dir = root.join(DIR_NAME);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{} is not a real folder (a symlink or file); import is refused", dir.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|e| io_err("cannot create the import folder", e))?
            }
            Err(e) => return Err(io_err("cannot read the import folder", e)),
        }
        Ok(Self { dir })
    }

    /// Like `open` but never creates anything: `None` if there is no real `import-tmp` folder.
    pub fn peek(root: &Path) -> Option<Self> {
        let dir = root.join(DIR_NAME);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() && root.is_dir() => Some(Self { dir }),
            _ => None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// `import-tmp/url-<key>` for a normalised URL (not created).
    pub fn url_dir(&self, normalised_url: &str) -> PathBuf {
        self.dir.join(format!("url-{}", url_key(normalised_url)))
    }

    fn job_json(kind: &str, extra: (&str, &str), created: &str) -> String {
        let mut m = serde_json::Map::new();
        m.insert("kind".into(), kind.into());
        m.insert(extra.0.into(), extra.1.into());
        m.insert("created".into(), created.into());
        serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap_or_default() + "\n"
    }

    fn read_job(dir: &Path) -> Option<serde_json::Value> {
        let job = dir.join(JOB_FILE);
        if !is_real_file(&job) {
            return None;
        }
        serde_json::from_slice(&fs::read(job).ok()?).ok()
    }

    /// True if `dir` is a URL job folder of exactly this URL (name, real folder, `job.json`).
    fn url_dir_matches(&self, dir: &Path, normalised_url: &str) -> bool {
        is_real_dir(dir)
            && Self::read_job(dir).is_some_and(|j| {
                j.get("kind").and_then(|v| v.as_str()) == Some("url")
                    && j.get("url").and_then(|v| v.as_str()) == Some(normalised_url)
            })
    }

    /// The folder for a URL import: created with its `job.json` when absent, reused when
    /// `job.json` names the same URL. A folder of that name that isn't ours is an error.
    pub fn prepare_url_dir(&self, normalised_url: &str, created: &str) -> Result<PathBuf, String> {
        let dir = self.url_dir(normalised_url);
        match fs::symlink_metadata(&dir) {
            Ok(_) => {
                if self.url_dir_matches(&dir, normalised_url) {
                    Ok(dir)
                } else {
                    Err(format!("{} exists but is not this import's folder; left alone", dir.display()))
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|e| io_err("cannot create the import folder", e))?;
                let job = Self::job_json("url", ("url", normalised_url), created);
                if let Err(e) = fsutil::write_atomic(&dir.join(JOB_FILE), job.as_bytes()) {
                    let _ = fs::remove_dir(&dir);
                    return Err(io_err("cannot write job.json", e));
                }
                Ok(dir)
            }
            Err(e) => Err(io_err("cannot read the import folder", e)),
        }
    }

    /// Looks for earlier work on this URL. `None` if there is no folder or its `job.json`
    /// doesn't name this exact URL.
    pub fn find_partial(&self, normalised_url: &str) -> Option<Partial> {
        let dir = self.url_dir(normalised_url);
        if !self.url_dir_matches(&dir, normalised_url) {
            return None;
        }
        let mut p = Partial { dir: dir.clone(), has_part: false, has_download: false, has_audio: false };
        for e in fs::read_dir(&dir).ok()?.flatten() {
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name == AUDIO_FLAC {
                p.has_audio = true;
            } else if name.starts_with("download.") && name.ends_with(".part") {
                p.has_part = true;
            } else if is_finished_download(&name) {
                p.has_download = true;
            }
        }
        Some(p)
    }

    /// "Start Over": removes everything inside the URL's folder (not the folder), then writes
    /// a fresh `job.json`. Requires the folder to be ours (see `find_partial`).
    pub fn clear_for_start_over(&self, normalised_url: &str, created: &str) -> Result<PathBuf, String> {
        let dir = self.url_dir(normalised_url);
        if !self.url_dir_matches(&dir, normalised_url) {
            return Err("there is no import folder of this URL to clear".into());
        }
        let rd = fs::read_dir(&dir).map_err(|e| io_err("cannot read the import folder", e))?;
        for e in rd.flatten() {
            let p = e.path();
            // symlink_metadata: a link is removed as a link, its target is never touched
            let r = match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => fs::remove_dir_all(&p),
                Ok(_) => fs::remove_file(&p),
                Err(e) => Err(e),
            };
            r.map_err(|e| io_err("cannot clear the import folder", e))?;
        }
        let job = Self::job_json("url", ("url", normalised_url), created);
        fsutil::write_atomic(&dir.join(JOB_FILE), job.as_bytes()).map_err(|e| io_err("cannot write job.json", e))?;
        Ok(dir)
    }

    /// A new `file-<uuidv7>/` folder for a local-file import. `kind` is `audio-file` or
    /// `video-file`; `name` is the bare file name (never a path).
    pub fn new_file_dir(&self, kind: &str, name: &str, created: &str) -> Result<PathBuf, String> {
        if kind != "audio-file" && kind != "video-file" {
            return Err(format!("unknown import kind \"{kind}\""));
        }
        let name = Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or("");
        let dir = self.dir.join(format!("file-{}", track_meta::new_id()));
        fs::create_dir(&dir).map_err(|e| io_err("cannot create the import folder", e))?;
        let job = Self::job_json(kind, ("name", name), created);
        if let Err(e) = fsutil::write_atomic(&dir.join(JOB_FILE), job.as_bytes()) {
            let _ = fs::remove_dir(&dir);
            return Err(io_err("cannot write job.json", e));
        }
        Ok(dir)
    }

    /// Is `dir` one of our job folders: a direct child of `import-tmp/` with a name matching
    /// the patterns, a real folder, holding `job.json`?
    fn is_job_dir(&self, dir: &Path) -> bool {
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else { return false };
        dir.parent() == Some(self.dir.as_path())
            && (is_url_name(name) || is_file_name(name))
            && is_real_dir(dir)
            && is_real_file(&dir.join(JOB_FILE))
    }

    /// Removes a finished/discarded job folder. Refuses anything that isn't provably ours.
    pub fn remove_job_dir(&self, dir: &Path) -> Result<(), String> {
        if !self.is_job_dir(dir) {
            return Err(format!("{} is not one of Calliope's import folders; left alone", dir.display()));
        }
        // remove_dir_all does not follow symlinks inside
        fs::remove_dir_all(dir).map_err(|e| io_err("cannot remove the import folder", e))
    }

    /// Removes stale job folders: `file-*` folders other than `keep` (a folder name or path
    /// of the running job), and `url-*` folders whose `job.json` `created` is older than
    /// [`URL_KEEP_SECS`] at `now` (Unix seconds); a `url-*` with an unreadable date is kept.
    /// Returns the number removed.
    pub fn clean_stale(&self, now: u64, keep: Option<&Path>) -> usize {
        let keep_name = keep.and_then(|k| k.file_name()).map(|n| n.to_os_string());
        let Ok(rd) = fs::read_dir(&self.dir) else { return 0 };
        let mut removed = 0;
        for e in rd.flatten() {
            let path = e.path();
            if keep_name.as_deref() == Some(e.file_name().as_os_str()) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !self.is_job_dir(&path) {
                continue;
            }
            let stale = if is_file_name(&name) {
                true
            } else {
                Self::read_job(&path)
                    .and_then(|j| j.get("created").and_then(|c| c.as_str().and_then(parse_rfc3339_utc)))
                    .is_some_and(|created| now.saturating_sub(created) > URL_KEEP_SECS)
            };
            if stale && fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

/// Parses `YYYY-MM-DDTHH:MM:SSZ` to Unix seconds (years 1970..9999).
pub fn parse_rfc3339_utc(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let t = &s[r];
        t.bytes().all(|c| c.is_ascii_digit()).then(|| t.parse().ok()).flatten()
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const URL: &str = "https://media.example/watch?v=abc123";
    const OLD: &str = "2020-01-01T00:00:00Z";

    fn setup() -> (TempDir, ImportTmp) {
        let tmp = tempfile::tempdir().unwrap();
        let t = ImportTmp::open(tmp.path()).unwrap();
        (tmp, t)
    }

    #[test]
    fn rfc3339_round_trip() {
        for secs in [0u64, 86_399, 1_700_000_000, 1_791_000_000, 4_102_444_800] {
            assert_eq!(parse_rfc3339_utc(&track_meta::rfc3339_utc(secs)), Some(secs));
        }
        assert_eq!(parse_rfc3339_utc("garbage"), None);
        assert_eq!(parse_rfc3339_utc("2020-13-01T00:00:00Z"), None);
    }

    #[test]
    fn open_creates_folder_and_requires_root() {
        let (tmp, t) = setup();
        assert!(tmp.path().join("import-tmp").is_dir());
        assert_eq!(t.path(), tmp.path().join("import-tmp"));
        assert!(ImportTmp::open(&tmp.path().join("nope")).unwrap_err().contains("missing"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_import_tmp_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let root = tmp.path().join("root");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("import-tmp")).unwrap();
        assert!(ImportTmp::open(&root).unwrap_err().contains("not a real folder"));
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn url_dir_is_keyed_and_job_json_checked() {
        let (_tmp, t) = setup();
        let dir = t.prepare_url_dir(URL, OLD).unwrap();
        assert_eq!(dir.file_name().unwrap().to_str().unwrap(), format!("url-{}", url_key(URL)));
        let job: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("job.json")).unwrap()).unwrap();
        assert_eq!(job["kind"], "url");
        assert_eq!(job["url"], URL);
        assert_eq!(job["created"], OLD);
        // same URL again reuses it
        assert_eq!(t.prepare_url_dir(URL, "2026-01-01T00:00:00Z").unwrap(), dir);
        // a job.json with another URL (hash clash or tampering) is not ours to reuse
        fs::write(dir.join("job.json"), r#"{"kind":"url","url":"https://other.example/","created":"x"}"#).unwrap();
        assert!(t.prepare_url_dir(URL, OLD).is_err());
        assert!(t.find_partial(URL).is_none());
        assert!(t.clear_for_start_over(URL, OLD).is_err());
    }

    #[test]
    fn find_partial_reports_what_is_there() {
        let (_tmp, t) = setup();
        assert!(t.find_partial(URL).is_none());
        let dir = t.prepare_url_dir(URL, OLD).unwrap();
        let p = t.find_partial(URL).unwrap();
        assert!(!p.is_resumable());
        fs::write(dir.join("info.json"), "{}").unwrap();
        assert!(!t.find_partial(URL).unwrap().is_resumable());
        fs::write(dir.join("download.webm.part"), "x").unwrap();
        let p = t.find_partial(URL).unwrap();
        assert!(p.has_part && !p.has_download && !p.has_audio && p.is_resumable());
        fs::write(dir.join("download.webm"), "x").unwrap();
        fs::write(dir.join("audio.flac"), "x").unwrap();
        let p = t.find_partial(URL).unwrap();
        assert!(p.has_part && p.has_download && p.has_audio);
        assert_eq!(p.dir, dir);
    }

    #[test]
    fn start_over_clears_contents_and_keeps_a_fresh_job_json() {
        let (tmp, t) = setup();
        let dir = t.prepare_url_dir(URL, OLD).unwrap();
        fs::write(dir.join("download.webm.part"), "x").unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/f"), "x").unwrap();
        let outside = tmp.path().join("outside.txt");
        fs::write(&outside, "keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        t.clear_for_start_over(URL, "2026-10-06T10:00:00Z").unwrap();
        let names: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        assert_eq!(names, ["job.json"]);
        assert!(fs::read_to_string(dir.join("job.json")).unwrap().contains("2026-10-06T10:00:00Z"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        assert!(!t.find_partial(URL).unwrap().is_resumable());
    }

    #[test]
    fn file_dirs_are_unique_and_named_by_pattern() {
        let (_tmp, t) = setup();
        let a = t.new_file_dir("audio-file", "song.mp3", OLD).unwrap();
        let b = t.new_file_dir("video-file", "/home/u/secret/clip.mp4", OLD).unwrap();
        assert_ne!(a, b);
        assert!(is_file_name(a.file_name().unwrap().to_str().unwrap()));
        let job: serde_json::Value = serde_json::from_slice(&fs::read(b.join("job.json")).unwrap()).unwrap();
        assert_eq!(job["kind"], "video-file");
        assert_eq!(job["name"], "clip.mp4", "only the bare name is recorded");
        assert!(t.new_file_dir("other", "x", OLD).is_err());
    }

    #[test]
    fn remove_job_dir_removes_only_our_folders() {
        let (tmp, t) = setup();
        let ours = t.new_file_dir("audio-file", "a.mp3", OLD).unwrap();
        fs::write(ours.join("audio.flac"), "x").unwrap();
        t.remove_job_dir(&ours).unwrap();
        assert!(!ours.exists());

        // name outside the patterns, even with a job.json
        let odd = t.path().join("my-stuff");
        fs::create_dir(&odd).unwrap();
        fs::write(odd.join("job.json"), "{}").unwrap();
        assert!(t.remove_job_dir(&odd).is_err());
        assert!(odd.join("job.json").exists());

        // right name, no job.json
        let nojob = t.path().join(format!("url-{}", "0".repeat(16)));
        fs::create_dir(&nojob).unwrap();
        fs::write(nojob.join("precious"), "x").unwrap();
        assert!(t.remove_job_dir(&nojob).is_err());
        assert!(nojob.join("precious").exists());

        // not a direct child, the import-tmp folder itself, the repo root
        let nested = t.path().join("my-stuff").join(format!("url-{}", "1".repeat(16)));
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("job.json"), "{}").unwrap();
        assert!(t.remove_job_dir(&nested).is_err());
        assert!(t.remove_job_dir(t.path()).is_err());
        assert!(t.remove_job_dir(tmp.path()).is_err());
        assert!(nested.exists() && t.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn remove_job_dir_refuses_a_symlink_and_never_follows_inner_links() {
        let (tmp, t) = setup();
        let target = tmp.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("job.json"), "{}").unwrap();
        fs::write(target.join("user-file"), "keep").unwrap();
        let link = t.path().join(format!("url-{}", "2".repeat(16)));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(t.remove_job_dir(&link).is_err());
        assert_eq!(t.clean_stale(u64::MAX / 2, None), 0);
        assert!(target.join("user-file").exists() && link.symlink_metadata().is_ok());

        // a link inside a real job folder is removed as a link, the target survives
        let job = t.new_file_dir("audio-file", "a.mp3", OLD).unwrap();
        std::os::unix::fs::symlink(&target, job.join("inner")).unwrap();
        t.remove_job_dir(&job).unwrap();
        assert!(target.join("user-file").exists());
    }

    #[test]
    fn clean_stale_keeps_running_job_and_fresh_url_folders() {
        let (_tmp, t) = setup();
        let now = parse_rfc3339_utc("2026-10-06T12:00:00Z").unwrap();
        let running = t.new_file_dir("audio-file", "run.mp3", OLD).unwrap();
        let leftover = t.new_file_dir("audio-file", "old.mp3", OLD).unwrap();
        let fresh = t.prepare_url_dir("https://a.example/1", "2026-10-01T00:00:00Z").unwrap();
        let old = t.prepare_url_dir("https://a.example/2", "2026-09-01T00:00:00Z").unwrap();
        let undated = t.prepare_url_dir("https://a.example/3", "not a date").unwrap();
        let removed = t.clean_stale(now, Some(&running));
        assert_eq!(removed, 2);
        assert!(running.exists() && fresh.exists() && undated.exists());
        assert!(!leftover.exists() && !old.exists());
        // an old url folder that is the running job survives too
        let old_running = t.prepare_url_dir("https://a.example/4", OLD).unwrap();
        assert_eq!(t.clean_stale(now, Some(&old_running)), 1, "only the file folder, which is no longer running");
        assert!(old_running.exists());
    }

    #[test]
    fn user_files_in_import_tmp_survive_every_cleanup() {
        let (_tmp, t) = setup();
        let file = t.path().join("my-notes.txt");
        fs::write(&file, "mine").unwrap();
        let dir = t.path().join("my-folder");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("a"), "mine").unwrap();
        // pattern-named but without job.json, and a pattern-named plain file
        let nojob = t.path().join(format!("file-{}", "a".repeat(36)));
        fs::create_dir(&nojob).unwrap();
        fs::write(nojob.join("b"), "mine").unwrap();
        let plain = t.path().join(format!("url-{}", "f".repeat(16)));
        fs::write(&plain, "mine").unwrap();
        // our own job next to them
        let job = t.new_file_dir("audio-file", "a.mp3", OLD).unwrap();
        assert_eq!(t.clean_stale(u64::MAX / 2, None), 1);
        assert!(!job.exists());
        for p in [&file, &dir.join("a"), &nojob.join("b"), &plain] {
            assert_eq!(fs::read_to_string(p).unwrap(), "mine", "{p:?}");
        }
        // start over on a url folder never touches siblings either
        t.prepare_url_dir(URL, OLD).unwrap();
        t.clear_for_start_over(URL, OLD).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "mine");
    }
}
