//! URL download with `yt-dlp` (plan sections 2.6 and 2.12): URL validation, the yt-dlp
//! command line, progress / error parsing, `info.json` -> track fields, and the run itself.
//! Pure logic, no Tauri.
//!
//! Security: the URL is validated here (http/https, a host, no userinfo, no whitespace or
//! control characters, at most 2000 characters) and reaches yt-dlp as one argv entry after
//! `--`, with `--ignore-config`; no shell is involved. A resumable partial lives in the
//! URL-keyed folder of `import_tmp`; cancelling keeps it.
#![allow(dead_code)] // used by the import job (a later task) and the tests

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calliope_common::process;

use crate::media::{clean_text, year_from, Cancel};
use crate::track_meta::TrackEdits;

pub const INVALID_URL_MESSAGE: &str = "Entered URL is invalid";
pub const MAX_URL_CHARS: usize = 2000;
const MAX_ERROR_CHARS: usize = 300;
const MAX_NAME: usize = 200;
const MAX_COMPOSERS: usize = 20;
const MAX_COPYRIGHT: usize = 500;
const PROGRESS_PREFIX: &str = "calliope-progress ";
pub const INFO_JSON: &str = "info.json";
/// What the real yt-dlp makes of `--output infojson:info` (it appends `.info.json`).
const INFO_JSON_YTDLP: &str = "info.info.json";

/// Checks a user-entered URL and returns it parsed. Only `http` / `https` with a host and no
/// user name or password are accepted.
pub fn validate_url(input: &str) -> Result<url::Url, String> {
    let s = input.trim();
    let invalid = || INVALID_URL_MESSAGE.to_string();
    if s.is_empty() || s.chars().count() > MAX_URL_CHARS || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let u = url::Url::parse(s).map_err(|_| invalid())?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(invalid());
    }
    if u.host_str().is_none_or(str::is_empty) || !u.username().is_empty() || u.password().is_some() {
        return Err(invalid());
    }
    Ok(u)
}

/// The canonical form of a valid URL: parsed and re-serialised, fragment dropped, query kept.
/// This string is what is keyed, stored in `job.json` and given to yt-dlp.
pub fn normalise(input: &str) -> Result<String, String> {
    let mut u = validate_url(input)?;
    u.set_fragment(None);
    Ok(u.into())
}

/// The 16-hex key of a normalised URL (the name of its `import-tmp` folder).
pub fn url_key(normalised_url: &str) -> String {
    crate::import_tmp::url_key(normalised_url)
}

/// The yt-dlp argv (without the program), one entry per value. `resume` picks `--continue`
/// or `--no-continue`. `url` must be normalised.
pub fn ytdlp_args(job_dir: &Path, url: &str, resume: bool) -> Vec<String> {
    let dir = job_dir.display().to_string();
    let mut a: Vec<String> = [
        "--ignore-config",
        "--no-playlist",
        "--no-colors",
        "--newline",
        "--no-mtime",
        "--format",
        "bestaudio/best",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(if resume { "--continue" } else { "--no-continue" }.into());
    for s in [
        "--write-info-json",
        "--no-write-thumbnail",
        "--no-write-comments",
        "--progress-template",
        "download:calliope-progress %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
        "--paths",
    ] {
        a.push(s.into());
    }
    a.push(dir.clone());
    a.push("--paths".into());
    a.push(format!("temp:{dir}"));
    for s in ["--output", "download.%(ext)s", "--output", "infojson:info", "--"] {
        a.push(s.into());
    }
    a.push(url.into());
    a
}

fn num(s: &str) -> Option<u64> {
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }
    let f: f64 = s.parse().ok()?;
    (0.0..1e18).contains(&f).then_some(f as u64)
}

/// `calliope-progress <downloaded> <total> <estimate>` -> `(downloaded, total or estimate)`.
/// `NA` and anything unparsable mean "unknown".
pub fn parse_progress(line: &str) -> Option<(u64, Option<u64>)> {
    let rest = line.trim().strip_prefix(PROGRESS_PREFIX)?;
    let mut it = rest.split_whitespace();
    let downloaded = num(it.next()?)?;
    let total = it.next().and_then(num).filter(|&t| t > 0);
    let estimate = it.next().and_then(num).filter(|&t| t > 0);
    Some((downloaded, total.or(estimate)))
}

/// The three-digit code of a `HTTP Error NNN` in a yt-dlp message.
pub fn http_status(line: &str) -> Option<u16> {
    let i = line.find("HTTP Error ")? + "HTTP Error ".len();
    let digits: String = line[i..].chars().take_while(char::is_ascii_digit).collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

/// Turns a stderr line into the user-facing error; `None` if it isn't an `ERROR:` line.
pub fn parse_error(line: &str) -> Option<String> {
    let msg = line.trim().strip_prefix("ERROR:")?.trim();
    if let Some(code) = http_status(msg) {
        return Some(format!("Error {code} when attempting download"));
    }
    let msg = clean_text(msg, MAX_ERROR_CHARS);
    Some(if msg.is_empty() { "Download failed".to_string() } else { format!("Download failed: {msg}") })
}

fn json_str(v: &serde_json::Value, key: &str, max: usize) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| clean_text(s, max)).filter(|s| !s.is_empty())
}

fn json_year(v: &serde_json::Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        serde_json::Value::Number(n) => n.as_i64().filter(|y| (1..=9999).contains(y)),
        serde_json::Value::String(s) => year_from(s),
        _ => None,
    }
}

/// "X - Y" -> (X, Y), both non-empty after cleaning.
fn split_title(title: &str) -> Option<(String, String)> {
    let (x, y) = title.split_once(" - ")?;
    let (x, y) = (clean_text(x, MAX_NAME), clean_text(y, MAX_NAME));
    (!x.is_empty() && !y.is_empty()).then_some((x, y))
}

fn split_composers(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split([';', '/']).map(|x| clean_text(x, MAX_NAME)).filter(|x| !x.is_empty())
}

/// Maps yt-dlp's `info.json` to the editable fields (plan section 2.6). `entered_url` is the
/// normalised URL, used for `source_url` when the info has no usable `webpage_url`, and as the
/// last title fallback. `None` if the text isn't a JSON object.
pub fn edits_from_info(json: &str, entered_url: &str) -> Option<TrackEdits> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    if !v.is_object() {
        return None;
    }
    let artist = json_str(&v, "artist", MAX_NAME).or_else(|| json_str(&v, "creator", MAX_NAME));
    let title_raw = json_str(&v, "title", MAX_NAME);
    let split = title_raw.as_deref().and_then(split_title);
    let (band, from_title) = match (&artist, &split) {
        (Some(a), _) => (a.clone(), false),
        (None, Some((x, _))) => (x.clone(), true),
        (None, None) => (String::new(), false),
    };
    let title = json_str(&v, "track", MAX_NAME)
        .or_else(|| match &split {
            Some((x, y)) if from_title || x.to_lowercase() == band.to_lowercase() => Some(y.clone()),
            _ => title_raw.clone(),
        })
        .unwrap_or_else(|| clean_text(entered_url, MAX_NAME));
    let mut composers: Vec<String> = Vec::new();
    if let Some(s) = v.get("composer").and_then(|x| x.as_str()) {
        composers.extend(split_composers(s));
    }
    if let Some(arr) = v.get("composers").and_then(|x| x.as_array()) {
        for s in arr.iter().filter_map(|x| x.as_str()) {
            composers.extend(split_composers(s));
        }
    }
    composers.truncate(MAX_COMPOSERS);
    let year = json_year(&v, "release_year").or_else(|| json_year(&v, "release_date")).or_else(|| json_year(&v, "upload_date"));
    let source_url = json_str(&v, "webpage_url", MAX_URL_CHARS)
        .and_then(|u| normalise(&u).ok())
        .unwrap_or_else(|| entered_url.to_string());
    Some(TrackEdits {
        band,
        album: json_str(&v, "album", MAX_NAME).unwrap_or_default(),
        title,
        composers,
        year,
        source_url: Some(source_url),
        copyright: json_str(&v, "license", MAX_COPYRIGHT),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// Stopped by the user; the partial file is kept for a resume.
    Cancelled,
    Failed { message: String, http_status: Option<u16> },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("Cancelled"),
            DownloadError::Failed { message, .. } => f.write_str(message),
        }
    }
}

fn failed<T>(message: impl Into<String>) -> Result<T, DownloadError> {
    Err(DownloadError::Failed { message: message.into(), http_status: None })
}

/// What a finished download left in the job folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// `download.<ext>`
    pub file: PathBuf,
    /// `info.json`, if yt-dlp wrote one.
    pub info: Option<PathBuf>,
}

pub fn find_download(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("download.") && !n.ends_with(".part") && !n.ends_with(".ytdl")
        })
        .map(|e| e.path())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Runs `ytdlp` for the normalised `url` into `job_dir` (the URL's `import-tmp` folder, which
/// must exist), reporting `(downloaded, total)` to `on_progress` from a reader thread. With
/// `resume` an existing `.part` is continued; otherwise yt-dlp starts from zero. On cancel
/// the partial file stays. The caller passes the yt-dlp path found by `tools`.
pub fn run_download(
    ytdlp: &Path,
    job_dir: &Path,
    url: &str,
    resume: bool,
    cancel: &Cancel,
    mut on_progress: impl FnMut(u64, Option<u64>) + Send + 'static,
) -> Result<Downloaded, DownloadError> {
    if normalise(url).as_deref() != Ok(url) {
        return failed(INVALID_URL_MESSAGE);
    }
    if !job_dir.is_dir() {
        return failed("The import folder is missing");
    }
    let last_error = Arc::new(Mutex::new(None::<String>));
    let last_raw = Arc::new(Mutex::new(None::<String>));
    let (le, lr) = (last_error.clone(), last_raw.clone());
    let args = ytdlp_args(job_dir, url, resume);
    let running = process::spawn(
        ytdlp,
        &args,
        Some(job_dir),
        move |l| {
            if let Some((d, t)) = parse_progress(&l) {
                on_progress(d, t);
            }
        },
        move |l| {
            if let Some(m) = parse_error(&l) {
                if let Ok(mut g) = le.lock() {
                    *g = Some(m);
                }
                if let Ok(mut g) = lr.lock() {
                    *g = Some(l);
                }
            }
        },
    )
    .map_err(|e| DownloadError::Failed { message: e.to_string(), http_status: None })?;
    cancel.register(running.cancel_handle());
    let status = running.wait();
    cancel.clear();
    let status = status.map_err(|e| DownloadError::Failed { message: e.to_string(), http_status: None })?;
    if cancel.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    if !status.success() {
        let message = last_error.lock().ok().and_then(|g| g.clone());
        let raw = last_raw.lock().ok().and_then(|g| g.clone());
        return Err(DownloadError::Failed {
            message: message.unwrap_or_else(|| match status.code() {
                Some(c) => format!("Download failed: yt-dlp exited with status {c}"),
                None => "Download failed: yt-dlp was stopped".to_string(),
            }),
            http_status: raw.as_deref().and_then(http_status),
        });
    }
    let Some(file) = find_download(job_dir) else {
        return failed("Download failed: yt-dlp produced no file");
    };
    // The real yt-dlp appends ".info.json" to the `infojson:` name; settle on `info.json`.
    let info = job_dir.join(INFO_JSON);
    let ytdlp_info = job_dir.join(INFO_JSON_YTDLP);
    if ytdlp_info.is_file() {
        let _ = std::fs::rename(&ytdlp_info, &info);
    }
    Ok(Downloaded { file, info: info.is_file().then_some(info) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_tmp::ImportTmp;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn fake() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/bin/yt-dlp")
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import").join(name)
    }

    /// A temp repository root with the URL's job folder prepared.
    fn setup(url: &str) -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let tmp = ImportTmp::open(root.path()).unwrap();
        let dir = tmp.prepare_url_dir(url, "2026-10-06T10:00:00Z").unwrap();
        (root, dir)
    }

    type Seen = Arc<Mutex<Vec<(u64, Option<u64>)>>>;

    fn collect() -> (Seen, impl FnMut(u64, Option<u64>) + Send + 'static) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (seen, move |d, t| s.lock().unwrap().push((d, t)))
    }

    fn log_lines(log: &Path) -> Vec<serde_json::Value> {
        fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    // The fake logs through this variable; tests that use it run in one process, so each test
    // uses a distinct log file passed through a wrapper script instead of the environment.
    fn fake_with_log(dir: &Path, log: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let w = dir.join("yt-dlp-wrapper");
        let body = format!("#!/bin/sh\nFAKE_YTDLP_LOG='{}' exec '{}' \"$@\"\n", log.display(), fake().display());
        fs::write(&w, body).unwrap();
        fs::set_permissions(&w, fs::Permissions::from_mode(0o755)).unwrap();
        w
    }

    #[test]
    fn url_table() {
        let long = format!("https://a.example/{}", "x".repeat(2000));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://x",
            "https://",
            "http://user:pw@h/",
            "http://user@h/",
            "https://a b.example",
            "-https://x",
            "",
            "   ",
            "https://a.example/\n--exec",
            "data:text/html,hi",
            long.as_str(),
        ] {
            assert_eq!(validate_url(bad).unwrap_err(), "Entered URL is invalid", "{bad:?}");
            assert!(normalise(bad).is_err(), "{bad:?}");
        }
        assert_eq!(normalise("https://media.example/watch?v=ok#t=5").unwrap(), "https://media.example/watch?v=ok");
        assert_eq!(normalise("  HTTP://Media.Example/a b").unwrap_err(), "Entered URL is invalid");
        assert_eq!(normalise(" HTTP://Media.Example/watch?v=1 ").unwrap(), "http://media.example/watch?v=1");
        assert!(normalise("https://a.example/").is_ok());
        let exactly = format!("https://a.example/{}", "x".repeat(2000 - 18));
        assert_eq!(exactly.chars().count(), 2000);
        assert!(validate_url(&exactly).is_ok());
    }

    #[test]
    fn url_key_is_stable_and_matches_the_folder() {
        let a = normalise("https://media.example/watch?v=1#x").unwrap();
        let b = normalise("https://media.example/watch?v=1").unwrap();
        assert_eq!(url_key(&a), url_key(&b));
        assert_ne!(url_key(&a), url_key("https://media.example/watch?v=2"));
        assert_eq!(url_key(&a).len(), 16);
    }

    #[test]
    fn argv_is_one_entry_per_value_with_the_url_last_after_double_dash() {
        let a = ytdlp_args(Path::new("/tmp/my job"), "https://h.example/watch?v=1&x=2", true);
        assert_eq!(a[0], "--ignore-config");
        assert!(a.contains(&"--continue".to_string()) && !a.contains(&"--no-continue".to_string()));
        assert_eq!(a[a.len() - 2], "--");
        assert_eq!(a[a.len() - 1], "https://h.example/watch?v=1&x=2");
        assert!(a.windows(2).any(|w| w[0] == "--paths" && w[1] == "/tmp/my job"));
        assert!(a.windows(2).any(|w| w[0] == "--paths" && w[1] == "temp:/tmp/my job"));
        assert!(a.windows(2).any(|w| w[0] == "--output" && w[1] == "download.%(ext)s"));
        assert!(a.windows(2).any(|w| w[0] == "--output" && w[1] == "infojson:info"));
        assert!(a.windows(2).any(|w| w[0] == "--format" && w[1] == "bestaudio/best"));
        for f in ["--no-playlist", "--newline", "--no-colors", "--no-mtime", "--write-info-json"] {
            assert!(a.contains(&f.to_string()), "{f}");
        }
        assert!(ytdlp_args(Path::new("/x"), "https://h.example/", false).contains(&"--no-continue".to_string()));
    }

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress("calliope-progress 100 1000 NA"), Some((100, Some(1000))));
        assert_eq!(parse_progress("calliope-progress 100 NA 2000"), Some((100, Some(2000))));
        assert_eq!(parse_progress("calliope-progress 100.0 NA NA"), Some((100, None)));
        assert_eq!(parse_progress("calliope-progress NA NA NA"), None);
        assert_eq!(parse_progress("[download] 50%"), None);
        assert_eq!(parse_progress("calliope-progress"), None);
    }

    #[test]
    fn error_lines() {
        assert_eq!(
            parse_error("ERROR: [generic] Unable to download webpage: HTTP Error 403: Forbidden").unwrap(),
            "Error 403 when attempting download"
        );
        assert_eq!(http_status("HTTP Error 404: Not Found"), Some(404));
        assert_eq!(http_status("HTTP Error 4: x"), None);
        assert_eq!(parse_error("ERROR: Unsupported URL: https://x.example/").unwrap(), "Download failed: Unsupported URL: https://x.example/");
        assert_eq!(parse_error("WARNING: something"), None);
        let long = format!("ERROR: {}", "y".repeat(1000));
        assert_eq!(parse_error(&long).unwrap().chars().count(), "Download failed: ".len() + 300);
    }

    #[test]
    fn info_mapping_from_the_fixture() {
        let json = fs::read_to_string(fixture("info.json")).unwrap();
        let e = edits_from_info(&json, "https://entered.example/x").unwrap();
        assert_eq!(e.band, "The Example Band");
        assert_eq!(e.title, "Night Drive");
        assert_eq!(e.year, Some(2020));
        assert_eq!(e.source_url.as_deref(), Some("https://media.example/watch?v=abc123"));
        assert_eq!(e.album, "");
        assert!(e.composers.is_empty());
    }

    #[test]
    fn info_mapping_rules() {
        let u = "https://e.example/v";
        let e = edits_from_info(
            r#"{"artist":"A","creator":"C","track":"T","title":"A - Other","album":"Alb","composers":["X; Y","Z/W"],
                "release_year":1999,"release_date":"20100101","upload_date":"20200101","license":" CC "}"#,
            u,
        )
        .unwrap();
        assert_eq!((e.band.as_str(), e.title.as_str(), e.album.as_str()), ("A", "T", "Alb"));
        assert_eq!(e.composers, ["X", "Y", "Z", "W"]);
        assert_eq!(e.year, Some(1999));
        assert_eq!(e.copyright.as_deref(), Some("CC"));
        assert_eq!(e.source_url.as_deref(), Some(u));
        let e = edits_from_info(r#"{"creator":"C","title":"C - Song","release_date":"20100101"}"#, u).unwrap();
        assert_eq!((e.band.as_str(), e.title.as_str(), e.year), ("C", "Song", Some(2010)));
        let e = edits_from_info(r#"{"artist":"A","title":"Some Title"}"#, u).unwrap();
        assert_eq!(e.title, "Some Title");
        let e = edits_from_info(r#"{"webpage_url":"javascript:1"}"#, u).unwrap();
        assert_eq!((e.title.as_str(), e.source_url.as_deref()), (u, Some(u)));
        assert!(edits_from_info("[1]", u).is_none());
        assert!(edits_from_info("nope", u).is_none());
    }

    #[test]
    fn fake_refuses_real_hosts_and_demands_safe_flags() {
        let d = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new(fake()).args(args).output().unwrap()
        };
        let o = run(&["--ignore-config", "--paths", d.path().to_str().unwrap(), "--", "https://www.youtube.com/watch?v=ok"]);
        assert_eq!(o.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&o.stderr).contains("real network URLs are not allowed"));
        let o = run(&["--paths", d.path().to_str().unwrap(), "--", "https://media.example/watch"]);
        assert_eq!(o.status.code(), Some(2));
        let o = run(&["--ignore-config", "https://media.example/watch"]);
        assert_eq!(o.status.code(), Some(2));
        let o = run(&["--version"]);
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "2025.09.26");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn run_download_refuses_a_real_host_through_the_fake() {
        // Even if validation were bypassed, the fake would refuse: fail closed.
        let url = "https://www.youtube.com/watch?v=ok";
        let (_r, dir) = setup(url);
        let err = run_download(&fake(), &dir, url, true, &Cancel::new(), |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("exited with status 2"), "{err}");
        assert!(find_download(&dir).is_none());
    }

    #[test]
    fn full_download_reports_increasing_progress() {
        let url = normalise("https://media.example/watch?v=ok").unwrap();
        let (_r, dir) = setup(&url);
        let (seen, cb) = collect();
        let got = run_download(&fake(), &dir, &url, true, &Cancel::new(), cb).unwrap();
        assert_eq!(got.file, dir.join("download.webm"));
        assert_eq!(got.info, Some(dir.join("info.json")));
        assert_eq!(fs::read(&got.file).unwrap(), fs::read(fixture("download.webm")).unwrap());
        assert!(!dir.join("info.info.json").exists());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
        let total = fs::metadata(&got.file).unwrap().len();
        assert_eq!(seen.last().unwrap(), &(total, Some(total)));
        let info = fs::read_to_string(got.info.unwrap()).unwrap();
        assert_eq!(edits_from_info(&info, &url).unwrap().title, "Night Drive");
    }

    #[test]
    fn no_total_reports_unknown_total() {
        let url = normalise("https://media.example/no-total").unwrap();
        let (_r, dir) = setup(&url);
        let (seen, cb) = collect();
        run_download(&fake(), &dir, &url, true, &Cancel::new(), cb).unwrap();
        assert!(seen.lock().unwrap().iter().all(|p| p.1.is_none()));
    }

    #[test]
    fn http_403_and_offline_errors() {
        let url = normalise("https://media.example/http403").unwrap();
        let (_r, dir) = setup(&url);
        let e = run_download(&fake(), &dir, &url, true, &Cancel::new(), |_, _| {}).unwrap_err();
        assert_eq!(e, DownloadError::Failed { message: "Error 403 when attempting download".into(), http_status: Some(403) });
        let url = normalise("https://media.example/offline").unwrap();
        let (_r, dir) = setup(&url);
        match run_download(&fake(), &dir, &url, true, &Cancel::new(), |_, _| {}).unwrap_err() {
            DownloadError::Failed { message, http_status: None } => {
                assert!(message.starts_with("Download failed: [generic] Unable to download webpage"), "{message}")
            }
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn missing_program_and_bad_url_are_failures_not_panics() {
        let url = normalise("https://media.example/watch").unwrap();
        let (_r, dir) = setup(&url);
        let e = run_download(Path::new("/nonexistent/yt-dlp"), &dir, &url, true, &Cancel::new(), |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("not found"), "{e}");
        let e = run_download(&fake(), &dir, "file:///etc/passwd", true, &Cancel::new(), |_, _| {}).unwrap_err();
        assert_eq!(e.to_string(), "Entered URL is invalid");
        // not normalised (fragment) is refused too: the caller must normalise
        let e = run_download(&fake(), &dir, "https://media.example/watch#x", true, &Cancel::new(), |_, _| {}).unwrap_err();
        assert_eq!(e.to_string(), "Entered URL is invalid");
    }

    #[test]
    fn cancel_keeps_the_part_and_resume_continues_from_its_size() {
        let url = normalise("https://media.example/slow").unwrap();
        let (root, dir) = setup(&url);
        let log = root.path().join("fake.log");
        let prog = fake_with_log(root.path(), &log);
        let cancel = Cancel::new();
        let reached = Arc::new(AtomicU64::new(0));
        let r = reached.clone();
        let c2 = cancel.clone();
        let started = Instant::now();
        let res = run_download(&prog, &dir, &url, true, &cancel, move |d, _| {
            r.store(d, Ordering::SeqCst);
            c2.cancel();
        });
        assert_eq!(res.unwrap_err(), DownloadError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
        let part = dir.join("download.webm.part");
        let size = fs::metadata(&part).expect("the partial file is kept").len();
        assert!(size > 0 && size < fs::metadata(fixture("download.webm")).unwrap().len());
        assert!(!dir.join("download.webm").exists());
        // the folder is resumable per import_tmp
        let tmp = ImportTmp::open(root.path()).unwrap();
        assert!(tmp.find_partial(&url).unwrap().has_part);

        // resume
        let got = run_download(&prog, &dir, &url, true, &Cancel::new(), |_, _| {}).unwrap();
        assert_eq!(fs::read(got.file).unwrap(), fs::read(fixture("download.webm")).unwrap());
        let lines = log_lines(&log);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["resume_from"], 0);
        assert_eq!(lines[1]["resume_from"], size);
        assert!(lines[1]["argv"].as_array().unwrap().iter().any(|a| a == "--continue"));
    }

    #[test]
    fn no_continue_starts_at_zero() {
        let url = normalise("https://media.example/watch?v=ok").unwrap();
        let (root, dir) = setup(&url);
        let log = root.path().join("fake.log");
        let prog = fake_with_log(root.path(), &log);
        let data = fs::read(fixture("download.webm")).unwrap();
        fs::write(dir.join("download.webm.part"), &data[..3000]).unwrap();
        let got = run_download(&prog, &dir, &url, false, &Cancel::new(), |_, _| {}).unwrap();
        assert_eq!(fs::read(got.file).unwrap(), data);
        let lines = log_lines(&log);
        assert_eq!(lines[0]["resume_from"], 0);
        assert!(lines[0]["argv"].as_array().unwrap().iter().any(|a| a == "--no-continue"));
    }

    #[test]
    fn the_fake_receives_the_url_after_double_dash_and_ignore_config() {
        let url = normalise("https://media.example/watch?v=ok&list=x").unwrap();
        let (root, dir) = setup(&url);
        let log = root.path().join("fake.log");
        let prog = fake_with_log(root.path(), &log);
        run_download(&prog, &dir, &url, true, &Cancel::new(), |_, _| {}).unwrap();
        let argv: Vec<String> =
            log_lines(&log)[0]["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
        assert_eq!(argv[0], "--ignore-config");
        assert_eq!(&argv[argv.len() - 2..], ["--", url.as_str()]);
    }
}
