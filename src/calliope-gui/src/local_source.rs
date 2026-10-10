//! Typed-path handling for the local audio / video import: turns what the user typed or pasted
//! into an absolute path and checks it before any job starts.
//!
//! Pure module (std + `url` only, no crate-internal imports). It only reads metadata and opens
//! the file for reading; it never creates, writes or canonicalises anything.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Extensions offered for local audio files (lowercase, no dot).
pub const AUDIO_EXTENSIONS: &[&str] =
    &["mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "wma", "aiff", "aif"];

/// Extensions offered for local video files (lowercase, no dot).
pub const VIDEO_EXTENSIONS: &[&str] =
    &["mp4", "mkv", "webm", "mov", "avi", "m4v", "flv", "wmv", "mpg", "mpeg", "ts"];

fn has_extension(name: &str, list: &[&str]) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => list.contains(&ext.to_ascii_lowercase().as_str()),
        _ => false,
    }
}

pub fn is_audio_name(name: &str) -> bool {
    has_extension(name, AUDIO_EXTENSIONS)
}

pub fn is_video_name(name: &str) -> bool {
    has_extension(name, VIDEO_EXTENSIONS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathProblem {
    pub reason: &'static str,
    pub message: String,
}

pub const MAX_PATH_BYTES: usize = 4096;

const EMPTY_MESSAGE: &str = "Enter the path of a file or choose one with Browse";

fn problem(reason: &'static str, message: impl Into<String>) -> PathProblem {
    PathProblem { reason, message: message.into() }
}

fn relative() -> PathProblem {
    problem("relative", "Enter the full path, starting with / or ~/")
}

/// Syntax only (touches no file): text the user typed becomes an absolute path.
pub fn normalise(input: &str, home: Option<&Path>) -> Result<PathBuf, PathProblem> {
    let mut s = input.trim();
    if s.is_empty() {
        return Err(problem("empty", EMPTY_MESSAGE));
    }
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'\'' || b[0] == b'"') && b[b.len() - 1] == b[0] {
        s = s[1..s.len() - 1].trim();
    }
    let invalid = || problem("invalid", "The path is not valid");
    if s.is_empty() {
        return Err(invalid());
    }
    if s.len() > MAX_PATH_BYTES || s.chars().any(char::is_control) {
        return Err(invalid());
    }
    let path = if s.starts_with("file:") {
        let url = url::Url::parse(s).map_err(|_| invalid())?;
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid());
        }
        url.to_file_path().map_err(|_| invalid())?
    } else if s == "~" || s.starts_with("~/") {
        let home = home.ok_or_else(relative)?;
        home.join(s[1..].trim_start_matches('/'))
    } else {
        PathBuf::from(s)
    };
    if !path.is_absolute() {
        return Err(relative());
    }
    // percent-decoding (file: links) can introduce control characters or length
    let decoded = path.to_string_lossy();
    if decoded.len() > MAX_PATH_BYTES || decoded.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(path)
}

/// `normalise` plus the file-system checks, in the documented order.
pub fn resolve(input: &str, kind: LocalKind, home: Option<&Path>) -> Result<PathBuf, PathProblem> {
    let path = normalise(input, home)?;
    let p = path.display();
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(problem("missing", format!("No file found at {p}")));
        }
        Err(e) => return Err(unreadable(&p, &e)),
    };
    if meta.is_dir() {
        return Err(problem("folder", format!("{p} is a folder, not a file")));
    }
    if !meta.is_file() {
        return Err(problem("not-file", format!("{p} is not a regular file")));
    }
    if let Err(e) = fs::File::open(&path) {
        return Err(unreadable(&p, &e));
    }
    let name = path.file_name().and_then(|n| n.to_str()).map(str::to_string);
    let (ok, label) = match kind {
        LocalKind::Audio => (name.as_deref().is_some_and(is_audio_name), "an audio"),
        LocalKind::Video => (name.as_deref().is_some_and(is_video_name), "a video"),
    };
    if !ok {
        let n = name.unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        return Err(problem("extension", format!("\"{n}\" is not {label} file")));
    }
    Ok(path)
}

fn unreadable(p: &std::path::Display<'_>, e: &std::io::Error) -> PathProblem {
    if e.kind() == ErrorKind::PermissionDenied {
        problem("unreadable", format!("Cannot read {p} (permission denied)"))
    } else {
        problem("unreadable", format!("Cannot read {p}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"x").unwrap();
        p
    }

    fn reason(r: Result<PathBuf, PathProblem>) -> &'static str {
        r.unwrap_err().reason
    }

    fn is_root() -> bool {
        Command::new("id").arg("-u").output().map(|o| o.stdout == b"0\n").unwrap_or(false)
    }

    #[test]
    fn extension_helpers() {
        assert!(is_audio_name("Song.MP3"));
        assert!(!is_audio_name("mp3"));
        assert!(is_video_name("a.b.mkv"));
        assert!(!is_video_name(".mp4"));
    }

    #[test]
    fn valid_audio_and_video_paths() {
        let d = tempfile::tempdir().unwrap();
        let a = touch(d.path(), "song.mp3");
        let v = touch(d.path(), "clip.MP4");
        assert_eq!(resolve(a.to_str().unwrap(), LocalKind::Audio, None).unwrap(), a);
        assert_eq!(resolve(v.to_str().unwrap(), LocalKind::Video, None).unwrap(), v);
    }

    #[test]
    fn empty_and_whitespace() {
        for s in ["", "   ", "\t \n", "''", "\"\""] {
            let e = resolve(s, LocalKind::Audio, None).unwrap_err();
            if s.contains('\'') || s.contains('"') {
                assert_eq!(e.reason, "invalid", "{s:?}");
            } else {
                assert_eq!(e.reason, "empty");
                assert_eq!(e.message, "Enter the path of a file or choose one with Browse");
            }
        }
    }

    #[test]
    fn missing_message_and_dangling_symlink() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nope.mp3");
        let e = resolve(p.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert_eq!(e.reason, "missing");
        assert_eq!(e.message, format!("No file found at {}", p.display()));
        let l = d.path().join("dangling.mp3");
        symlink(d.path().join("target.mp3"), &l).unwrap();
        assert_eq!(reason(resolve(l.to_str().unwrap(), LocalKind::Audio, None)), "missing");
    }

    #[test]
    fn folder_even_when_named_like_audio() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.mp3");
        fs::create_dir(&f).unwrap();
        let e = resolve(f.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert_eq!(e.reason, "folder");
        assert_eq!(e.message, format!("{} is a folder, not a file", f.display()));
        assert_eq!(reason(resolve(d.path().to_str().unwrap(), LocalKind::Video, None)), "folder");
    }

    #[test]
    fn fifo_is_not_a_regular_file_and_does_not_block() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("pipe.mp3");
        assert!(Command::new("mkfifo").arg(&f).status().unwrap().success());
        let t = Instant::now();
        let e = resolve(f.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(1));
        assert_eq!(e.reason, "not-file");
        assert_eq!(e.message, format!("{} is not a regular file", f.display()));
    }

    #[test]
    fn unreadable_file() {
        if is_root() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = touch(d.path(), "secret.mp3");
        fs::set_permissions(&f, fs::Permissions::from_mode(0o000)).unwrap();
        let e = resolve(f.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert_eq!(e.reason, "unreadable");
        assert_eq!(e.message, format!("Cannot read {} (permission denied)", f.display()));
    }

    #[test]
    fn wrong_type_messages() {
        let d = tempfile::tempdir().unwrap();
        let t = touch(d.path(), "notes.txt");
        let e = resolve(t.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert_eq!((e.reason, e.message.as_str()), ("extension", "\"notes.txt\" is not an audio file"));
        let m4 = touch(d.path(), "song.mp4");
        let e = resolve(m4.to_str().unwrap(), LocalKind::Audio, None).unwrap_err();
        assert_eq!(e.message, "\"song.mp4\" is not an audio file");
        let m3 = touch(d.path(), "song.mp3");
        let e = resolve(m3.to_str().unwrap(), LocalKind::Video, None).unwrap_err();
        assert_eq!(e.message, "\"song.mp3\" is not a video file");
    }

    #[test]
    fn missing_is_reported_before_wrong_type() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("typo.txt");
        assert_eq!(reason(resolve(p.to_str().unwrap(), LocalKind::Audio, None)), "missing");
    }

    #[test]
    fn symlink_to_regular_file_is_accepted() {
        let d = tempfile::tempdir().unwrap();
        let t = touch(d.path(), "real.mp3");
        let l = d.path().join("link.mp3");
        symlink(&t, &l).unwrap();
        assert_eq!(resolve(l.to_str().unwrap(), LocalKind::Audio, None).unwrap(), l);
    }

    #[test]
    fn normalise_whitespace_and_quotes() {
        assert_eq!(normalise("  /a/b.mp3 \n", None).unwrap(), PathBuf::from("/a/b.mp3"));
        assert_eq!(normalise("'/a/b c.mp3'", None).unwrap(), PathBuf::from("/a/b c.mp3"));
        assert_eq!(normalise("\"/a/b c.mp3\"", None).unwrap(), PathBuf::from("/a/b c.mp3"));
        assert_eq!(normalise(" ' /a/b.mp3 ' ", None).unwrap(), PathBuf::from("/a/b.mp3"));
        // mismatched quotes are not stripped, so the path is relative
        assert_eq!(reason(normalise("\"/a/b.mp3'", None)), "relative");
    }

    #[test]
    fn normalise_tilde() {
        let home = Path::new("/home/me");
        assert_eq!(normalise("~", Some(home)).unwrap(), PathBuf::from("/home/me"));
        assert_eq!(normalise("~/a b.mp3", Some(home)).unwrap(), PathBuf::from("/home/me/a b.mp3"));
        assert_eq!(reason(normalise("~/a b.mp3", None)), "relative");
        assert_eq!(reason(normalise("~bob/x.mp3", Some(home))), "relative");
    }

    #[test]
    fn normalise_file_uri() {
        assert_eq!(normalise("file:///tmp/a%20b.mp3", None).unwrap(), PathBuf::from("/tmp/a b.mp3"));
        assert_eq!(normalise("file://localhost/tmp/x.mp3", None).unwrap(), PathBuf::from("/tmp/x.mp3"));
        assert_eq!(reason(normalise("file://otherhost/x.mp3", None)), "invalid");
    }

    #[test]
    fn normalise_rejects_relative_and_control_and_long() {
        let e = normalise("a.mp3", None).unwrap_err();
        assert_eq!(e.reason, "relative");
        assert_eq!(e.message, "Enter the full path, starting with / or ~/");
        assert_eq!(normalise("/a/b\n.mp3", None).unwrap_err().message, "The path is not valid");
        assert_eq!(reason(normalise("/a/b\0.mp3", None)), "invalid");
        assert_eq!(reason(normalise("/a/b\t.mp3", None)), "invalid");
        let ok = format!("/{}", "a".repeat(MAX_PATH_BYTES - 1));
        assert!(normalise(&ok, None).is_ok());
        let long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
        assert_eq!(long.len(), 4097);
        assert_eq!(reason(normalise(&long, None)), "invalid");
    }

    #[test]
    fn normalise_file_uri_decoded_control_query_fragment() {
        assert_eq!(reason(normalise("file:///tmp/a%00b.mp3", None)), "invalid");
        assert_eq!(reason(normalise("file:///tmp/a%0Ab.mp3", None)), "invalid");
        assert_eq!(reason(normalise("file:///tmp/a.mp3?x=1", None)), "invalid");
        assert_eq!(reason(normalise("file:///tmp/a.mp3#frag", None)), "invalid");
        let long = format!("file:///{}", "%61".repeat(MAX_PATH_BYTES));
        assert_eq!(reason(normalise(&long, None)), "invalid");
    }

    #[test]
    fn dotdot_is_kept_as_typed() {
        assert_eq!(normalise("/a/../b.mp3", None).unwrap(), PathBuf::from("/a/../b.mp3"));
    }
}
