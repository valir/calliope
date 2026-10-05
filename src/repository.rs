//! The on-disk track repository: layout, scan, save transaction, delete and export.
//! Pure file logic, no Tauri. See plan sections 2.2 to 2.4.
//!
//! Data safety rules: nothing a user put in the repository is ever deleted or overwritten.
//! Removed and replaced files go to `trash/`; the only things removed are Calliope's own
//! temp/part files and its own freshly created copies when a transaction is rolled back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::track_meta::{self, TrackEdits, TrackMeta};

pub const MARKER: &str = "calliope-repository.json";
pub const REPO_SCHEMA: u64 = 1;
const TRACK_FILE: &str = "track.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoStatus {
    /// Marker present.
    Ok,
    /// Exists and is empty.
    Empty,
    /// Exists, not empty, no (readable) marker.
    Other,
    Missing,
    /// Marker written by a newer Calliope.
    Newer,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackRecord {
    pub id: String,
    pub band: String,
    pub album: String,
    pub title: String,
    pub composers: Vec<String>,
    pub year: Option<i64>,
    pub source_url: Option<String>,
    pub copyright: Option<String>,
    pub audio: String,
    pub tablatures: Vec<String>,
    pub imported: String,
    pub modified: String,
    pub revision: String,
    /// Files listed in the metadata that are not on disk.
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub dir: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Library {
    pub root: String,
    pub tracks: Vec<TrackRecord>,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TabEntry {
    Keep { name: String },
    Add { token: String },
    Replace { name: String, token: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SaveTrackRequest {
    pub id: String,
    pub revision: String,
    pub edits: TrackEdits,
    pub tablatures: Vec<TabEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SaveResult {
    pub track: TrackRecord,
    pub warnings: Vec<String>,
}

/// A track to create from scratch (tests, the fixture builder, the future import feature).
#[derive(Debug, Clone)]
#[allow(dead_code)] // for tests and the future import feature
pub struct NewTrack {
    /// `None` generates a UUIDv7.
    pub id: Option<String>,
    pub edits: TrackEdits,
    /// File name of the audio inside the track folder.
    pub audio_name: String,
    /// `(file name in the track folder, source path)`.
    pub tablatures: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub root: PathBuf,
}

fn io_err(what: &str, e: io::Error) -> String {
    format!("{what}: {e}")
}

fn stamp_now() -> String {
    track_meta::now_rfc3339().replace(['-', ':'], "")
}

fn read_marker(root: &Path) -> Option<Result<u64, ()>> {
    let bytes = match fs::read(root.join(MARKER)) {
        Ok(b) => b,
        Err(_) => return None,
    };
    let v: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return Some(Err(())),
    };
    Some(v.get("schema_version").and_then(|n| n.as_u64()).ok_or(()))
}

pub fn status(root: &Path) -> RepoStatus {
    if !root.is_dir() {
        return RepoStatus::Missing;
    }
    match read_marker(root) {
        Some(Ok(n)) if n > REPO_SCHEMA => return RepoStatus::Newer,
        Some(Ok(_)) => return RepoStatus::Ok,
        _ => {}
    }
    match fs::read_dir(root) {
        Ok(mut it) => {
            if it.next().is_none() {
                RepoStatus::Empty
            } else {
                RepoStatus::Other
            }
        }
        Err(_) => RepoStatus::Other,
    }
}

/// Is `path` (canonicalised, or its nearest existing ancestor) inside `<root>/tracks`?
fn inside_tracks(root: &Path, path: &Path) -> bool {
    let Ok(tracks) = root.join("tracks").canonicalize() else {
        return false;
    };
    let mut p = path.to_path_buf();
    loop {
        if let Ok(c) = p.canonicalize() {
            return c.starts_with(&tracks);
        }
        if !p.pop() {
            return false;
        }
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    if track_meta::is_valid_id(id) {
        Ok(())
    } else {
        Err(format!("invalid track id \"{id}\""))
    }
}

fn is_dot_or_tmp(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(".tmp")
}

struct Loaded {
    meta: TrackMeta,
    revision: String,
}

impl Repository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn tracks_dir(&self) -> PathBuf {
        self.root.join("tracks")
    }

    /// Adds the marker and `tracks/` only. Never changes anything else.
    pub fn ensure_layout(&self) -> Result<(), String> {
        if let Some(Ok(n)) = read_marker(&self.root) {
            if n > REPO_SCHEMA {
                return Err("the repository was written by a newer Calliope".into());
            }
        }
        fs::create_dir_all(self.tracks_dir()).map_err(|e| io_err("cannot create tracks folder", e))?;
        let marker = self.root.join(MARKER);
        if !marker.exists() {
            fsutil::write_atomic(&marker, format!("{{\n  \"schema_version\": {REPO_SCHEMA}\n}}\n").as_bytes())
                .map_err(|e| io_err("cannot write repository marker", e))?;
        }
        Ok(())
    }

    /// `<root>/tracks/<id>`, for a valid id naming a real folder (not a symlink).
    fn track_dir(&self, id: &str) -> Result<PathBuf, String> {
        valid_id(id)?;
        let dir = self.tracks_dir().join(id);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => Ok(dir),
            Ok(_) => Err(format!("\"{id}\" is not a track folder")),
            Err(_) => Err(format!("track \"{id}\" not found")),
        }
    }

    fn load(&self, id: &str) -> Result<(PathBuf, Loaded), String> {
        let dir = self.track_dir(id)?;
        let bytes = fs::read(dir.join(TRACK_FILE)).map_err(|e| io_err("cannot read track.json", e))?;
        let meta = track_meta::parse(&bytes, id).map_err(|e| format!("track \"{id}\": {e}"))?;
        Ok((dir, Loaded { meta, revision: fsutil::fnv1a64_hex(&bytes) }))
    }

    fn record(dir: &Path, l: Loaded) -> TrackRecord {
        let m = l.meta;
        let missing = std::iter::once(&m.audio)
            .chain(m.tablatures.iter())
            .filter(|n| !dir.join(n).is_file())
            .cloned()
            .collect();
        TrackRecord {
            id: m.id,
            band: m.band,
            album: m.album,
            title: m.title,
            composers: m.composers,
            year: m.year,
            source_url: m.source_url,
            copyright: m.copyright,
            audio: m.audio,
            tablatures: m.tablatures,
            imported: m.imported,
            modified: m.modified,
            revision: l.revision,
            missing,
        }
    }

    fn load_record(&self, id: &str) -> Result<TrackRecord, String> {
        let (dir, l) = self.load(id)?;
        Ok(Self::record(&dir, l))
    }

    /// Reads every track folder. Unreadable ones become problems; the scan never fails and
    /// never writes.
    pub fn scan(&self) -> Library {
        let mut tracks = Vec::new();
        let mut problems = Vec::new();
        let mut names: Vec<(String, bool)> = Vec::new();
        if let Ok(rd) = fs::read_dir(self.tracks_dir()) {
            for e in rd.flatten() {
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                names.push((e.file_name().to_string_lossy().into_owned(), is_dir));
            }
        }
        names.sort();
        for (name, is_dir) in names {
            if !is_dir || is_dot_or_tmp(&name) {
                continue;
            }
            let problem = |message: String| Problem { dir: name.clone(), message };
            if !track_meta::is_valid_id(&name) {
                problems.push(problem("folder name is not a valid track id".into()));
                continue;
            }
            match self.load_record(&name) {
                Ok(r) => tracks.push(r),
                Err(m) => problems.push(problem(m)),
            }
        }
        Library { root: self.root.to_string_lossy().into_owned(), tracks, problems }
    }

    /// Creates a new track folder (the id must not exist yet). On failure only what this call
    /// created is removed.
    #[allow(dead_code)] // for tests and the future import feature
    pub fn create_track(&self, new: NewTrack, audio_src: &Path) -> Result<TrackRecord, String> {
        let id = new.id.unwrap_or_else(track_meta::new_id);
        valid_id(&id)?;
        let now = track_meta::now_rfc3339();
        let e = new.edits;
        let meta = TrackMeta {
            schema_version: track_meta::CURRENT_SCHEMA,
            id: id.clone(),
            band: e.band.trim().to_string(),
            album: e.album.trim().to_string(),
            title: e.title.trim().to_string(),
            composers: e.composers.iter().map(|c| c.trim().to_string()).collect(),
            year: e.year,
            source_url: e.source_url.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            copyright: e.copyright.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            audio: new.audio_name.clone(),
            tablatures: new.tablatures.iter().map(|(n, _)| n.clone()).collect(),
            imported: now.clone(),
            modified: now,
            extra: Default::default(),
        };
        track_meta::validate_for_write(&meta)?;
        let mut files: Vec<(&str, &Path)> = vec![(meta.audio.as_str(), audio_src)];
        files.extend(new.tablatures.iter().map(|(n, p)| (n.as_str(), p.as_path())));
        for (_, src) in &files {
            if !src.is_file() {
                return Err(format!("{} is not a file", src.display()));
            }
        }
        fs::create_dir_all(self.tracks_dir()).map_err(|e| io_err("cannot create tracks folder", e))?;
        let dir = self.tracks_dir().join(&id);
        fs::create_dir(&dir).map_err(|e| io_err(&format!("cannot create track folder \"{id}\""), e))?;
        let mut created: Vec<PathBuf> = Vec::new();
        let result = (|| {
            for (name, src) in &files {
                let dest = dir.join(name);
                fsutil::copy_no_clobber(src, &dest).map_err(|e| io_err(&format!("cannot copy {name}"), e))?;
                created.push(dest);
            }
            fsutil::write_atomic(&dir.join(TRACK_FILE), track_meta::to_json_pretty(&meta).as_bytes())
                .map_err(|e| io_err("cannot write track.json", e))
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(dir.join("track.json.tmp"));
            for f in created {
                let _ = fs::remove_file(f);
            }
            let _ = fs::remove_dir(&dir);
            return Err(e);
        }
        self.load_record(&id)
    }

    /// The save transaction (plan 2.4). `lookup` resolves a pick token to the picked file.
    pub fn save_track(
        &self,
        req: SaveTrackRequest,
        lookup: &dyn Fn(&str) -> Option<PathBuf>,
    ) -> Result<SaveResult, String> {
        // 1. resolve, re-read, compare the revision
        let (dir, loaded) = self.load(&req.id)?;
        if loaded.revision != req.revision {
            return Err("conflict: track.json changed on disk since it was loaded".into());
        }
        let old = loaded.meta;

        // 2. validate edits, tab entries and sources
        let mut meta = old.clone();
        track_meta::apply_edits(&mut meta, req.edits)?;

        struct Plan {
            name: String,
            src: Option<PathBuf>,
            /// old name replaced by this entry
            replaces: Option<String>,
        }
        let mut plan: Vec<Plan> = Vec::new();
        let mut used_old: Vec<String> = Vec::new();
        let mut take_old = |name: &str| -> Result<(), String> {
            if !old.tablatures.iter().any(|t| t == name) {
                return Err(format!("unknown tablature \"{name}\""));
            }
            if used_old.iter().any(|u| u == name) {
                return Err(format!("tablature \"{name}\" is listed twice"));
            }
            used_old.push(name.to_string());
            Ok(())
        };
        let resolve = |token: &str| -> Result<(String, PathBuf), String> {
            let path = lookup(token).ok_or_else(|| format!("unknown or expired file pick \"{token}\""))?;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("the picked file has no usable name")?
                .to_string();
            fsutil::validate_file_name(&name).map_err(|e| format!("picked file: {e}"))?;
            if !path.is_file() {
                return Err(format!("{} is not a file", path.display()));
            }
            Ok((name, path))
        };
        for entry in &req.tablatures {
            match entry {
                TabEntry::Keep { name } => {
                    take_old(name)?;
                    plan.push(Plan { name: name.clone(), src: None, replaces: None });
                }
                TabEntry::Add { token } => {
                    let (name, path) = resolve(token)?;
                    plan.push(Plan { name, src: Some(path), replaces: None });
                }
                TabEntry::Replace { name: old_name, token } => {
                    take_old(old_name)?;
                    let (name, path) = resolve(token)?;
                    plan.push(Plan { name, src: Some(path), replaces: Some(old_name.clone()) });
                }
            }
        }
        let mut seen: Vec<String> = vec![old.audio.to_lowercase()];
        for p in &plan {
            fsutil::validate_file_name(&p.name)?;
            let lower = p.name.to_lowercase();
            if seen.contains(&lower) {
                return Err(format!("tablature name \"{}\" clashes with another file of the track", p.name));
            }
            seen.push(lower);
        }
        meta.tablatures = plan.iter().map(|p| p.name.clone()).collect();
        track_meta::validate_for_read(&meta)?;
        meta.modified = track_meta::now_rfc3339();

        // 3. copy new files in
        let mut created: Vec<PathBuf> = Vec::new();
        // (part file, final path) for same-name replaces
        let mut parts: Vec<(PathBuf, PathBuf)> = Vec::new();
        let rollback = |created: &[PathBuf], parts: &[(PathBuf, PathBuf)]| {
            for f in created {
                let _ = fs::remove_file(f);
            }
            for (part, _) in parts {
                let _ = fs::remove_file(part);
            }
        };
        for p in &plan {
            let Some(src) = &p.src else { continue };
            let dest = dir.join(&p.name);
            let same_name = p.replaces.as_deref() == Some(p.name.as_str());
            let r = if same_name {
                let part = fsutil::part_path(&dest).map_err(|e| io_err("cannot stage file", e));
                match part {
                    Ok(part) => fsutil::copy_to_part(src, &part).map(|()| parts.push((part, dest.clone()))),
                    Err(e) => {
                        rollback(&created, &parts);
                        return Err(e);
                    }
                }
            } else {
                fsutil::copy_no_clobber(src, &dest).map(|()| created.push(dest.clone()))
            };
            if let Err(e) = r {
                rollback(&created, &parts);
                return Err(io_err(&format!("cannot copy \"{}\" into the track", p.name), e));
            }
        }

        // 4. write track.json atomically
        let json = track_meta::to_json_pretty(&meta);
        if let Err(e) = fsutil::write_atomic(&dir.join(TRACK_FILE), json.as_bytes()) {
            let _ = fs::remove_file(dir.join("track.json.tmp"));
            rollback(&created, &parts);
            return Err(io_err("cannot write track.json (nothing was changed)", e));
        }

        // 5. old files to the trash; failures are warnings
        let mut warnings = Vec::new();
        let mut trash = fsutil::TablatureTrash::new(&self.root, &stamp_now(), &req.id);
        let mut to_trash: Vec<&String> = old.tablatures.iter().filter(|t| !used_old.contains(t)).collect();
        to_trash.extend(plan.iter().filter_map(|p| p.replaces.as_ref()));
        for name in to_trash {
            let path = dir.join(name);
            let same_name_part = parts.iter().any(|(_, dest)| *dest == path);
            if !path.exists() {
                continue;
            }
            match trash.move_in(&path) {
                Ok(_) => {}
                Err(e) => {
                    warnings.push(format!("could not move \"{name}\" to the trash: {e}"));
                    if same_name_part {
                        if let Some((part, _)) = parts.iter().find(|(_, d)| *d == path) {
                            warnings.push(format!(
                                "the new version of \"{name}\" was left as {}",
                                part.display()
                            ));
                        }
                        parts.retain(|(_, d)| *d != path);
                    }
                }
            }
        }
        for (part, dest) in &parts {
            if let Err(e) = fs::rename(part, dest) {
                warnings.push(format!(
                    "could not move the new version into place ({e}); it was left as {}",
                    part.display()
                ));
            }
        }
        for w in &warnings {
            eprintln!("calliope: repository warning: {w}");
        }

        // 6. re-scan the record
        let track = self.load_record(&req.id)?;
        Ok(SaveResult { track, warnings })
    }

    /// Moves the whole track folder to `trash/`.
    pub fn delete_track(&self, id: &str, revision: &str) -> Result<(), String> {
        let (dir, loaded) = self.load(id)?;
        if loaded.revision != revision {
            return Err("conflict: track.json changed on disk since it was loaded".into());
        }
        fsutil::move_track_to_trash(&self.root, &dir, &stamp_now(), id)
            .map(|_| ())
            .map_err(|e| io_err("cannot move the track to the trash", e))
    }

    /// Copies the track into a new `"Band - Album - Title"` folder under `dest_parent`.
    pub fn export_track(&self, id: &str, dest_parent: &Path) -> Result<PathBuf, String> {
        let (dir, loaded) = self.load(id)?;
        let m = loaded.meta;
        if !dest_parent.is_dir() {
            return Err(format!("{} is not a folder", dest_parent.display()));
        }
        if inside_tracks(&self.root, dest_parent) {
            return Err("cannot export into the repository's tracks folder".into());
        }
        let mut names = vec![TRACK_FILE.to_string(), m.audio.clone()];
        names.extend(m.tablatures.iter().cloned());
        let missing: Vec<&String> = names.iter().filter(|n| !dir.join(n).is_file()).collect();
        if !missing.is_empty() {
            return Err(format!(
                "cannot export: missing file(s) {}",
                missing.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")
            ));
        }
        let parts: Vec<String> = [&m.band, &m.album, &m.title]
            .iter()
            .map(|s| sanitize_component(s))
            .filter(|s| !s.is_empty())
            .collect();
        let mut folder = parts.join(" - ");
        if folder.is_empty() {
            folder = id.to_string();
        }
        let dest = fsutil::create_unique_dir(dest_parent, &folder)
            .map_err(|e| io_err("cannot create the export folder", e))?;
        let mut created: Vec<PathBuf> = Vec::new();
        for n in &names {
            let target = dest.join(n);
            if let Err(e) = fsutil::copy_no_clobber(&dir.join(n), &target) {
                for f in created {
                    let _ = fs::remove_file(f);
                }
                let _ = fs::remove_dir(&dest);
                return Err(io_err(&format!("cannot export \"{n}\""), e));
            }
            created.push(target);
        }
        Ok(dest)
    }

    /// Copies one tablature to `dest` (the caller's save dialog confirmed an overwrite).
    pub fn export_tablature(&self, id: &str, name: &str, dest: &Path) -> Result<(), String> {
        let (dir, loaded) = self.load(id)?;
        if !loaded.meta.tablatures.iter().any(|t| t == name) {
            return Err(format!("unknown tablature \"{name}\""));
        }
        let src = dir.join(name);
        if !src.is_file() {
            return Err(format!("tablature \"{name}\" is missing on disk"));
        }
        let parent = dest.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        if inside_tracks(&self.root, parent) {
            return Err("cannot export into the repository's tracks folder".into());
        }
        if dest.is_dir() {
            return Err(format!("{} is a folder", dest.display()));
        }
        fsutil::copy_replace(&src, dest).map_err(|e| io_err("cannot export the tablature", e))
    }
}

/// Makes one part of an export folder name safe on every platform.
fn sanitize_component(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t: String = replaced.trim().trim_start_matches('.').chars().take(100).collect();
    t.trim().trim_end_matches('.').trim().to_string()
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
