//! External tools on the laptop (plan section 2.6): finds `yt-dlp`, `ffmpeg` and `ffprobe` on
//! `PATH`, checks their versions and produces the install hints. Pure logic, no Tauri; the
//! `PATH` value is a parameter (the GUI captures it once). Tools run through
//! `calliope_common::process` (argv only, never a shell).
#![allow(dead_code)] // used by the download/media/job modules (later tasks) and the tests

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calliope_common::process;
use serde::Serialize;

/// How long a `--version` run may take before it is stopped.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// yt-dlp releases are dated; 2023.01 or newer is needed.
pub const MIN_YT_DLP: (u32, u32) = (2023, 1);
/// ffmpeg/ffprobe major version needed.
pub const MIN_FFMPEG_MAJOR: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolName {
    YtDlp,
    Ffmpeg,
    Ffprobe,
}

impl ToolName {
    pub fn exe(self) -> &'static str {
        match self {
            ToolName::YtDlp => "yt-dlp",
            ToolName::Ffmpeg => "ffmpeg",
            ToolName::Ffprobe => "ffprobe",
        }
    }

    fn version_arg(self) -> &'static str {
        match self {
            ToolName::YtDlp => "--version",
            _ => "-version",
        }
    }

    /// "yt-dlp was not found. Install it (Arch: sudo pacman -S yt-dlp) and try again."
    pub fn missing_message(self) -> String {
        match self {
            ToolName::YtDlp => "yt-dlp was not found. Install it (Arch: sudo pacman -S yt-dlp) and try again.".into(),
            ToolName::Ffmpeg => "ffmpeg was not found. Install it (Arch: sudo pacman -S ffmpeg) and try again.".into(),
            ToolName::Ffprobe => {
                "ffprobe was not found. It comes with ffmpeg: install it (Arch: sudo pacman -S ffmpeg) and try again."
                    .into()
            }
        }
    }
}

/// Finds an executable regular file called `name` in the folders of `path_var`, in order.
/// Empty and relative entries are skipped (never the current folder); a symlink to an
/// executable counts. `name` must be a bare name.
pub fn find_in_path(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return None;
    }
    std::env::split_paths(path_var)
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| is_executable_file(p))
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

// ---- version parsing ----

/// `YYYY.MM.DD[.n]` (yt-dlp, also `YYYY.MM.DD.<n>.dev` style suffixes are tolerated).
pub fn parse_yt_dlp_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().split('.');
    let mut num = |len: usize| -> Option<u32> {
        let p = it.next()?;
        (p.len() == len || (len == 2 && p.len() == 1)).then_some(())?;
        p.bytes().all(|b| b.is_ascii_digit()).then(|| p.parse().ok()).flatten()
    };
    let (y, m, d) = (num(4)?, num(2)?, num(2)?);
    ((1..=12).contains(&m) && (1..=31).contains(&d) && y >= 2000).then_some((y, m, d))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfVersion {
    /// A release: the major number, e.g. 9 for `n9.0.2`, 7 for `7.1`.
    Release(u32),
    /// A git/snapshot build (`N-12345-gabc`, `git-2024-...`); counts as new.
    Git,
}

/// Reads the version from a first line like `ffmpeg version n9.0.2 Copyright ...`.
pub fn parse_ffmpeg_version(line: &str) -> Option<(String, FfVersion)> {
    let token = line.trim().split_once(" version ")?.1.split_whitespace().next()?;
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("n-") || lower.starts_with("git-") {
        return Some((token.to_string(), FfVersion::Git));
    }
    let digits = lower.strip_prefix('n').unwrap_or(&lower);
    let major: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if major.is_empty() {
        return None;
    }
    Some((token.to_string(), FfVersion::Release(major.parse().ok()?)))
}

// ---- status ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolState {
    Ok,
    Missing,
    TooOld,
    /// Found and started, but the version could not be read. Usable.
    UnknownVersion,
    /// Found but could not be run.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolStatus {
    pub name: ToolName,
    pub state: ToolState,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Install hint / explanation, shown to the user; `None` when fine.
    pub message: Option<String>,
}

impl ToolStatus {
    pub fn usable(&self) -> bool {
        matches!(self.state, ToolState::Ok | ToolState::UnknownVersion)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolsStatus {
    pub yt_dlp: ToolStatus,
    pub ffmpeg: ToolStatus,
    pub ffprobe: ToolStatus,
}

/// Where the tools are (found once from `PATH`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    pub yt_dlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}

impl Tools {
    /// `ffprobe` is looked up next to `ffmpeg` first, then on `PATH`.
    pub fn discover(path_var: &OsStr) -> Self {
        let ffmpeg = find_in_path("ffmpeg", path_var);
        let next_to = ffmpeg
            .as_deref()
            .and_then(Path::parent)
            .map(|d| d.join("ffprobe"))
            .filter(|p| is_executable_file(p));
        Tools {
            yt_dlp: find_in_path("yt-dlp", path_var),
            ffprobe: next_to.or_else(|| find_in_path("ffprobe", path_var)),
            ffmpeg,
        }
    }

    pub fn path_of(&self, tool: ToolName) -> Option<&Path> {
        match tool {
            ToolName::YtDlp => self.yt_dlp.as_deref(),
            ToolName::Ffmpeg => self.ffmpeg.as_deref(),
            ToolName::Ffprobe => self.ffprobe.as_deref(),
        }
    }

    /// Runs each found tool's version command and judges it.
    pub fn check(&self) -> ToolsStatus {
        ToolsStatus {
            yt_dlp: self.check_one(ToolName::YtDlp),
            ffmpeg: self.check_one(ToolName::Ffmpeg),
            ffprobe: self.check_one(ToolName::Ffprobe),
        }
    }

    pub fn check_one(&self, tool: ToolName) -> ToolStatus {
        let mut st = ToolStatus { name: tool, state: ToolState::Missing, path: None, version: None, message: None };
        let Some(path) = self.path_of(tool) else {
            st.message = Some(tool.missing_message());
            return st;
        };
        st.path = Some(path.to_string_lossy().into_owned());
        let lines = match run_version(path, tool.version_arg()) {
            Ok(l) => l,
            Err(e) => {
                st.state = ToolState::Broken;
                st.message = Some(format!("{} could not be run: {e}", tool.exe()));
                return st;
            }
        };
        let first = lines.first().map(String::as_str).unwrap_or("");
        match tool {
            ToolName::YtDlp => match parse_yt_dlp_version(first) {
                Some((y, m, _)) => {
                    let text = first.trim().to_string();
                    if (y, m) < MIN_YT_DLP {
                        st.state = ToolState::TooOld;
                        st.message =
                            Some(format!("yt-dlp {text} is too old (need {}.{:02} or newer).", MIN_YT_DLP.0, MIN_YT_DLP.1));
                    } else {
                        st.state = ToolState::Ok;
                    }
                    st.version = Some(text);
                }
                None => unknown_version(&mut st, first),
            },
            ToolName::Ffmpeg | ToolName::Ffprobe => match parse_ffmpeg_version(first) {
                Some((text, FfVersion::Release(major))) => {
                    if major < MIN_FFMPEG_MAJOR {
                        st.state = ToolState::TooOld;
                        st.message =
                            Some(format!("{} {text} is too old (need {MIN_FFMPEG_MAJOR} or newer).", tool.exe()));
                    } else {
                        st.state = ToolState::Ok;
                    }
                    st.version = Some(text);
                }
                Some((text, FfVersion::Git)) => {
                    st.state = ToolState::Ok;
                    st.version = Some(text);
                }
                None => unknown_version(&mut st, first),
            },
        }
        st
    }
}

fn unknown_version(st: &mut ToolStatus, first_line: &str) {
    st.state = ToolState::UnknownVersion;
    let shown: String = first_line.chars().filter(|c| !c.is_control()).take(80).collect();
    st.message = Some(format!("The version of {} could not be read (\"{shown}\"); using it anyway.", st.name.exe()));
}

/// Runs `<path> <arg>` and returns its stdout lines. Stopped after [`VERSION_TIMEOUT`].
fn run_version(path: &Path, arg: &str) -> Result<Vec<String>, String> {
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let running = process::spawn(
        path,
        &[arg],
        None,
        move |l| {
            if let Ok(mut v) = sink.lock() {
                if v.len() < 20 {
                    v.push(l);
                }
            }
        },
        |_| {},
    )
    .map_err(|e| e.to_string())?;
    let handle = running.cancel_handle();
    let started = Instant::now();
    let watchdog = std::thread::spawn(move || {
        while !handle.is_finished() {
            if started.elapsed() > VERSION_TIMEOUT {
                handle.cancel();
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    });
    // the spawning thread waits for the child (PR_SET_PDEATHSIG ties it to this thread)
    let status = running.wait().map_err(|e| e.to_string());
    let timed_out = watchdog.join().unwrap_or(false);
    if timed_out {
        return Err("it did not answer in time".into());
    }
    let status = status?;
    if !status.success() {
        return Err(format!("it exited with {status}"));
    }
    let v = lines.lock().map(|v| v.clone()).unwrap_or_default();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn path_of(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn yt_dlp_version_table() {
        for (s, want) in [
            ("2023.01.06", Some((2023, 1, 6))),
            ("2024.12.13\n", Some((2024, 12, 13))),
            ("2025.01.15.232754", Some((2025, 1, 15))),
            ("2022.11.11", Some((2022, 11, 11))),
            ("", None),
            ("yt-dlp 2023.01.06", None),
            ("2023.13.01", None),
            ("23.1.6", None),
            ("stable", None),
        ] {
            assert_eq!(parse_yt_dlp_version(s), want, "{s:?}");
        }
    }

    #[test]
    fn ffmpeg_version_table() {
        let r = |n: u32| Some(FfVersion::Release(n));
        for (line, want) in [
            ("ffmpeg version n9.0.2 Copyright (c) 2000-2025 the FFmpeg developers", r(9)),
            ("ffmpeg version 7.1 Copyright (c) 2000-2024", r(7)),
            ("ffprobe version 6.1.1-3ubuntu5 Copyright", r(6)),
            ("ffmpeg version 5.1.2-0+deb12u1 Copyright", r(5)),
            ("ffmpeg version 4.4.2 Copyright", r(4)),
            ("ffmpeg version N-117000-gabcdef Copyright", Some(FfVersion::Git)),
            ("ffmpeg version git-2024-05-01-abc Copyright", Some(FfVersion::Git)),
            ("ffmpeg version unknown Copyright", None),
            ("something else", None),
            ("", None),
        ] {
            assert_eq!(parse_ffmpeg_version(line).map(|(_, v)| v), want, "{line:?}");
        }
        assert_eq!(parse_ffmpeg_version("ffmpeg version n9.0.2 Copyright").unwrap().0, "n9.0.2");
    }

    #[test]
    fn find_in_path_skips_non_executables_and_bad_entries() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        // a: a data file named like the tool, a folder named like another one
        fs::write(a.path().join("yt-dlp"), "not executable").unwrap();
        fs::create_dir(a.path().join("ffmpeg")).unwrap();
        fs::set_permissions(a.path().join("ffmpeg"), fs::Permissions::from_mode(0o755)).unwrap();
        let in_b = script(b.path(), "yt-dlp", "echo hi");
        let p = path_of(&[a.path(), b.path()]);
        assert_eq!(find_in_path("yt-dlp", &p), Some(in_b.clone()));
        assert_eq!(find_in_path("ffmpeg", &p), None);
        // first hit wins
        let in_a = script(a.path(), "tool", "");
        script(b.path(), "tool", "");
        assert_eq!(find_in_path("tool", &p), Some(in_a));
        // relative and empty entries are ignored, bad names refused
        let rel = std::ffi::OsString::from(format!(":relative/dir:{}", b.path().display()));
        assert_eq!(find_in_path("yt-dlp", &rel), Some(in_b));
        assert_eq!(find_in_path("../yt-dlp", &p), None);
        assert_eq!(find_in_path("", &p), None);
        assert_eq!(find_in_path("yt-dlp", OsStr::new("")), None);
    }

    #[test]
    fn symlinked_executable_is_found() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let real = script(b.path(), "real-tool", "");
        std::os::unix::fs::symlink(&real, a.path().join("yt-dlp")).unwrap();
        assert_eq!(find_in_path("yt-dlp", &path_of(&[a.path()])), Some(a.path().join("yt-dlp")));
    }

    #[test]
    fn ffprobe_next_to_ffmpeg_wins_over_path() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        script(a.path(), "ffmpeg", "echo ffmpeg version 7.1");
        script(b.path(), "ffprobe", "echo ffprobe version 6.0");
        // ffprobe only on PATH, after ffmpeg's folder
        let t = Tools::discover(&path_of(&[a.path(), b.path()]));
        assert_eq!(t.ffprobe, Some(b.path().join("ffprobe")));
        // now one next to ffmpeg, listed after the other folder
        script(a.path(), "ffprobe", "echo ffprobe version 7.1");
        let t = Tools::discover(&path_of(&[b.path(), a.path()]));
        assert_eq!(t.ffmpeg, Some(a.path().join("ffmpeg")));
        assert_eq!(t.ffprobe, Some(a.path().join("ffprobe")), "next to ffmpeg first");
        assert_eq!(t.yt_dlp, None);
    }

    #[test]
    fn check_reports_versions_hints_and_old_tools() {
        let d = tempfile::tempdir().unwrap();
        script(d.path(), "yt-dlp", "echo 2024.12.13");
        script(d.path(), "ffmpeg", "echo 'ffmpeg version n9.0.2 Copyright (c) the FFmpeg developers'; echo more");
        script(d.path(), "ffprobe", "echo 'ffprobe version N-117000-gabc Copyright'");
        let st = Tools::discover(&path_of(&[d.path()])).check();
        assert_eq!((st.yt_dlp.state, st.ffmpeg.state, st.ffprobe.state), (ToolState::Ok, ToolState::Ok, ToolState::Ok));
        assert_eq!(st.yt_dlp.version.as_deref(), Some("2024.12.13"));
        assert_eq!(st.ffmpeg.version.as_deref(), Some("n9.0.2"));
        assert!(st.yt_dlp.usable() && st.yt_dlp.message.is_none());
        assert_eq!(st.ffmpeg.path.as_deref(), Some(d.path().join("ffmpeg").to_str().unwrap()));

        script(d.path(), "yt-dlp", "echo 2022.01.01");
        script(d.path(), "ffmpeg", "echo 'ffmpeg version 4.4.2 Copyright'");
        let st = Tools::discover(&path_of(&[d.path()])).check();
        assert_eq!(st.yt_dlp.state, ToolState::TooOld);
        assert_eq!(st.yt_dlp.message.as_deref(), Some("yt-dlp 2022.01.01 is too old (need 2023.01 or newer)."));
        assert!(!st.yt_dlp.usable());
        assert_eq!(st.ffmpeg.message.as_deref(), Some("ffmpeg 4.4.2 is too old (need 5 or newer)."));
    }

    #[test]
    fn missing_tools_get_install_hints() {
        let d = tempfile::tempdir().unwrap();
        let st = Tools::discover(&path_of(&[d.path()])).check();
        assert_eq!(st.yt_dlp.state, ToolState::Missing);
        assert_eq!(
            st.yt_dlp.message.as_deref(),
            Some("yt-dlp was not found. Install it (Arch: sudo pacman -S yt-dlp) and try again.")
        );
        assert!(st.ffmpeg.message.unwrap().contains("sudo pacman -S ffmpeg"));
        assert!(st.ffprobe.message.unwrap().contains("ffprobe was not found"));
        assert!(st.yt_dlp.path.is_none() && !st.yt_dlp.usable());
    }

    #[test]
    fn unreadable_version_and_broken_tools() {
        let d = tempfile::tempdir().unwrap();
        script(d.path(), "yt-dlp", "echo nightly-build");
        script(d.path(), "ffmpeg", "exit 3");
        script(d.path(), "ffprobe", "echo 'ffprobe version 7.0'");
        let st = Tools::discover(&path_of(&[d.path()])).check();
        assert_eq!(st.yt_dlp.state, ToolState::UnknownVersion);
        assert!(st.yt_dlp.usable() && st.yt_dlp.message.unwrap().contains("could not be read"));
        assert_eq!(st.ffmpeg.state, ToolState::Broken);
        assert!(!st.ffmpeg.usable());
        assert_eq!(st.ffprobe.state, ToolState::Ok);
    }

    #[test]
    fn a_tool_that_is_not_runnable_is_broken_not_a_panic() {
        let d = tempfile::tempdir().unwrap();
        // executable, but no shebang and not a binary
        let p = d.path().join("yt-dlp");
        fs::write(&p, "garbage").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        let st = Tools::discover(&path_of(&[d.path()])).check_one(ToolName::YtDlp);
        assert_eq!(st.state, ToolState::Broken);
    }
}
