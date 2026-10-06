//! Running the separator and checking what it produced.

use std::io::Read;
use std::path::{Path, PathBuf};

use calliope_common::stems_api::{is_valid_stem_name, MAX_STEMS};

/// The usual stems come first, in this order; any others follow alphabetically.
const STANDARD_ORDER: [&str; 6] = ["vocals", "drums", "bass", "guitar", "piano", "other"];

pub const INVALID_OUTPUT: &str = "separator produced invalid output";

/// Parses a `progress <0..1>` line.
pub fn parse_progress(line: &str) -> Option<f64> {
    let v: f64 = line.strip_prefix("progress ")?.trim().parse().ok()?;
    (v.is_finite() && (0.0..=1.0).contains(&v)).then_some(v)
}

pub fn sort_stems(stems: &mut [String]) {
    let rank = |s: &str| STANDARD_ORDER.iter().position(|n| *n == s).unwrap_or(STANDARD_ORDER.len());
    stems.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
}

/// Checks `out_dir`: only regular files `<stem>.flac` with a valid stem name, 1 to 16 of them,
/// each starting with `fLaC`. Returns the stem names in the documented order.
pub fn validate_output(out_dir: &Path) -> Result<Vec<String>, String> {
    let bad = |why: &str| Err(format!("{INVALID_OUTPUT} ({why})"));
    let entries = match std::fs::read_dir(out_dir) {
        Ok(e) => e,
        Err(_) => return bad("no output folder"),
    };
    let mut stems = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { return bad("unreadable folder") };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return bad("odd file name") };
        let Some(stem) = name.strip_suffix(".flac") else { return bad("unexpected file") };
        if !is_valid_stem_name(stem) {
            return bad("bad stem name");
        }
        let path: PathBuf = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => {}
            _ => return bad("not a regular file"),
        }
        let mut magic = [0u8; 4];
        let ok = std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut magic)).is_ok();
        if !ok || &magic != b"fLaC" {
            return bad("stem is not FLAC");
        }
        stems.push(stem.to_string());
        if stems.len() > MAX_STEMS {
            return bad("too many stems");
        }
    }
    if stems.is_empty() {
        return bad("no stems");
    }
    sort_stems(&mut stems);
    Ok(stems)
}

/// Shortens `s` to at most `max` characters.
pub fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stems_dir(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (n, c) in files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        d
    }

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress("progress 0.5"), Some(0.5));
        assert_eq!(parse_progress("progress 1"), Some(1.0));
        assert_eq!(parse_progress("progress 1.5"), None);
        assert_eq!(parse_progress("progress -1"), None);
        assert_eq!(parse_progress("progress NaN"), None);
        assert_eq!(parse_progress("progress"), None);
        assert_eq!(parse_progress("hello progress 0.5"), None);
    }

    #[test]
    fn documented_order() {
        let d = stems_dir(&[
            ("other.flac", b"fLaC1"), ("zeta.flac", b"fLaC1"), ("vocals.flac", b"fLaC1"),
            ("piano.flac", b"fLaC1"), ("alpha.flac", b"fLaC1"), ("guitar.flac", b"fLaC1"),
            ("bass.flac", b"fLaC1"), ("drums.flac", b"fLaC1"),
        ]);
        assert_eq!(
            validate_output(d.path()).unwrap(),
            ["vocals", "drums", "bass", "guitar", "piano", "other", "alpha", "zeta"]
        );
    }

    #[test]
    fn rejects_bad_outputs() {
        let ok = (&"vocals.flac", &b"fLaC.."[..]);
        let cases: Vec<Vec<(&str, &[u8])>> = vec![
            vec![],
            vec![("README.txt", b"hi")],
            vec![(*ok.0, ok.1), ("README.txt", b"hi")],
            vec![("Vocals.flac", b"fLaC")],
            vec![("..flac", b"fLaC")],
            vec![(".hidden.flac", b"fLaC")],
            vec![("a b.flac", b"fLaC")],
            vec![("vocals.flac", b"not flac")],
            vec![("vocals.flac", b"fL")],
            vec![("vocals.flac", b"")],
        ];
        for files in cases {
            let d = stems_dir(&files);
            let r = validate_output(d.path());
            assert!(r.as_ref().is_err_and(|e| e.starts_with(INVALID_OUTPUT)), "{files:?} -> {r:?}");
        }
        assert!(validate_output(Path::new("/nonexistent/out")).is_err());
    }

    #[test]
    fn rejects_subfolders_symlinks_and_too_many() {
        let d = stems_dir(&[("vocals.flac", b"fLaC")]);
        std::fs::create_dir(d.path().join("sub.flac")).unwrap();
        assert!(validate_output(d.path()).is_err());

        let d = stems_dir(&[]);
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("vocals.flac")).unwrap();
        assert!(validate_output(d.path()).is_err());

        let files: Vec<(String, &[u8])> = (0..17).map(|i| (format!("s{i:02}.flac"), &b"fLaC"[..])).collect();
        let d = tempfile::tempdir().unwrap();
        for (n, c) in &files {
            std::fs::write(d.path().join(n), c).unwrap();
        }
        assert!(validate_output(d.path()).is_err());
        std::fs::remove_file(d.path().join("s16.flac")).unwrap();
        assert_eq!(validate_output(d.path()).unwrap().len(), 16);
    }

    #[test]
    fn truncates_on_char_boundaries() {
        assert_eq!(truncate("abcdef", 3), "abc");
        assert_eq!(truncate("ab", 3), "ab");
        assert_eq!(truncate("äöüß", 2), "äö");
    }
}
