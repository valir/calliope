//! The work directory: `<work>/jobs/<id>/` per job. Only folders carrying the marker file are
//! ever deleted; everything else in the work directory is left alone.

use std::io;
use std::path::{Path, PathBuf};

use calliope_common::stems_api::is_valid_job_id;

pub const MARKER: &str = ".calliope-stems-job";

pub fn jobs_root(work_dir: &Path) -> PathBuf {
    work_dir.join("jobs")
}

pub fn ensure(work_dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(jobs_root(work_dir))
}

/// Creates `<jobs>/<id>/` with its marker.
pub fn create_job_dir(work_dir: &Path, id: &str) -> io::Result<PathBuf> {
    debug_assert!(is_valid_job_id(id));
    let dir = jobs_root(work_dir).join(id);
    std::fs::create_dir(&dir)?;
    std::fs::write(dir.join(MARKER), b"calliope-stems job folder; deleted by the server\n")?;
    Ok(dir)
}

fn is_own_dir(dir: &Path) -> bool {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => {}
        _ => return false,
    }
    matches!(std::fs::symlink_metadata(dir.join(MARKER)), Ok(m) if m.is_file())
}

/// Deletes the job folder if it is one of ours. A missing folder is fine.
pub fn remove_job_dir(work_dir: &Path, id: &str) -> io::Result<()> {
    if !is_valid_job_id(id) {
        return Ok(());
    }
    let dir = jobs_root(work_dir).join(id);
    if is_own_dir(&dir) {
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Start-up cleanup: deletes every `jobs/<id>/` folder that has the marker. Returns how many.
pub fn clean_start(work_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(jobs_root(work_dir)) else { return 0 };
    let mut removed = 0;
    for e in entries.flatten() {
        let path = e.path();
        if is_own_dir(&path) && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_up_cleanup_removes_marked_folders_only() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path();
        ensure(w).unwrap();
        let a = create_job_dir(w, "aaaa-1").unwrap();
        std::fs::write(a.join("input.flac"), b"x").unwrap();
        let b = create_job_dir(w, "bbbb-2").unwrap();
        // Not ours: no marker.
        let foreign = jobs_root(w).join("keepme");
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("precious.txt"), b"x").unwrap();
        // A loose file and a symlink to a marked-looking folder elsewhere.
        std::fs::write(jobs_root(w).join("loose.txt"), b"x").unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join(MARKER), b"").unwrap();
        std::fs::write(outside.join("data"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, jobs_root(w).join("cccc-3")).unwrap();
        // Something next to jobs/.
        std::fs::write(w.join("notes.txt"), b"x").unwrap();

        assert_eq!(clean_start(w), 2);
        assert!(!a.exists() && !b.exists());
        assert!(foreign.join("precious.txt").exists());
        assert!(jobs_root(w).join("loose.txt").exists());
        assert!(outside.join("data").exists(), "a symlink must not be followed");
        assert!(w.join("notes.txt").exists());
    }

    #[test]
    fn remove_job_dir_refuses_foreign_and_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path();
        ensure(w).unwrap();
        let foreign = jobs_root(w).join("abcd");
        std::fs::create_dir(&foreign).unwrap();
        remove_job_dir(w, "abcd").unwrap();
        assert!(foreign.exists());
        remove_job_dir(w, "../x").unwrap();
        remove_job_dir(w, "missing").unwrap();
        let own = create_job_dir(w, "abce").unwrap();
        remove_job_dir(w, "abce").unwrap();
        assert!(!own.exists());
        remove_job_dir(w, "abce").unwrap();
    }
}
