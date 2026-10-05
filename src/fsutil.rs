//! File-system helpers shared by the settings store and the track repository. Pure logic, no
//! Tauri. Data safety first: nothing here removes user data (only our own temp files).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Creates parent directories, writes `<path>.tmp` (fsynced), then renames it (atomic) and
/// best-effort fsyncs the directory on Unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(parent.unwrap_or_else(|| Path::new("")));
    Ok(())
}

pub(crate) fn part_path(dest: &Path) -> io::Result<PathBuf> {
    let name = dest
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no file name"))?;
    let mut part = std::ffi::OsString::from(".");
    part.push(name);
    part.push(".part");
    Ok(dest.with_file_name(part))
}

/// Copies `src` to `part` and fsyncs it. The part file is removed on any error.
pub(crate) fn copy_to_part(src: &Path, part: &Path) -> io::Result<()> {
    let r = (|| {
        let mut from = fs::File::open(src)?;
        let mut to = fs::OpenOptions::new().write(true).create_new(true).open(part)?;
        io::copy(&mut from, &mut to)?;
        to.sync_all()
    })();
    if r.is_err() {
        let _ = fs::remove_file(part);
    }
    r
}

/// Copies `src` to `dest` and never overwrites: fails with `AlreadyExists` if `dest` exists.
/// Copies to `.<name>.part`, fsyncs, then `hard_link`s it to `dest` (atomic, fails if `dest`
/// exists) and removes the part. Where hard links are unsupported, falls back to an exists
/// check plus rename. The part file is removed on any error.
pub fn copy_no_clobber(src: &Path, dest: &Path) -> io::Result<()> {
    let part = part_path(dest)?;
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", dest.display()),
        ));
    }
    copy_to_part(src, &part)?;
    let r = match fs::hard_link(&part, dest) {
        Ok(()) => fs::remove_file(&part),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
            ) =>
        {
            if dest.exists() {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} already exists", dest.display()),
                ))
            } else {
                fs::rename(&part, dest)
            }
        }
        Err(e) => Err(e),
    };
    if r.is_err() {
        let _ = fs::remove_file(&part);
    }
    if r.is_ok() {
        if let Some(p) = dest.parent() {
            sync_dir(p);
        }
    }
    r
}

/// Copies `src` to `dest`, replacing an existing file atomically (part file + rename). Only
/// for an overwrite the user confirmed in the save dialog.
pub fn copy_replace(src: &Path, dest: &Path) -> io::Result<()> {
    let part = part_path(dest)?;
    copy_to_part(src, &part)?;
    if let Err(e) = fs::rename(&part, dest) {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    if let Some(p) = dest.parent() {
        sync_dir(p);
    }
    Ok(())
}

/// Rules for a plain file name in track metadata (plan section 2.2).
pub fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("file name is empty".into());
    }
    if name.len() > 255 {
        return Err("file name is longer than 255 bytes".into());
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return Err(format!("file name \"{name}\" must not start with a dot"));
    }
    if name == "track.json" {
        return Err("\"track.json\" is reserved".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(format!("file name \"{name}\" contains the character {c:?}"));
    }
    Ok(())
}

/// 64-bit FNV-1a hash as 16 lower-case hex digits.
pub fn fnv1a64_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Creates `parent/base`, or `parent/base (2)`, `base (3)`, ... if taken. Returns the path.
/// `create_dir` fails if the folder exists, so two callers never get the same folder.
pub fn create_unique_dir(parent: &Path, base: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(parent)?;
    for n in 1u32.. {
        let name = if n == 1 { base.to_string() } else { format!("{base} ({n})") };
        let p = parent.join(name);
        match fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn bad_input(m: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, m)
}

/// Canonicalises `root` and `src` and checks that `src` is strictly inside `root` and not
/// already under `root/trash`. Returns `(root, src, trash)`, all canonical.
fn trash_guard(root: &Path, src: &Path) -> io::Result<(PathBuf, PathBuf, PathBuf)> {
    let root_c = root.canonicalize()?;
    let src_c = src.canonicalize()?;
    if src_c == root_c || !src_c.starts_with(&root_c) {
        return Err(bad_input(format!("{} is not inside the repository", src.display())));
    }
    let trash = root_c.join("trash");
    if src_c.starts_with(&trash) {
        return Err(bad_input("item is already in the trash".into()));
    }
    Ok((root_c, src_c, trash))
}

/// Moves a whole track folder `src` (inside `root`) to `root/trash/<stamp>-<id>/` with one
/// `rename` (the folder itself becomes the trash folder, no extra nesting). If that name
/// exists, `-2`, `-3`, ... is appended. Returns the new path. Nothing is ever removed.
/// Errors if `src` is not strictly inside `root` or is already under `trash/`.
pub fn move_track_to_trash(root: &Path, src: &Path, stamp: &str, id: &str) -> io::Result<PathBuf> {
    let (_, src_c, trash) = trash_guard(root, src)?;
    fs::create_dir_all(&trash)?;
    let base = format!("{stamp}-{id}");
    for n in 1u32.. {
        let dest = trash.join(if n == 1 { base.clone() } else { format!("{base}-{n}") });
        if dest.exists() {
            continue;
        }
        fs::rename(&src_c, &dest)?;
        sync_dir(&trash);
        return Ok(dest);
    }
    unreachable!()
}

/// One trash folder `root/trash/<stamp>-<id>-tablatures/` for all tablature files removed or
/// replaced in one save. The folder is created on the first `move_in` (`-2`, ... if the name
/// is taken) and reused for the following files; a name clash inside it gives the file a
/// ` (2)` suffix instead of a new folder.
pub struct TablatureTrash {
    root: PathBuf,
    base: String,
    dir: Option<PathBuf>,
}

impl TablatureTrash {
    pub fn new(root: &Path, stamp: &str, id: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            base: format!("{stamp}-{id}-tablatures"),
            dir: None,
        }
    }

    /// The trash folder, once a file has been moved in.
    #[allow(dead_code)] // used by tests
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Renames the file `src` into the trash folder and returns its new path.
    pub fn move_in(&mut self, src: &Path) -> io::Result<PathBuf> {
        let (_, src_c, trash) = trash_guard(&self.root, src)?;
        let name = src_c
            .file_name()
            .ok_or_else(|| bad_input("source has no file name".into()))?
            .to_string_lossy()
            .into_owned();
        let dir = match &self.dir {
            Some(d) => d.clone(),
            None => {
                fs::create_dir_all(&trash)?;
                let mut made = None;
                for n in 1u32.. {
                    let d = trash.join(if n == 1 {
                        self.base.clone()
                    } else {
                        format!("{}-{n}", self.base)
                    });
                    match fs::create_dir(&d) {
                        Ok(()) => {
                            made = Some(d);
                            break;
                        }
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(e) => return Err(e),
                    }
                }
                let d = made.expect("loop returns or breaks with a folder");
                self.dir = Some(d.clone());
                d
            }
        };
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        for n in 1u32.. {
            let file = if n == 1 { name.clone() } else { format!("{stem} ({n}){ext}") };
            let dest = dir.join(file);
            if dest.exists() {
                continue;
            }
            fs::rename(&src_c, &dest)?;
            sync_dir(&dir);
            return Ok(dest);
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_writes_and_leaves_no_tmp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/x.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        assert!(!d.path().join("sub/x.json.tmp").exists());
    }

    #[test]
    fn no_clobber_copies_and_leaves_no_part() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.bin");
        fs::write(&src, b"data").unwrap();
        let dest = d.path().join("dest.bin");
        copy_no_clobber(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"data");
        assert!(!d.path().join(".dest.bin.part").exists());
    }

    #[test]
    fn no_clobber_refuses_existing_dest() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.bin");
        let dest = d.path().join("dest.bin");
        fs::write(&src, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        let e = copy_no_clobber(&src, &dest).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!d.path().join(".dest.bin.part").exists());
    }

    #[test]
    fn no_clobber_missing_source_leaves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("dest.bin");
        assert!(copy_no_clobber(&d.path().join("nope"), &dest).is_err());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn replace_overwrites_atomically() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.bin");
        let dest = d.path().join("dest.bin");
        fs::write(&src, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        copy_replace(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!d.path().join(".dest.bin.part").exists());
    }

    #[test]
    fn file_names() {
        for bad in ["..", ".", "a/b", ".x", "track.json", "a\\b", "x?", "", "a\nb", "a:b"] {
            assert!(validate_file_name(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_file_name(&"a".repeat(256)).is_err());
        assert!(validate_file_name(&"a".repeat(255)).is_ok());
        assert!(validate_file_name("Slow Burn (live).gp5").is_ok());
        assert!(validate_file_name("track.json.bak").is_ok());
    }

    #[test]
    fn fnv() {
        assert_eq!(fnv1a64_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a64_hex(b"a"), "af63dc4c8601ec8c");
    }

    #[test]
    fn unique_dirs() {
        let d = tempfile::tempdir().unwrap();
        let a = create_unique_dir(d.path(), "T").unwrap();
        let b = create_unique_dir(d.path(), "T").unwrap();
        let c = create_unique_dir(d.path(), "T").unwrap();
        assert_eq!(a.file_name().unwrap(), "T");
        assert_eq!(b.file_name().unwrap(), "T (2)");
        assert_eq!(c.file_name().unwrap(), "T (3)");
    }

    #[test]
    fn track_trash_renames_folder_without_nesting() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        for i in 0..2 {
            let t = root.join("tracks/abc");
            fs::create_dir_all(&t).unwrap();
            fs::write(t.join("track.json"), format!("{i}")).unwrap();
            let moved = move_track_to_trash(root, &t, "S", "abc").unwrap();
            assert!(!t.exists());
            assert_eq!(fs::read_to_string(moved.join("track.json")).unwrap(), format!("{i}"));
        }
        assert_eq!(fs::read_to_string(root.join("trash/S-abc/track.json")).unwrap(), "0");
        assert_eq!(fs::read_to_string(root.join("trash/S-abc-2/track.json")).unwrap(), "1");
    }

    #[test]
    fn tablature_trash_uses_one_folder_per_save() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let t = root.join("tracks/abc");
        fs::create_dir_all(&t).unwrap();
        fs::write(t.join("a.gp5"), b"a1").unwrap();
        fs::write(t.join("b.gp5"), b"b").unwrap();
        let mut tt = TablatureTrash::new(root, "S", "abc");
        assert!(tt.dir().is_none());
        assert!(!root.join("trash").exists());
        let m1 = tt.move_in(&t.join("a.gp5")).unwrap();
        // same name again in the same save: suffix on the file, same folder
        fs::write(t.join("a.gp5"), b"a2").unwrap();
        let m2 = tt.move_in(&t.join("a.gp5")).unwrap();
        let m3 = tt.move_in(&t.join("b.gp5")).unwrap();
        let dir = root.canonicalize().unwrap().join("trash/S-abc-tablatures");
        assert_eq!(tt.dir().unwrap(), dir);
        assert_eq!(m1, dir.join("a.gp5"));
        assert_eq!(m2, dir.join("a (2).gp5"));
        assert_eq!(m3, dir.join("b.gp5"));
        assert_eq!(fs::read(&m1).unwrap(), b"a1");
        assert_eq!(fs::read(&m2).unwrap(), b"a2");
        assert_eq!(fs::read_dir(root.join("trash")).unwrap().count(), 1);
        // a later save gets a new folder
        fs::write(t.join("c.gp5"), b"c").unwrap();
        let mut t2 = TablatureTrash::new(root, "S", "abc");
        t2.move_in(&t.join("c.gp5")).unwrap();
        assert!(root.join("trash/S-abc-tablatures-2/c.gp5").exists());
    }

    #[test]
    fn trash_refuses_outside_root_root_itself_and_trash() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("repo");
        fs::create_dir_all(&root).unwrap();
        let outside = d.path().join("other.txt");
        fs::write(&outside, b"x").unwrap();
        assert!(move_track_to_trash(&root, &outside, "S", "abc").is_err());
        assert!(outside.exists());
        assert!(move_track_to_trash(&root, &root, "S", "abc").is_err());
        assert!(TablatureTrash::new(&root, "S", "abc").move_in(&outside).is_err());
        assert!(!root.join("trash").exists());
        fs::create_dir_all(root.join("trash/old")).unwrap();
        assert!(move_track_to_trash(&root, &root.join("trash/old"), "S", "abc").is_err());
        assert!(root.join("trash/old").exists());
    }
}
