//! QA acceptance and data-safety tests for gui-tracks-repository (Rust side).
//!
//! The crate is a binary, so the pure modules are compiled into this test crate with `#[path]`.
//! Everything runs on temp repositories; a sibling "outside" folder must never change.
#![allow(dead_code, unused_imports, unused_variables, clippy::type_complexity, clippy::all)]

#[path = "../src/fsutil.rs"]
mod fsutil;
#[path = "../src/track_meta.rs"]
mod track_meta;
#[path = "../src/repository.rs"]
mod repository;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/picker.rs"]
mod picker;
#[path = "../src/import_tmp.rs"]
mod import_tmp;
#[path = "../src/media.rs"]
mod media;
#[path = "../src/download.rs"]
mod download;
#[path = "../src/tools.rs"]
mod tools;
#[path = "../src/import_job.rs"]
mod import_job;
#[path = "../src/ipc.rs"]
mod ipc;

pub const VERSION: &str = env!("CALLIOPE_VERSION");

use repository::{NewTrack, Repository, SaveTrackRequest, TabEntry, TrackRecord};
use std::fs;
use std::path::{Path, PathBuf};
use track_meta::TrackEdits;

fn edits(band: &str, album: &str, title: &str) -> TrackEdits {
    TrackEdits {
        band: band.into(),
        album: album.into(),
        title: title.into(),
        composers: vec!["Ann".into()],
        year: Some(2020),
        source_url: None,
        copyright: None,
    }
}

fn snapshot(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let mut ents: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            let md = fs::symlink_metadata(&p).unwrap();
            if md.file_type().is_symlink() {
                out.push(format!("{rel} -> {:?}", fs::read_link(&p).unwrap()));
            } else if md.is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &p, out);
            } else {
                out.push(format!("{rel} {}", fs::read_to_string(&p).unwrap_or_else(|_| "<bin>".into())));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v
}

struct Env {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    outside: PathBuf,
    sources: PathBuf,
    repo: Repository,
    outside_before: Vec<String>,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().to_path_buf();
        let root = base.join("repo");
        let outside = base.join("outside");
        let sources = base.join("sources");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&sources).unwrap();
        fs::write(outside.join("secret.gp5"), "SECRET").unwrap();
        fs::write(outside.join("audio.mp3"), "AUDIO").unwrap();
        let repo = Repository::new(&root);
        repo.ensure_layout().unwrap();
        let outside_before = snapshot(&outside);
        Env { _tmp: tmp, base, outside, sources, repo, outside_before }
    }
    fn src(&self, name: &str, content: &str) -> PathBuf {
        let p = self.sources.join(name);
        fs::write(&p, content).unwrap();
        p
    }
    fn track(&self, id: &str, tabs: &[&str]) -> TrackRecord {
        let tablatures = tabs.iter().map(|t| (t.to_string(), self.src(t, &format!("content of {t}")))).collect();
        self.repo
            .create_track(
                NewTrack {
                    id: Some(id.into()),
                    edits: edits("Band", "Album", "Title"),
                    audio_name: "backing.mp3".into(),
                    tablatures,
                },
                &self.outside.join("audio.mp3"),
            )
            .unwrap()
    }
    fn dir(&self, id: &str) -> PathBuf {
        self.repo.root.join("tracks").join(id)
    }
    fn json(&self, id: &str) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.dir(id).join("track.json")).unwrap()).unwrap()
    }
    fn req(&self, t: &TrackRecord, tabs: Vec<TabEntry>) -> SaveTrackRequest {
        SaveTrackRequest {
            id: t.id.clone(),
            revision: t.revision.clone(),
            edits: edits(&t.band, &t.album, &t.title),
            tablatures: tabs,
        }
    }
    fn keep_all(&self, t: &TrackRecord) -> Vec<TabEntry> {
        t.tablatures.iter().map(|n| TabEntry::Keep { name: n.clone() }).collect()
    }
    fn assert_outside_unchanged(&self) {
        assert_eq!(snapshot(&self.outside), self.outside_before, "the outside folder changed");
    }
    fn trash_files(&self) -> Vec<String> {
        let t = self.repo.root.join("trash");
        if !t.exists() {
            return vec![];
        }
        snapshot(&t)
    }
}

const ID1: &str = "0199b0a0-0000-7000-8000-000000000001";

// ---------- req 1-6: layout and metadata ----------

#[test]
fn req1_metadata_has_all_fields_and_version() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    let j = e.json(ID1);
    for k in [
        "schema_version", "id", "band", "album", "title", "composers", "year", "source_url", "copyright",
        "type", "audio", "original", "stems", "stem_model", "tablatures", "imported", "modified",
    ] {
        assert!(j.get(k).is_some(), "missing field {k}");
    }
    assert_eq!(j["schema_version"], 2);
    assert_eq!(j["id"], ID1);
    assert_eq!(j["audio"], "backing.mp3");
    assert_eq!(j["tablatures"], serde_json::json!(["a.gp5"]));
}

#[test]
fn req3_4_5_layout_tracks_dir_one_folder_per_track_relative_names() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    e.track("hand-made_2", &[]);
    assert!(e.repo.root.join("tracks").is_dir());
    assert!(e.repo.root.join("calliope-repository.json").is_file());
    assert!(e.dir(ID1).join("backing.mp3").is_file());
    assert!(e.dir(ID1).join("a.gp5").is_file());
    let lib = e.repo.scan();
    assert_eq!(lib.tracks.len(), 2);
    assert!(lib.problems.is_empty());
    for t in lib.tracks {
        assert!(!t.audio.as_deref().unwrap_or_default().contains('/'));
        assert!(t.tablatures.iter().all(|n| !n.contains('/')));
    }
    e.assert_outside_unchanged();
}

#[test]
fn req6_new_ids_are_uuid_like_and_not_name_derived() {
    let a = track_meta::new_id();
    let b = track_meta::new_id();
    assert_ne!(a, b);
    assert!(track_meta::is_valid_id(&a));
    assert_eq!(a.len(), 36);
    assert_eq!(a.as_bytes()[14], b'7');
}

#[test]
fn req2_default_root_function() {
    assert_eq!(
        settings::default_repository_root(Path::new("/home/u/.local/share")),
        PathBuf::from("/home/u/.local/share/calliope")
    );
}

#[test]
fn scan_never_writes() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    fs::create_dir(e.repo.root.join("tracks/bad name")).unwrap();
    fs::write(e.repo.root.join("tracks/loose-file"), "x").unwrap();
    let before = snapshot(&e.repo.root);
    let lib = e.repo.scan();
    assert_eq!(lib.tracks.len(), 1);
    assert_eq!(lib.problems.len(), 1);
    assert_eq!(snapshot(&e.repo.root), before);
}

// ---------- AC10 save ----------

#[test]
fn ac10_save_writes_disk_modified_updated_imported_kept() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    // make timestamps old so "now" is distinguishable
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap()
        .replace(&t.modified, "2020-01-01T00:00:00Z")
        .replace(&t.imported, "2020-01-01T00:00:00Z");
    fs::write(&p, &s).unwrap();
    let t = e.repo.scan().tracks.remove(0);
    let mut r = e.req(&t, vec![]);
    r.edits.album = "New Album".into();
    r.edits.composers = vec!["X".into(), "Y".into()];
    r.edits.year = None;
    r.edits.source_url = Some("https://example.org/x".into());
    let res = e.repo.save_track(r, &|_| None).unwrap();
    let j = e.json(ID1);
    assert_eq!(j["album"], "New Album");
    assert_eq!(j["composers"], serde_json::json!(["X", "Y"]));
    assert!(j["year"].is_null());
    assert_eq!(j["source_url"], "https://example.org/x");
    assert_eq!(j["imported"], "2020-01-01T00:00:00Z");
    let m = j["modified"].as_str().unwrap();
    assert_ne!(m, "2020-01-01T00:00:00Z");
    assert_eq!(m.len(), 20);
    assert!(m.ends_with('Z'));
    assert_eq!(res.track.modified, m);
    assert_ne!(res.track.revision, t.revision);
    assert!(!e.dir(ID1).join("track.json.tmp").exists());
    e.assert_outside_unchanged();
}

#[test]
fn save_without_changes_still_updates_modified() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace(&t.modified, "2020-01-01T00:00:00Z");
    fs::write(&p, s).unwrap();
    let t = e.repo.scan().tracks.remove(0);
    let r = e.req(&t, vec![]);
    let res = e.repo.save_track(r, &|_| None).unwrap();
    assert_ne!(res.track.modified, "2020-01-01T00:00:00Z");
}

// ---------- AC16 / AC19 tablature add and remove ----------

#[test]
fn ac16_add_copies_lists_and_updates_timestamp_source_kept() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let src = e.src("riff.gp5", "RIFF");
    let lookup = {
        let src = src.clone();
        move |tok: &str| (tok == "p1").then(|| src.clone())
    };
    let mut tabs = e.keep_all(&t);
    tabs.push(TabEntry::Add { token: "p1".into() });
    let res = e.repo.save_track(e.req(&t, tabs), &lookup).unwrap();
    assert_eq!(res.track.tablatures, vec!["a.gp5", "riff.gp5"]);
    assert_eq!(fs::read_to_string(e.dir(ID1).join("riff.gp5")).unwrap(), "RIFF");
    assert!(src.exists());
    assert_eq!(e.json(ID1)["tablatures"], serde_json::json!(["a.gp5", "riff.gp5"]));
    assert!(res.warnings.is_empty());
    e.assert_outside_unchanged();
}

#[test]
fn ac19_remove_goes_to_trash_not_deleted_metadata_updated() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5", "b.gp5"]);
    let tabs = vec![TabEntry::Keep { name: "b.gp5".into() }];
    let res = e.repo.save_track(e.req(&t, tabs), &|_| None).unwrap();
    assert_eq!(res.track.tablatures, vec!["b.gp5"]);
    assert!(!e.dir(ID1).join("a.gp5").exists());
    let trash = e.trash_files();
    assert!(trash.iter().any(|l| l.contains("a.gp5") && l.contains("content of a.gp5")), "{trash:?}");
    assert_eq!(e.json(ID1)["tablatures"], serde_json::json!(["b.gp5"]));
}

#[test]
fn ac18_staging_a_remove_does_not_touch_disk_until_save() {
    // The repository only changes in save_track: a scan after "staging" (nothing) is identical.
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    let before = snapshot(&e.repo.root);
    let _ = e.repo.scan();
    assert_eq!(snapshot(&e.repo.root), before);
}

#[test]
fn same_name_replace_old_in_trash_new_in_place() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let newsrc = e.src("a.gp5.new", "x"); // different file name on disk
    let _ = newsrc;
    let dir = e.base.join("pick");
    fs::create_dir_all(&dir).unwrap();
    let pick = dir.join("a.gp5");
    fs::write(&pick, "NEW CONTENT").unwrap();
    let lookup = { let p = pick.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let res = e
        .repo
        .save_track(e.req(&t, vec![TabEntry::Replace { name: "a.gp5".into(), token: "p1".into() }]), &lookup)
        .unwrap();
    assert!(res.warnings.is_empty(), "{:?}", res.warnings);
    assert_eq!(fs::read_to_string(e.dir(ID1).join("a.gp5")).unwrap(), "NEW CONTENT");
    assert!(e.trash_files().iter().any(|l| l.contains("content of a.gp5")));
    assert!(!e.dir(ID1).join(".a.gp5.part").exists());
    assert!(pick.exists());
}

#[test]
fn replace_with_file_picked_from_inside_the_track_folder_itself() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let pick = e.dir(ID1).join("a.gp5");
    let lookup = { let p = pick.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let res = e
        .repo
        .save_track(e.req(&t, vec![TabEntry::Replace { name: "a.gp5".into(), token: "p1".into() }]), &lookup)
        .unwrap();
    assert_eq!(fs::read_to_string(e.dir(ID1).join("a.gp5")).unwrap(), "content of a.gp5", "{:?}", res.warnings);
}

// ---------- data safety: name clashes ----------

#[test]
fn untracked_file_with_same_name_is_never_overwritten() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    fs::write(e.dir(ID1).join("riff.gp5"), "USER FILE").unwrap();
    let src = e.src("riff.gp5", "NEW");
    let lookup = { let p = src.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let before = fs::read(e.dir(ID1).join("track.json")).unwrap();
    let r = e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup);
    assert!(r.is_err());
    assert_eq!(fs::read_to_string(e.dir(ID1).join("riff.gp5")).unwrap(), "USER FILE");
    assert_eq!(fs::read(e.dir(ID1).join("track.json")).unwrap(), before);
}

#[test]
fn untracked_dangling_symlink_with_same_name_is_not_followed() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let victim = e.outside.join("victim.gp5");
    std::os::unix::fs::symlink(&victim, e.dir(ID1).join("riff.gp5")).unwrap();
    let before_outside = snapshot(&e.outside);
    let src = e.src("riff.gp5", "NEW");
    let lookup = { let p = src.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let r = e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup);
    assert!(r.is_err());
    assert!(!victim.exists(), "wrote through a dangling symlink");
    assert_eq!(snapshot(&e.outside), before_outside);
}

#[test]
fn untracked_symlink_to_outside_file_not_overwritten() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    std::os::unix::fs::symlink(e.outside.join("secret.gp5"), e.dir(ID1).join("riff.gp5")).unwrap();
    let src = e.src("riff.gp5", "NEW");
    let lookup = { let p = src.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup).is_err());
    e.assert_outside_unchanged();
}

#[test]
fn case_insensitive_duplicate_names_rejected() {
    let e = Env::new();
    let t = e.track(ID1, &["A.gp5"]);
    let src = e.src("a.gp5", "NEW");
    let lookup = { let p = src.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let mut tabs = e.keep_all(&t);
    tabs.push(TabEntry::Add { token: "p1".into() });
    assert!(e.repo.save_track(e.req(&t, tabs), &lookup).is_err());
    assert_eq!(fs::read_to_string(e.dir(ID1).join("A.gp5")).unwrap(), "content of A.gp5");
}

#[test]
fn adding_audio_name_or_track_json_rejected() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    for name in ["backing.mp3", "track.json"] {
        let d = e.base.join(format!("p-{name}"));
        fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        fs::write(&p, "EVIL").unwrap();
        let lookup = { let p = p.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
        let before = fs::read(e.dir(ID1).join("track.json")).unwrap();
        assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup).is_err(), "{name}");
        assert_eq!(fs::read(e.dir(ID1).join("track.json")).unwrap(), before);
        assert_eq!(fs::read_to_string(e.dir(ID1).join("backing.mp3")).unwrap(), "AUDIO");
    }
}

#[test]
fn stale_part_file_is_not_overwritten_and_save_fails_cleanly() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    fs::write(e.dir(ID1).join(".a.gp5.part"), "STALE").unwrap();
    let pick = e.base.join("a.gp5");
    fs::write(&pick, "NEW").unwrap();
    let lookup = { let p = pick.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let before = fs::read(e.dir(ID1).join("track.json")).unwrap();
    let r = e.repo.save_track(e.req(&t, vec![TabEntry::Replace { name: "a.gp5".into(), token: "p1".into() }]), &lookup);
    assert!(r.is_err());
    assert_eq!(fs::read_to_string(e.dir(ID1).join(".a.gp5.part")).unwrap(), "STALE");
    assert_eq!(fs::read_to_string(e.dir(ID1).join("a.gp5")).unwrap(), "content of a.gp5");
    assert_eq!(fs::read(e.dir(ID1).join("track.json")).unwrap(), before);
}

// ---------- data safety: path traversal ----------

const BAD_IDS: &[&str] = &["../x", "..", ".", "a/b", ".hidden", "", "/etc", "a\\b", "x\0y", "trash/../tracks", "../repo/tracks/x"];

#[test]
fn traversal_ids_rejected_everywhere_nothing_touched() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    fs::create_dir_all(e.repo.root.join("x")).unwrap();
    fs::write(e.repo.root.join("x/track.json"), "{}").unwrap();
    fs::create_dir_all(e.base.join("x")).unwrap();
    let before_repo = snapshot(&e.repo.root);
    let before_base_x = snapshot(&e.base.join("x"));
    for id in BAD_IDS {
        let mut r = e.req(&t, vec![]);
        r.id = id.to_string();
        assert!(e.repo.save_track(r, &|_| None).is_err(), "save {id:?}");
        assert!(e.repo.delete_track(id, &t.revision).is_err(), "delete {id:?}");
        assert!(e.repo.export_track(id, &e.outside).is_err(), "export {id:?}");
        assert!(e.repo.export_tablature(id, "a.gp5", &e.outside.join("o.gp5")).is_err(), "export tab {id:?}");
    }
    assert_eq!(snapshot(&e.repo.root), before_repo);
    assert_eq!(snapshot(&e.base.join("x")), before_base_x);
    e.assert_outside_unchanged();
}

#[test]
fn traversal_tab_names_rejected() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let before = snapshot(&e.repo.root);
    for name in ["../../outside/secret.gp5", "../a.gp5", "/etc/passwd", "sub/a.gp5", "..", ".hidden", "track.json", "a\\b.gp5"] {
        let r = e.repo.save_track(e.req(&t, vec![TabEntry::Keep { name: name.into() }]), &|_| None);
        assert!(r.is_err(), "keep {name}");
        let r = e.repo.save_track(
            e.req(&t, vec![TabEntry::Replace { name: name.into(), token: "p".into() }]),
            &|_| Some(e.outside.join("secret.gp5")),
        );
        assert!(r.is_err(), "replace {name}");
        assert!(e.repo.export_tablature(ID1, name, &e.outside.join("o.gp5")).is_err(), "export {name}");
    }
    assert_eq!(snapshot(&e.repo.root), before);
    e.assert_outside_unchanged();
}

#[test]
fn picked_files_with_unsafe_names_rejected() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let d = e.base.join("p");
    fs::create_dir_all(&d).unwrap();
    for name in [".bashrc", "a:b.gp5", "x?.gp5"] {
        let p = d.join(name);
        fs::write(&p, "x").unwrap();
        let lookup = { let p = p.clone(); move |_: &str| Some(p.clone()) };
        assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup).is_err(), "{name}");
    }
    // a directory token and a missing file
    let lookup = { let p = d.clone(); move |_: &str| Some(p.clone()) };
    assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup).is_err());
    let lookup = { let p = d.join("gone.gp5"); move |_: &str| Some(p.clone()) };
    assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup).is_err());
    let lookup = |_: &str| None;
    assert!(e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "forged".into() }]), &lookup).is_err());
    assert_eq!(e.repo.scan().tracks[0].tablatures.len(), 0);
}

#[test]
fn hand_edited_metadata_with_traversal_names_is_a_problem_not_followed() {
    let e = Env::new();
    e.track(ID1, &[]);
    for (n, bad) in ["../../outside/secret.gp5", "/etc/passwd", "a/b.gp5"].iter().enumerate() {
        let id = format!("hand{n}");
        e.track(&id, &[]);
        let p = e.dir(&id).join("track.json");
        let mut j: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        j["tablatures"] = serde_json::json!([bad]);
        fs::write(&p, serde_json::to_vec(&j).unwrap()).unwrap();
        j["tablatures"] = serde_json::json!([]);
    }
    let lib = e.repo.scan();
    assert_eq!(lib.tracks.len(), 1);
    assert_eq!(lib.problems.len(), 3);
    // and they cannot be saved / exported / deleted either
    for n in 0..3 {
        let id = format!("hand{n}");
        assert!(e.repo.export_track(&id, &e.base.join("outside")).is_err());
        assert!(e.repo.export_tablature(&id, "../../outside/secret.gp5", &e.outside.join("o")).is_err());
    }
    e.assert_outside_unchanged();
}

// ---------- concurrent hand edits (optimistic concurrency) ----------

#[test]
fn hand_edit_after_load_causes_conflict_on_save_and_delete_edit_kept() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("Album", "HandAlbum");
    fs::write(&p, &s).unwrap();
    let r = e.repo.save_track(e.req(&t, e.keep_all(&t)), &|_| None);
    assert!(r.unwrap_err().contains("conflict"));
    assert_eq!(fs::read_to_string(&p).unwrap(), s, "hand edit lost");
    let r = e.repo.delete_track(ID1, &t.revision);
    assert!(r.unwrap_err().contains("conflict"));
    assert!(e.dir(ID1).is_dir());
    assert!(e.trash_files().is_empty());
}

#[test]
fn same_length_hand_edit_is_detected() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"Album\"", "\"Albun\"");
    fs::write(&p, &s).unwrap();
    assert!(e.repo.save_track(e.req(&t, vec![]), &|_| None).is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), s);
}

#[test]
fn double_save_with_stale_revision_second_fails() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let mut r = e.req(&t, vec![]);
    r.edits.album = "First".into();
    e.repo.save_track(r.clone(), &|_| None).unwrap();
    r.edits.album = "Second".into();
    assert!(e.repo.save_track(r, &|_| None).is_err());
    assert_eq!(e.json(ID1)["album"], "First");
}

#[test]
fn track_removed_by_hand_while_editing_gives_error() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    fs::remove_dir_all(e.dir(ID1)).unwrap();
    assert!(e.repo.save_track(e.req(&t, vec![]), &|_| None).is_err());
    assert!(!e.dir(ID1).exists(), "save recreated the folder");
    assert!(e.repo.delete_track(ID1, &t.revision).is_err());
}

#[test]
fn hand_edit_that_adds_a_tab_while_editing_conflicts_not_trashed() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    // user adds a new tab by hand + metadata
    fs::write(e.dir(ID1).join("hand.gp5"), "H").unwrap();
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"a.gp5\"", "\"a.gp5\", \"hand.gp5\"");
    fs::write(&p, s).unwrap();
    assert!(e.repo.save_track(e.req(&t, e.keep_all(&t)), &|_| None).is_err());
    assert!(e.dir(ID1).join("hand.gp5").exists());
}

// ---------- validation: nothing written on bad edits ----------

#[test]
fn invalid_edits_write_nothing() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let before = snapshot(&e.repo.root);
    let mut cases: Vec<Box<dyn Fn(&mut TrackEdits)>> = vec![
        Box::new(|x| x.title = "   ".into()),
        Box::new(|x| x.year = Some(0)),
        Box::new(|x| x.year = Some(10000)),
        Box::new(|x| x.year = Some(-5)),
        Box::new(|x| x.composers = (0..21).map(|i| format!("c{i}")).collect()),
        Box::new(|x| x.composers = vec!["".into()]),
        Box::new(|x| x.band = "a\u{0}b".into()),
        Box::new(|x| x.album = "x".repeat(201)),
        Box::new(|x| x.title = "x\ny".into()),
        Box::new(|x| x.source_url = Some("u".repeat(2001))),
        Box::new(|x| x.copyright = Some("c".repeat(501))),
    ];
    for c in cases.iter_mut() {
        let mut r = e.req(&t, vec![]);
        c(&mut r.edits);
        assert!(e.repo.save_track(r, &|_| None).is_err());
    }
    assert_eq!(snapshot(&e.repo.root), before);
}

#[test]
fn edits_are_trimmed_and_empty_optionals_become_null() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let mut r = e.req(&t, vec![]);
    r.edits.band = "  Padded  ".into();
    r.edits.copyright = Some("   ".into());
    e.repo.save_track(r, &|_| None).unwrap();
    let j = e.json(ID1);
    assert_eq!(j["band"], "Padded");
    assert!(j["copyright"].is_null());
}

// ---------- newer schema / forward compatibility ----------

#[test]
fn newer_schema_track_is_a_problem_and_never_rewritten_or_deleted() {
    let e = Env::new();
    e.track(ID1, &[]);
    let p = e.dir(ID1).join("track.json");
    let mut j: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    j["schema_version"] = 3.into();
    j["future_field"] = serde_json::json!({"a": [1,2,3]});
    let bytes = serde_json::to_vec_pretty(&j).unwrap();
    fs::write(&p, &bytes).unwrap();
    let lib = e.repo.scan();
    assert!(lib.tracks.is_empty());
    assert_eq!(lib.problems.len(), 1);
    assert!(lib.problems[0].message.contains("newer"), "{}", lib.problems[0].message);
    let fake = TrackRecord {
        id: ID1.into(), band: "".into(), album: "".into(), title: "T".into(), composers: vec![], year: None,
        source_url: None, copyright: None, audio: Some("backing.mp3".into()), tablatures: vec![],
        track_type: track_meta::TrackType::Backing, original: None, stems: vec![], stem_model: None, imported: "".into(),
        modified: "".into(), revision: fsutil::fnv1a64_hex(&bytes), missing: vec![],
    };
    assert!(e.repo.save_track(e.req(&fake, vec![]), &|_| None).is_err());
    assert!(e.repo.delete_track(ID1, &fake.revision).is_err());
    assert!(e.repo.export_track(ID1, &e.outside).is_err());
    assert_eq!(fs::read(&p).unwrap(), bytes);
    assert!(e.trash_files().is_empty());
    e.assert_outside_unchanged();
}

#[test]
fn wrong_type_field_track_is_a_problem_and_untouched() {
    let e = Env::new();
    e.track(ID1, &[]);
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("2020", "\"2020\"");
    fs::write(&p, &s).unwrap();
    let lib = e.repo.scan();
    assert!(lib.tracks.is_empty());
    assert_eq!(lib.problems.len(), 1);
    assert_eq!(fs::read_to_string(&p).unwrap(), s);
}

#[test]
fn unknown_fields_survive_a_save() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let p = e.dir(ID1).join("track.json");
    let mut j: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    j["future_stems"] = serde_json::json!({"vocals": "stems/v.mp3"});
    j["x_note"] = "keep me".into();
    fs::write(&p, serde_json::to_vec(&j).unwrap()).unwrap();
    let t2 = e.repo.scan().tracks.remove(0);
    assert_ne!(t.revision, t2.revision);
    e.repo.save_track(e.req(&t2, vec![]), &|_| None).unwrap();
    let j = e.json(ID1);
    assert_eq!(j["x_note"], "keep me");
    assert_eq!(j["future_stems"]["vocals"], "stems/v.mp3");
}

#[test]
fn newer_repository_marker_refused_and_not_rewritten() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("calliope-repository.json"), "{\"schema_version\": 9, \"x\": 1}").unwrap();
    let repo = Repository::new(&root);
    assert_eq!(repository::status(&root), repository::RepoStatus::Newer);
    assert!(repo.ensure_layout().is_err());
    assert_eq!(fs::read_to_string(root.join("calliope-repository.json")).unwrap(), "{\"schema_version\": 9, \"x\": 1}");
    assert!(!root.join("tracks").exists());
}

#[test]
fn ensure_layout_in_foreign_folder_adds_only_marker_and_tracks() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    fs::create_dir_all(root.join("Music")).unwrap();
    fs::write(root.join("Music/song.mp3"), "m").unwrap();
    fs::write(root.join("notes.txt"), "n").unwrap();
    assert_eq!(repository::status(&root), repository::RepoStatus::Other);
    Repository::new(&root).ensure_layout().unwrap();
    let mut names: Vec<_> = fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, vec!["Music", "calliope-repository.json", "notes.txt", "tracks"]);
    assert_eq!(fs::read_to_string(root.join("notes.txt")).unwrap(), "n");
    // idempotent, and the marker is not rewritten
    Repository::new(&root).ensure_layout().unwrap();
}

// ---------- delete ----------

#[test]
fn delete_moves_whole_folder_to_trash_nothing_removed() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    fs::write(e.dir(ID1).join("unlisted-user-file.txt"), "mine").unwrap();
    e.repo.delete_track(ID1, &t.revision).unwrap();
    assert!(!e.dir(ID1).exists());
    let trash = e.trash_files();
    assert!(trash.iter().any(|l| l.contains("unlisted-user-file.txt mine")), "{trash:?}");
    assert!(trash.iter().any(|l| l.contains("track.json")));
    assert!(trash.iter().any(|l| l.contains("a.gp5")));
    assert!(e.repo.scan().tracks.is_empty());
}

#[test]
fn delete_same_id_twice_in_same_second_keeps_both_in_trash() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    e.repo.delete_track(ID1, &t.revision).unwrap();
    let t = e.track(ID1, &[]);
    e.repo.delete_track(ID1, &t.revision).unwrap();
    let n = fs::read_dir(e.repo.root.join("trash")).unwrap().count();
    assert_eq!(n, 2);
}

#[test]
fn delete_with_stale_revision_refused() {
    let e = Env::new();
    e.track(ID1, &[]);
    assert!(e.repo.delete_track(ID1, "0000000000000000").is_err());
    assert!(e.dir(ID1).is_dir());
}

#[test]
fn stale_tab_trash_clash_names_get_suffix_not_overwritten() {
    let e = Env::new();
    // remove a.gp5 twice in quick succession (re-add by hand in between)
    let t = e.track(ID1, &["a.gp5", "b.gp5"]);
    let t = e.repo.save_track(e.req(&t, vec![TabEntry::Keep { name: "b.gp5".into() }]), &|_| None).unwrap().track;
    fs::write(e.dir(ID1).join("a.gp5"), "second a").unwrap();
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"b.gp5\"", "\"a.gp5\", \"b.gp5\"");
    fs::write(&p, s).unwrap();
    let t = e.repo.scan().tracks.remove(0);
    let _ = t.revision;
    e.repo.save_track(e.req(&t, vec![TabEntry::Keep { name: "b.gp5".into() }]), &|_| None).unwrap();
    let all = e.trash_files().join("\n");
    assert!(all.contains("content of a.gp5") && all.contains("second a"), "{all}");
}

// ---------- symlinks ----------

#[test]
fn symlinked_track_folder_is_not_operated_on() {
    let e = Env::new();
    let real = e.base.join("outside/realtrack");
    fs::create_dir_all(&real).unwrap();
    let t = e.track("tmpid", &["a.gp5"]);
    // move the real track out and symlink it in
    fs::rename(e.dir("tmpid"), &real).unwrap();
    std::os::unix::fs::symlink(&real, e.dir("linked")).unwrap();
    let before = snapshot(&e.outside);
    let mut r = e.req(&t, vec![]);
    r.id = "linked".into();
    assert!(e.repo.save_track(r, &|_| None).is_err());
    assert!(e.repo.delete_track("linked", &t.revision).is_err());
    assert!(e.repo.export_track("linked", &e.base).is_err());
    assert_eq!(snapshot(&e.outside), before);
    assert!(e.repo.scan().tracks.is_empty());
}

#[test]
fn removing_a_tab_that_is_a_symlink_to_an_outside_file_leaves_outside_file_alone() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    std::os::unix::fs::symlink(e.outside.join("secret.gp5"), e.dir(ID1).join("link.gp5")).unwrap();
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"tablatures\": []", "\"tablatures\": [\"link.gp5\"]");
    fs::write(&p, s).unwrap();
    let t2 = e.repo.scan().tracks.remove(0);
    assert_eq!(t2.tablatures, vec!["link.gp5"]);
    let _ = t;
    let res = e.repo.save_track(e.req(&t2, vec![]), &|_| None);
    let _ = res;
    e.assert_outside_unchanged();
    assert_eq!(fs::read_to_string(e.outside.join("secret.gp5")).unwrap(), "SECRET");
}

#[test]
fn removing_a_tab_symlinked_to_another_tracks_file_does_not_trash_that_other_file() {
    let e = Env::new();
    let _a = e.track(ID1, &["a.gp5"]);
    let _b = e.track("second", &[]);
    std::os::unix::fs::symlink(e.dir(ID1).join("a.gp5"), e.dir("second").join("link.gp5")).unwrap();
    let p = e.dir("second").join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"tablatures\": []", "\"tablatures\": [\"link.gp5\"]");
    fs::write(&p, s).unwrap();
    let lib = e.repo.scan();
    let b = lib.tracks.iter().find(|t| t.id == "second").unwrap().clone();
    let _ = e.repo.save_track(e.req(&b, vec![]), &|_| None);
    assert!(
        e.dir(ID1).join("a.gp5").exists(),
        "removing a symlinked tab from track 'second' moved track 1's real file into the trash"
    );
}

#[test]
fn trash_symlinked_outside_does_not_receive_deleted_track() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let out = e.base.join("outside/trashlink");
    fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink(&out, e.repo.root.join("trash")).unwrap();
    let before_exists = e.dir(ID1).exists();
    assert!(before_exists);
    let r = e.repo.delete_track(ID1, &t.revision);
    // Either refused, or (if allowed) the data must still exist somewhere reachable.
    if r.is_ok() {
        let moved = fs::read_dir(&out).unwrap().count();
        panic!("trash is a symlink to outside; delete moved the track outside the repository ({moved} entries)");
    }
}

#[test]
fn track_json_symlink_is_not_written_through() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let target = e.outside.join("meta-copy.json");
    fs::rename(e.dir(ID1).join("track.json"), &target).unwrap();
    std::os::unix::fs::symlink(&target, e.dir(ID1).join("track.json")).unwrap();
    let before = fs::read(&target).unwrap();
    let mut r = e.req(&t, vec![]);
    r.edits.album = "Changed".into();
    let _ = e.repo.save_track(r, &|_| None);
    assert_eq!(fs::read(&target).unwrap(), before, "wrote through the symlink to an outside file");
}

#[test]
fn export_does_not_follow_symlinked_destination_into_tracks() {
    let e = Env::new();
    e.track(ID1, &[]);
    let link = e.base.join("link-to-tracks");
    std::os::unix::fs::symlink(e.repo.root.join("tracks"), &link).unwrap();
    assert!(e.repo.export_track(ID1, &link).is_err());
    assert!(e.repo.export_track(ID1, &e.repo.root.join("tracks")).is_err());
    assert!(e.repo.export_track(ID1, &e.dir(ID1)).is_err());
    assert_eq!(e.repo.scan().tracks.len(), 1);
    assert!(e.repo.scan().problems.is_empty());
}

// ---------- read-only folders ----------

#[test]
fn read_only_track_folder_save_fails_nothing_left_behind() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let src = e.src("riff.gp5", "RIFF");
    let lookup = { let p = src.clone(); move |t: &str| (t == "p1").then(|| p.clone()) };
    let dir = e.dir(ID1);
    let before = snapshot(&dir);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let r = e.repo.save_track(e.req(&t, vec![TabEntry::Add { token: "p1".into() }]), &lookup);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.is_err());
    assert_eq!(snapshot(&dir), before);
}

#[test]
fn read_only_tracks_dir_delete_fails_nothing_moved() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    let tracks = e.repo.root.join("tracks");
    fs::set_permissions(&tracks, fs::Permissions::from_mode(0o555)).unwrap();
    let r = e.repo.delete_track(ID1, &t.revision);
    fs::set_permissions(&tracks, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.is_err());
    assert!(e.dir(ID1).join("a.gp5").exists());
}

#[test]
fn read_only_root_remove_tab_keeps_file_on_disk_and_reports_warning() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    fs::set_permissions(&e.repo.root, fs::Permissions::from_mode(0o555)).unwrap();
    let r = e.repo.save_track(e.req(&t, vec![]), &|_| None);
    fs::set_permissions(&e.repo.root, fs::Permissions::from_mode(0o755)).unwrap();
    let res = r.unwrap();
    assert!(!res.warnings.is_empty(), "expected a trash warning");
    assert!(e.dir(ID1).join("a.gp5").exists(), "the removed tab content must not be lost");
    assert_eq!(fs::read_to_string(e.dir(ID1).join("a.gp5")).unwrap(), "content of a.gp5");
}

#[test]
fn rollback_when_second_copy_fails_removes_first_copy_only() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let t = e.track(ID1, &["keep.gp5"]);
    let ok = e.src("one.gp5", "ONE");
    let unreadable = e.src("two.gp5", "TWO");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    let (o, u) = (ok.clone(), unreadable.clone());
    let lookup = move |t: &str| match t { "p1" => Some(o.clone()), "p2" => Some(u.clone()), _ => None };
    let before = snapshot(&e.dir(ID1));
    let mut tabs = e.keep_all(&t);
    tabs.push(TabEntry::Add { token: "p1".into() });
    tabs.push(TabEntry::Add { token: "p2".into() });
    let r = e.repo.save_track(e.req(&t, tabs), &lookup);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(r.is_err());
    assert_eq!(snapshot(&e.dir(ID1)), before);
}

// ---------- export ----------

#[test]
fn export_track_creates_named_folder_and_numbered_duplicates() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    let dest = e.base.join("exports");
    fs::create_dir_all(&dest).unwrap();
    let p1 = e.repo.export_track(ID1, &dest).unwrap();
    let p2 = e.repo.export_track(ID1, &dest).unwrap();
    assert_eq!(p1.file_name().unwrap(), "Band - Album - Title");
    assert_eq!(p2.file_name().unwrap(), "Band - Album - Title (2)");
    for p in [&p1, &p2] {
        for f in ["track.json", "backing.mp3", "a.gp5"] {
            assert!(p.join(f).is_file(), "{f}");
        }
    }
    // source untouched
    assert!(e.dir(ID1).join("a.gp5").is_file());
}

#[test]
fn export_name_with_separators_stays_inside_destination() {
    let e = Env::new();
    let id = "weird";
    e.repo
        .create_track(
            NewTrack {
                id: Some(id.into()),
                edits: edits("../../etc", "a/b\\c", "..\\x:y*z?"),
                audio_name: "backing.mp3".into(),
                tablatures: vec![],
            },
            &e.outside.join("audio.mp3"),
        )
        .unwrap();
    let dest = e.base.join("exports");
    fs::create_dir_all(&dest).unwrap();
    let p = e.repo.export_track(id, &dest).unwrap();
    assert_eq!(p.parent().unwrap(), dest);
    e.assert_outside_unchanged();
}

#[test]
fn export_with_missing_file_fails_before_copying_anything() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    fs::remove_file(e.dir(ID1).join("a.gp5")).unwrap();
    let dest = e.base.join("exports");
    fs::create_dir_all(&dest).unwrap();
    assert!(e.repo.export_track(ID1, &dest).is_err());
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
}

#[test]
fn export_into_file_or_missing_destination_fails() {
    let e = Env::new();
    e.track(ID1, &[]);
    assert!(e.repo.export_track(ID1, &e.outside.join("audio.mp3")).is_err());
    assert!(e.repo.export_track(ID1, &e.base.join("nonexistent")).is_err());
    assert!(!e.base.join("nonexistent").exists());
}

#[test]
fn export_track_with_maximal_valid_metadata_names() {
    // band, album and title may each be 200 chars; the export folder name must still be creatable.
    let e = Env::new();
    let id = "longnames";
    e.repo
        .create_track(
            NewTrack {
                id: Some(id.into()),
                edits: edits(&"b".repeat(200), &"a".repeat(200), &"t".repeat(200)),
                audio_name: "backing.mp3".into(),
                tablatures: vec![],
            },
            &e.outside.join("audio.mp3"),
        )
        .unwrap();
    let dest = e.base.join("exports");
    fs::create_dir_all(&dest).unwrap();
    let r = e.repo.export_track(id, &dest);
    assert!(r.is_ok(), "export of a valid track failed: {:?}", r.err());
}

#[test]
fn export_track_with_long_multibyte_names() {
    let e = Env::new();
    let id = "longmb";
    e.repo
        .create_track(
            NewTrack {
                id: Some(id.into()),
                edits: edits(&"é".repeat(100), "", &"日".repeat(100)),
                audio_name: "backing.mp3".into(),
                tablatures: vec![],
            },
            &e.outside.join("audio.mp3"),
        )
        .unwrap();
    let dest = e.base.join("exports");
    fs::create_dir_all(&dest).unwrap();
    let r = e.repo.export_track(id, &dest);
    assert!(r.is_ok(), "export failed: {:?}", r.err());
}

#[test]
fn export_tablature_inside_tracks_refused_and_overwrite_only_target() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    assert!(e.repo.export_tablature(ID1, "a.gp5", &e.dir(ID1).join("copy.gp5")).is_err());
    assert!(e.repo.export_tablature(ID1, "a.gp5", &e.dir(ID1).join("a.gp5")).is_err());
    assert!(e.repo.export_tablature(ID1, "nope.gp5", &e.outside.join("x.gp5")).is_err());
    assert!(e.repo.export_tablature(ID1, "a.gp5", &e.outside).is_err());
    let out = e.outside.join("exported.gp5");
    e.repo.export_tablature(ID1, "a.gp5", &out).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "content of a.gp5");
    assert_eq!(fs::read_to_string(e.dir(ID1).join("a.gp5")).unwrap(), "content of a.gp5");
}

// ---------- scan: missing / hand-made ----------

#[test]
fn missing_files_listed_and_scan_survives_junk() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    fs::remove_file(e.dir(ID1).join("a.gp5")).unwrap();
    fs::remove_file(e.dir(ID1).join("backing.mp3")).unwrap();
    fs::create_dir_all(e.repo.root.join("tracks/nojson")).unwrap();
    fs::create_dir_all(e.repo.root.join("tracks/badjson")).unwrap();
    fs::write(e.repo.root.join("tracks/badjson/track.json"), "{not json").unwrap();
    fs::create_dir_all(e.repo.root.join("tracks/.hidden")).unwrap();
    fs::create_dir_all(e.repo.root.join("tracks/x.tmp")).unwrap();
    fs::create_dir_all(e.repo.root.join("tracks/mismatch")).unwrap();
    fs::write(e.repo.root.join("tracks/mismatch/track.json"),
        fs::read(e.dir(ID1).join("track.json")).unwrap()).unwrap();
    let lib = e.repo.scan();
    assert_eq!(lib.tracks.len(), 1);
    let mut missing = lib.tracks[0].missing.clone();
    missing.sort();
    assert_eq!(missing, vec!["a.gp5", "backing.mp3"]);
    let mut dirs: Vec<_> = lib.problems.iter().map(|p| p.dir.clone()).collect();
    dirs.sort();
    assert_eq!(dirs, vec!["badjson", "mismatch", "nojson"]);
}

// ---------- IPC layer: tokens ----------

mod ipc_layer {
    use super::*;
    use ipc::*;
    use picker::{DialogKind, FileReq, FolderReq, Picker, SaveReq};
    use std::sync::Mutex;

    struct Fake {
        files: Mutex<Vec<Option<PathBuf>>>,
        folders: Mutex<Vec<Option<PathBuf>>>,
    }
    impl Picker for Fake {
        fn pick_file(&self, _: &FileReq) -> Option<PathBuf> { self.files.lock().unwrap().remove(0) }
        fn pick_folder(&self, _: &FolderReq) -> Option<PathBuf> { self.folders.lock().unwrap().remove(0) }
        fn save_file(&self, _: &SaveReq) -> Option<PathBuf> { None }
    }

    fn import_state(st: &RepoState) -> import_job::ImportState {
        import_job::ImportState::new(tools::Tools::default(), st.repo_lock(), std::time::Duration::from_millis(10))
    }

    fn state(root: &Path, files: Vec<Option<PathBuf>>, folders: Vec<Option<PathBuf>>) -> RepoState {
        RepoState::new(root.to_path_buf(), Box::new(Fake { files: Mutex::new(files), folders: Mutex::new(folders) }))
    }

    #[test]
    fn root_token_cannot_be_used_as_a_tablature_token() {
        let e = Env::new();
        let t = e.track(ID1, &[]);
        let folder = e.base.join("somefolder");
        fs::create_dir_all(&folder).unwrap();
        let st = state(&e.repo.root, vec![], vec![Some(folder)]);
        let picked = do_choose_root(&st, None).unwrap();
        let req = SaveTrackRequest {
            id: t.id.clone(), revision: t.revision.clone(),
            edits: edits("Band", "Album", "Title"),
            tablatures: vec![TabEntry::Add { token: picked.token.clone() }],
        };
        assert!(do_save_track(&st, req).is_err());
    }

    #[test]
    fn tablature_token_cannot_set_the_root_and_forged_tokens_fail() {
        let e = Env::new();
        let f = e.base.join("riff.gp5");
        fs::write(&f, "x").unwrap();
        let st = state(&e.repo.root, vec![Some(f)], vec![]);
        let store = settings::SettingsStore::open(e.base.join("settings.json"));
        let picked = do_pick_tablature(&st, PickPurpose::Add, None).unwrap().unwrap();
        assert_eq!(picked.name, "riff.gp5");
        let default = e.base.join("default");
        assert!(do_set_root(&st, &store, &import_state(&st), &default, &picked.token).is_err());
        assert!(do_set_root(&st, &store, &import_state(&st), &default, "p999").is_err());
        assert!(do_set_root(&st, &store, &import_state(&st), &default, "../../etc").is_err());
        assert_eq!(store.get().repository_root, None);
    }

    #[test]
    fn non_tablature_extension_and_hidden_names_are_refused_at_pick() {
        let e = Env::new();
        let d = e.base.join("p");
        fs::create_dir_all(&d).unwrap();
        for n in ["notes.txt", ".hidden.gp5", "noext", "x.mp3"] {
            let f = d.join(n);
            fs::write(&f, "x").unwrap();
            let st = state(&e.repo.root, vec![Some(f)], vec![]);
            assert!(do_pick_tablature(&st, PickPurpose::Add, None).is_err(), "{n}");
        }
    }

    #[test]
    fn token_is_single_use_after_save() {
        let e = Env::new();
        let t = e.track(ID1, &[]);
        let f = e.base.join("riff.gp5");
        fs::write(&f, "x").unwrap();
        let st = state(&e.repo.root, vec![Some(f)], vec![]);
        let picked = do_pick_tablature(&st, PickPurpose::Add, None).unwrap().unwrap();
        let mk = |rev: &str| SaveTrackRequest {
            id: t.id.clone(), revision: rev.into(), edits: edits("Band", "Album", "Title"),
            tablatures: vec![TabEntry::Add { token: picked.token.clone() }],
        };
        let r = do_save_track(&st, mk(&t.revision)).unwrap();
        // replay with the fresh revision: must not re-add (token consumed; also name would clash)
        assert!(do_save_track(&st, mk(&r.track.revision)).is_err());
    }

    #[test]
    fn newer_root_refused_set_root_makes_no_changes_in_foreign_folder_beyond_layout() {
        let e = Env::new();
        let newer = e.base.join("newer");
        fs::create_dir_all(&newer).unwrap();
        fs::write(newer.join("calliope-repository.json"), "{\"schema_version\":5}").unwrap();
        let st = state(&e.repo.root, vec![], vec![Some(newer.clone())]);
        let store = settings::SettingsStore::open(e.base.join("settings.json"));
        let picked = do_choose_root(&st, None).unwrap();
        assert_eq!(picked.status, repository::RepoStatus::Newer);
        assert!(do_set_root(&st, &store, &import_state(&st), &e.base.join("d"), &picked.token).is_err());
        assert!(!newer.join("tracks").exists());
        assert_eq!(store.get().repository_root, None);
    }

    #[test]
    fn configured_missing_root_is_not_created_by_listing() {
        let e = Env::new();
        let missing = e.base.join("usb/lib");
        let st = state(&missing, vec![], vec![]);
        let lib = do_list_tracks(&st, &e.base.join("default")).unwrap();
        assert!(lib.tracks.is_empty());
        assert!(!missing.exists());
    }
}

// ---------- fix round 1 re-probes ----------

#[test]
fn crash_left_unique_tmp_files_do_not_break_scan_or_save() {
    let e = Env::new();
    let t = e.track(ID1, &["a.gp5"]);
    // crash leftovers: inside a track folder, and as stray entries in tracks/
    fs::write(e.dir(ID1).join(".track.json.4242.0.tmp"), "{ half written").unwrap();
    fs::write(e.dir(ID1).join(".a.gp5.4242.1.tmp"), "junk").unwrap();
    fs::write(e.repo.root.join("tracks").join(".track.json.4242.2.tmp"), "junk").unwrap();
    fs::create_dir_all(e.repo.root.join("tracks").join(".track.json.4242.3.tmp")).unwrap();
    let lib = e.repo.scan();
    assert_eq!(lib.tracks.len(), 1, "{:?}", lib.problems);
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(lib.tracks[0].tablatures, vec!["a.gp5"]);
    // a save still works, and leaves the foreign leftovers alone
    let t2 = lib.tracks[0].clone();
    let _ = t;
    e.repo.save_track(e.req(&t2, e.keep_all(&t2)), &|_| None).unwrap();
    assert_eq!(fs::read_to_string(e.dir(ID1).join(".track.json.4242.0.tmp")).unwrap(), "{ half written");
    assert_eq!(e.repo.scan().tracks.len(), 1);
    // delete moves the folder, leftovers included, nothing lost
    let t3 = e.repo.scan().tracks.remove(0);
    e.repo.delete_track(&t3.id, &t3.revision).unwrap();
    assert!(e.repo.scan().tracks.is_empty());
}

#[test]
fn tmp_named_track_folder_is_ignored_not_a_problem() {
    let e = Env::new();
    fs::create_dir_all(e.repo.root.join("tracks").join(format!("{ID1}.tmp"))).unwrap();
    let lib = e.repo.scan();
    assert!(lib.tracks.is_empty() && lib.problems.is_empty(), "{:?}", lib.problems);
}

#[test]
fn export_into_root_trash_and_tracks_refused_nothing_written() {
    let e = Env::new();
    e.track(ID1, &["a.gp5"]);
    let t = e.repo.scan().tracks.remove(0);
    e.repo.delete_track(&t.id, &t.revision).unwrap();
    e.track(ID1, &["a.gp5"]);
    let root = e.repo.root.clone();
    fs::create_dir_all(root.join("trash")).unwrap();
    let nested = root.join("tracks").join(ID1);
    let before = snapshot(&root);
    for d in [root.clone(), root.join("trash"), root.join("tracks"), nested.clone(), root.join("tracks").join("..")] {
        assert!(e.repo.export_track(ID1, &d).is_err(), "export into {} allowed", d.display());
    }
    // via a symlink pointing into the root
    let link = e.base.join("lnk");
    std::os::unix::fs::symlink(root.join("trash"), &link).unwrap();
    assert!(e.repo.export_track(ID1, &link).is_err(), "export through symlink into trash allowed");
    assert_eq!(snapshot(&root), before);
}

#[test]
fn replacing_a_symlinked_tab_does_not_write_through_the_link() {
    let e = Env::new();
    let t = e.track(ID1, &[]);
    let _ = t;
    std::os::unix::fs::symlink(e.outside.join("secret.gp5"), e.dir(ID1).join("link.gp5")).unwrap();
    let p = e.dir(ID1).join("track.json");
    let s = fs::read_to_string(&p).unwrap().replace("\"tablatures\": []", "\"tablatures\": [\"link.gp5\"]");
    fs::write(&p, s).unwrap();
    let t2 = e.repo.scan().tracks.remove(0);
    let newsrc = e.src("new.gp5", "NEW CONTENT");
    let tok = newsrc.to_string_lossy().into_owned();
    let res = e.repo.save_track(
        e.req(&t2, vec![TabEntry::Replace { name: "link.gp5".into(), token: tok.clone() }]),
        &|t| if t == tok { Some(newsrc.clone()) } else { None },
    );
    let _ = res;
    assert_eq!(fs::read_to_string(e.outside.join("secret.gp5")).unwrap(), "SECRET", "wrote through symlink");
    e.assert_outside_unchanged();
}
