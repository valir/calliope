use super::*;
use std::collections::BTreeMap;
use tempfile::TempDir;

struct Env {
    _tmp: TempDir,
    outside: PathBuf,
    sources: PathBuf,
    repo: Repository,
    before: Vec<String>,
}

fn list(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let mut ents: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &p, out);
            } else {
                out.push(format!("{rel} {}", fs::metadata(&p).unwrap().len()));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let outside = tmp.path().join("outside");
        let sources = tmp.path().join("sources");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(&sources).unwrap();
        fs::write(outside.join("tab-new.gp5"), b"NEW TAB").unwrap();
        fs::write(outside.join("other.gp"), b"OTHER").unwrap();
        fs::write(outside.join("audio.mp3"), b"AUDIO").unwrap();
        let repo = Repository::new(&root);
        repo.ensure_layout().unwrap();
        let before = list(&outside);
        Env { _tmp: tmp, outside, sources, repo, before }
    }

    fn track(&self, id: &str, tabs: &[&str]) -> TrackRecord {
        let tablatures = tabs
            .iter()
            .map(|t| {
                let p = self.sources.join(t);
                fs::write(&p, format!("content of {t}")).unwrap();
                (t.to_string(), p)
            })
            .collect();
        self.repo
            .create_track(
                NewTrack { id: Some(id.into()), edits: edits("Band", "Album", "Title"), audio_name: "backing.mp3".into(), tablatures },
                &self.outside.join("audio.mp3"),
            )
            .unwrap()
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.repo.root.join("tracks").join(id)
    }

    fn lookup(&self) -> impl Fn(&str) -> Option<PathBuf> + '_ {
        |t| match t {
            "new" => Some(self.outside.join("tab-new.gp5")),
            "other" => Some(self.outside.join("other.gp")),
            "missing" => Some(self.outside.join("nope.gp5")),
            "bad" => Some(PathBuf::from(".hidden")),
            _ => None,
        }
    }

    fn trash_files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        let trash = self.repo.root.join("trash");
        if trash.exists() {
            fn walk(base: &Path, d: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
                for e in fs::read_dir(d).unwrap().flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(base, &p, m);
                    } else {
                        m.insert(p.strip_prefix(base).unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap());
                    }
                }
            }
            walk(&trash, &trash, &mut m);
        }
        m
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(list(&self.outside), self.before, "the outside folder changed");
        }
    }
}

fn edits(band: &str, album: &str, title: &str) -> TrackEdits {
    TrackEdits {
        band: band.into(),
        album: album.into(),
        title: title.into(),
        composers: vec!["C One".into()],
        year: Some(2020),
        source_url: None,
        copyright: None,
    }
}

fn req(rec: &TrackRecord, tabs: Vec<TabEntry>) -> SaveTrackRequest {
    SaveTrackRequest {
        id: rec.id.clone(),
        revision: rec.revision.clone(),
        edits: edits(&rec.band, &rec.album, &rec.title),
        tablatures: tabs,
    }
}

fn keep(n: &str) -> TabEntry {
    TabEntry::Keep { name: n.into() }
}

fn tree(env: &Env) -> Vec<String> {
    list(&env.repo.root)
}

#[test]
fn status_values() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().join("r");
    assert_eq!(status(&r), RepoStatus::Missing);
    fs::create_dir(&r).unwrap();
    assert_eq!(status(&r), RepoStatus::Empty);
    fs::write(r.join("x.txt"), "x").unwrap();
    assert_eq!(status(&r), RepoStatus::Other);
    fs::write(r.join(MARKER), "{\"schema_version\": 1}").unwrap();
    assert_eq!(status(&r), RepoStatus::Ok);
    fs::write(r.join(MARKER), "{\"schema_version\": 2}").unwrap();
    assert_eq!(status(&r), RepoStatus::Newer);
    assert!(Repository::new(&r).ensure_layout().is_err());
}

#[test]
fn ensure_layout_in_foreign_folder_adds_only_marker_and_tracks() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().join("foreign");
    fs::create_dir_all(r.join("docs")).unwrap();
    fs::write(r.join("docs/a.txt"), "keep").unwrap();
    fs::write(r.join("b.txt"), "keep").unwrap();
    Repository::new(&r).ensure_layout().unwrap();
    Repository::new(&r).ensure_layout().unwrap();
    let mut names: Vec<_> = fs::read_dir(&r).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["b.txt", MARKER, "docs", "tracks"]);
    assert_eq!(fs::read_to_string(r.join("docs/a.txt")).unwrap(), "keep");
    assert_eq!(status(&r), RepoStatus::Ok);
}

#[test]
fn create_and_scan() {
    let env = Env::new();
    let a = env.track("aaa", &["one.gp5"]);
    env.track("bbb", &[]);
    let lib = env.repo.scan();
    assert_eq!(lib.tracks.len(), 2);
    assert!(lib.problems.is_empty());
    assert_eq!(lib.tracks[0], a);
    assert_eq!(a.revision.len(), 16);
    assert!(a.missing.is_empty());
    assert_eq!(fs::read(env.dir("aaa").join("one.gp5")).unwrap(), b"content of one.gp5");
}

#[test]
fn create_refuses_existing_id_and_cleans_up_on_failure() {
    let env = Env::new();
    env.track("aaa", &[]);
    let r = env.repo.create_track(
        NewTrack { id: Some("aaa".into()), edits: edits("a", "b", "c"), audio_name: "x.mp3".into(), tablatures: vec![] },
        &env.outside.join("audio.mp3"),
    );
    assert!(r.is_err());
    assert!(env.dir("aaa").join("backing.mp3").exists());
    // missing tablature source: nothing is left behind
    let r = env.repo.create_track(
        NewTrack {
            id: Some("ccc".into()),
            edits: edits("a", "b", "c"),
            audio_name: "x.mp3".into(),
            tablatures: vec![("t.gp5".into(), env.outside.join("nope"))],
        },
        &env.outside.join("audio.mp3"),
    );
    assert!(r.is_err());
    assert!(!env.dir("ccc").exists());
}

#[test]
fn scan_reports_problems_without_failing() {
    let env = Env::new();
    env.track("good", &[]);
    let tracks = env.repo.root.join("tracks");
    // bad JSON
    fs::create_dir(tracks.join("badjson")).unwrap();
    fs::write(tracks.join("badjson/track.json"), "{nope").unwrap();
    // id mismatch
    env.track("orig", &[]);
    fs::create_dir(tracks.join("copy")).unwrap();
    fs::copy(tracks.join("orig/track.json"), tracks.join("copy/track.json")).unwrap();
    // newer schema
    fs::create_dir(tracks.join("newer")).unwrap();
    fs::write(tracks.join("newer/track.json"), r#"{"schema_version": 3, "id": "newer"}"#).unwrap();
    // invalid folder name, missing track.json, dot folder, stray file
    fs::create_dir(tracks.join("bad name")).unwrap();
    fs::create_dir(tracks.join("empty")).unwrap();
    fs::create_dir(tracks.join(".hidden")).unwrap();
    fs::write(tracks.join("stray.txt"), "x").unwrap();
    let lib = env.repo.scan();
    let ok: Vec<_> = lib.tracks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ok, ["good", "orig"]);
    let bad: BTreeMap<_, _> = lib.problems.iter().map(|p| (p.dir.as_str(), p.message.as_str())).collect();
    assert_eq!(bad.len(), 5, "{bad:?}");
    assert!(bad["badjson"].contains("invalid JSON"));
    assert!(bad["copy"].contains("does not match"));
    assert!(bad["newer"].contains("newer Calliope"));
    assert!(bad["bad name"].contains("valid track id"));
    assert!(bad.contains_key("empty"));
}

#[test]
fn scan_lists_missing_files_and_never_writes() {
    let env = Env::new();
    env.track("aaa", &["one.gp5", "two.gp5"]);
    fs::remove_file(env.dir("aaa").join("two.gp5")).unwrap();
    fs::remove_file(env.dir("aaa").join("backing.mp3")).unwrap();
    let snapshot = tree(&env);
    let lib = env.repo.scan();
    assert_eq!(lib.tracks[0].missing, ["backing.mp3", "two.gp5"]);
    assert_eq!(tree(&env), snapshot);
}

#[test]
fn save_edits_updates_modified_keeps_imported_and_extra() {
    let env = Env::new();
    let rec = env.track("aaa", &[]);
    let p = env.dir("aaa").join("track.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    v["future"] = serde_json::json!({"x": 1});
    v["imported"] = "2020-01-01T00:00:00+02:00".into();
    v["modified"] = "2020-01-01T00:00:00Z".into();
    fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let rec = Repository::new(&env.repo.root).scan().tracks.into_iter().find(|t| t.id == rec.id).unwrap();
    let mut r = req(&rec, vec![]);
    r.edits.title = "  New title ".into();
    r.edits.year = None;
    let res = env.repo.save_track(r, &env.lookup()).unwrap();
    assert!(res.warnings.is_empty());
    assert_eq!(res.track.title, "New title");
    assert_eq!(res.track.year, None);
    assert_eq!(res.track.imported, "2020-01-01T00:00:00+02:00");
    assert_ne!(res.track.modified, "2020-01-01T00:00:00Z");
    assert!(res.track.modified.ends_with('Z') && res.track.modified.len() == 20);
    assert_ne!(res.track.revision, rec.revision);
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert_eq!(after["future"], serde_json::json!({"x": 1}));
    assert!(!env.dir("aaa").join("track.json.tmp").exists());
}

#[test]
fn save_add_copies_file_and_lists_it() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let res = env
        .repo
        .save_track(req(&rec, vec![keep("one.gp5"), TabEntry::Add { token: "new".into() }]), &env.lookup())
        .unwrap();
    assert_eq!(res.track.tablatures, ["one.gp5", "tab-new.gp5"]);
    assert_eq!(fs::read(env.dir("aaa").join("tab-new.gp5")).unwrap(), b"NEW TAB");
    assert!(res.track.missing.is_empty());
    assert!(!env.repo.root.join("trash").exists());
    assert!(!env.dir("aaa").join(".tab-new.gp5.part").exists());
}

#[test]
fn save_order_follows_the_request() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5", "two.gp5"]);
    let res = env.repo.save_track(req(&rec, vec![keep("two.gp5"), keep("one.gp5")]), &env.lookup()).unwrap();
    assert_eq!(res.track.tablatures, ["two.gp5", "one.gp5"]);
}

#[test]
fn save_remove_moves_file_to_trash() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5", "two.gp5"]);
    let res = env.repo.save_track(req(&rec, vec![keep("two.gp5")]), &env.lookup()).unwrap();
    assert_eq!(res.track.tablatures, ["two.gp5"]);
    assert!(!env.dir("aaa").join("one.gp5").exists());
    let trash = env.trash_files();
    assert_eq!(trash.len(), 1);
    let (k, v) = trash.iter().next().unwrap();
    assert!(k.contains("-aaa-tablatures/one.gp5"), "{k}");
    assert_eq!(v, b"content of one.gp5");
}

#[test]
fn save_replace_same_name_puts_new_content_and_trashes_old() {
    let env = Env::new();
    let rec = env.track("aaa", &["tab-new.gp5"]);
    let res = env
        .repo
        .save_track(req(&rec, vec![TabEntry::Replace { name: "tab-new.gp5".into(), token: "new".into() }]), &env.lookup())
        .unwrap();
    assert!(res.warnings.is_empty());
    assert_eq!(res.track.tablatures, ["tab-new.gp5"]);
    assert_eq!(fs::read(env.dir("aaa").join("tab-new.gp5")).unwrap(), b"NEW TAB");
    assert!(!env.dir("aaa").join(".tab-new.gp5.part").exists());
    let trash = env.trash_files();
    assert_eq!(trash.values().next().unwrap(), b"content of tab-new.gp5");
}

#[test]
fn save_replace_different_name() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let res = env
        .repo
        .save_track(req(&rec, vec![TabEntry::Replace { name: "one.gp5".into(), token: "other".into() }]), &env.lookup())
        .unwrap();
    assert_eq!(res.track.tablatures, ["other.gp"]);
    assert_eq!(fs::read(env.dir("aaa").join("other.gp")).unwrap(), b"OTHER");
    assert!(!env.dir("aaa").join("one.gp5").exists());
    assert_eq!(env.trash_files().len(), 1);
}

#[test]
fn save_conflict_changes_nothing() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let p = env.dir("aaa").join("track.json");
    let mut text = fs::read_to_string(&p).unwrap();
    text.push_str("\n\n");
    fs::write(&p, &text).unwrap();
    let snapshot = tree(&env);
    let e = env.repo.save_track(req(&rec, vec![TabEntry::Add { token: "new".into() }]), &env.lookup()).unwrap_err();
    assert!(e.contains("conflict"), "{e}");
    assert_eq!(tree(&env), snapshot);
    assert_eq!(fs::read_to_string(&p).unwrap(), text);
    let e = env.repo.delete_track("aaa", &rec.revision).unwrap_err();
    assert!(e.contains("conflict"));
    assert!(env.dir("aaa").exists());
}

#[test]
fn save_rejects_bad_input_with_nothing_written() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let snapshot = tree(&env);
    let l = env.lookup();
    let cases: Vec<Vec<TabEntry>> = vec![
        vec![keep("one.gp5"), TabEntry::Add { token: "nope".into() }],
        vec![keep("one.gp5"), TabEntry::Add { token: "missing".into() }],
        vec![keep("one.gp5"), TabEntry::Add { token: "bad".into() }],
        vec![keep("unknown.gp5")],
        vec![keep("../../x")],
        vec![keep("one.gp5"), keep("one.gp5")],
        vec![keep("one.gp5"), TabEntry::Replace { name: "one.gp5".into(), token: "new".into() }],
        vec![TabEntry::Replace { name: "ghost.gp5".into(), token: "new".into() }],
        vec![TabEntry::Add { token: "new".into() }, TabEntry::Add { token: "new".into() }],
    ];
    for tabs in cases {
        let r = env.repo.save_track(req(&rec, tabs.clone()), &l);
        assert!(r.is_err(), "{tabs:?}");
        assert_eq!(tree(&env), snapshot, "{tabs:?}");
    }
    // invalid edits
    let mut r = req(&rec, vec![keep("one.gp5"), TabEntry::Add { token: "new".into() }]);
    r.edits.title = "   ".into();
    assert!(env.repo.save_track(r, &l).is_err());
    assert_eq!(tree(&env), snapshot);
}

#[test]
fn bad_ids_are_rejected_everywhere() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let l = env.lookup();
    for id in ["../x", "a/b", ".hidden", "", "..", "a\\b"] {
        let mut r = req(&rec, vec![]);
        r.id = id.into();
        assert!(env.repo.save_track(r, &l).is_err(), "{id}");
        assert!(env.repo.delete_track(id, &rec.revision).is_err(), "{id}");
        assert!(env.repo.export_track(id, &env.outside).is_err(), "{id}");
        assert!(env.repo.export_tablature(id, "one.gp5", &env.outside.join("o.gp5")).is_err(), "{id}");
    }
    assert!(env.dir("aaa").exists());
}

#[test]
fn untracked_file_is_never_overwritten_by_add() {
    let env = Env::new();
    let rec = env.track("aaa", &[]);
    fs::write(env.dir("aaa").join("tab-new.gp5"), b"PRECIOUS").unwrap();
    let snapshot = tree(&env);
    let e = env.repo.save_track(req(&rec, vec![TabEntry::Add { token: "new".into() }]), &env.lookup());
    assert!(e.is_err());
    assert_eq!(fs::read(env.dir("aaa").join("tab-new.gp5")).unwrap(), b"PRECIOUS");
    assert_eq!(tree(&env), snapshot);
}

#[test]
fn failed_add_after_earlier_copy_rolls_back_only_our_copies() {
    let env = Env::new();
    let rec = env.track("aaa", &[]);
    fs::write(env.dir("aaa").join("other.gp"), b"PRECIOUS").unwrap();
    let snapshot = tree(&env);
    let e = env.repo.save_track(
        req(&rec, vec![TabEntry::Add { token: "new".into() }, TabEntry::Add { token: "other".into() }]),
        &env.lookup(),
    );
    assert!(e.is_err());
    assert_eq!(tree(&env), snapshot);
    assert_eq!(fs::read(env.dir("aaa").join("other.gp")).unwrap(), b"PRECIOUS");
}

#[cfg(unix)]
#[test]
fn metadata_write_failure_leaves_no_new_files() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let dir = env.dir("aaa");
    let json_before = fs::read(dir.join("track.json")).unwrap();
    // Read-only folder: copying the new file in fails first or the metadata write does.
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::File::create(dir.join("probe")).is_ok() {
        // running as root: permissions are not enforced
        fs::remove_file(dir.join("probe")).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let r = env.repo.save_track(req(&rec, vec![TabEntry::Add { token: "new".into() }]), &env.lookup());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(r.is_err());
    let mut names: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["backing.mp3", "one.gp5", "track.json"]);
    assert_eq!(fs::read(dir.join("track.json")).unwrap(), json_before);
    assert!(!env.repo.root.join("trash").exists());
}

#[cfg(unix)]
#[test]
fn metadata_write_failure_after_copy_removes_copies() {
    // Only the track.json write is made to fail (test hook), so the copy succeeds first.
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let dir = env.dir("aaa");
    crate::fsutil::FAIL_WRITE_OF.with(|f| *f.borrow_mut() = Some("track.json".into()));
    let r = env.repo.save_track(req(&rec, vec![keep("one.gp5"), TabEntry::Add { token: "new".into() }]), &env.lookup());
    crate::fsutil::FAIL_WRITE_OF.with(|f| *f.borrow_mut() = None);
    assert!(r.is_err());
    assert!(!dir.join("tab-new.gp5").exists());
    assert!(!dir.join(".tab-new.gp5.part").exists());
    assert!(dir.join("one.gp5").exists());
    assert!(!env.repo.root.join("trash").exists());
}

#[test]
fn hand_edited_and_newer_tracks_are_never_rewritten() {
    let env = Env::new();
    let rec = env.track("aaa", &[]);
    let p = env.dir("aaa").join("track.json");
    let newer = fs::read_to_string(&p).unwrap().replace("\"schema_version\": 2", "\"schema_version\": 3");
    fs::write(&p, &newer).unwrap();
    let l = env.lookup();
    assert!(env.repo.save_track(req(&rec, vec![]), &l).is_err());
    assert!(env.repo.delete_track("aaa", &rec.revision).is_err());
    assert_eq!(fs::read_to_string(&p).unwrap(), newer);
    assert!(env.dir("aaa").exists());
}

#[test]
fn delete_moves_whole_folder_to_trash() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    env.repo.delete_track("aaa", &rec.revision).unwrap();
    assert!(!env.dir("aaa").exists());
    let trash = env.trash_files();
    assert_eq!(trash.len(), 3);
    assert!(trash.keys().any(|k| k.ends_with("-aaa/one.gp5")));
    assert_eq!(trash.iter().find(|(k, _)| k.ends_with("backing.mp3")).unwrap().1, b"AUDIO");
    assert!(env.repo.scan().tracks.is_empty());
}

#[test]
fn export_track_creates_named_folders() {
    let env = Env::new();
    env.track("aaa", &["one.gp5"]);
    let dest = env.outside.join("exports");
    fs::create_dir(&dest).unwrap();
    let a = env.repo.export_track("aaa", &dest).unwrap();
    assert_eq!(a.file_name().unwrap(), "Band - Album - Title");
    let b = env.repo.export_track("aaa", &dest).unwrap();
    assert_eq!(b.file_name().unwrap(), "Band - Album - Title (2)");
    for d in [a, b] {
        assert_eq!(fs::read(d.join("one.gp5")).unwrap(), b"content of one.gp5");
        assert_eq!(fs::read(d.join("backing.mp3")).unwrap(), b"AUDIO");
        assert_eq!(fs::read(d.join("track.json")).unwrap(), fs::read(env.dir("aaa").join("track.json")).unwrap());
    }
    fs::remove_dir_all(&dest).unwrap();
}

#[test]
fn export_track_sanitises_and_drops_empty_parts() {
    let env = Env::new();
    let rec = env.track("aaa", &[]);
    let mut r = req(&rec, vec![]);
    r.edits.band = String::new();
    r.edits.title = "AC/DC: live?".into();
    env.repo.save_track(r, &env.lookup()).unwrap();
    let dest = env.outside.join("exports");
    fs::create_dir(&dest).unwrap();
    let a = env.repo.export_track("aaa", &dest).unwrap();
    assert_eq!(a.file_name().unwrap(), "Album - AC_DC_ live_");
    fs::remove_dir_all(&dest).unwrap();
}

#[test]
fn export_track_refuses_destination_inside_tracks_and_missing_sources() {
    let env = Env::new();
    env.track("aaa", &["one.gp5"]);
    env.track("bbb", &[]);
    let snapshot = tree(&env);
    assert!(env.repo.export_track("aaa", &env.dir("bbb")).is_err());
    assert!(env.repo.export_track("aaa", &env.repo.root.join("tracks")).is_err());
    assert_eq!(tree(&env), snapshot);
    fs::remove_file(env.dir("aaa").join("one.gp5")).unwrap();
    let snapshot = tree(&env);
    let dest = env.outside.join("exports");
    fs::create_dir(&dest).unwrap();
    assert!(env.repo.export_track("aaa", &dest).is_err());
    assert!(list(&dest).is_empty());
    assert_eq!(tree(&env), snapshot);
    fs::remove_dir(&dest).unwrap();
}

#[test]
fn export_tablature_copies_and_replaces_at_dest() {
    let env = Env::new();
    env.track("aaa", &["one.gp5"]);
    let dest = env.outside.join("exported.gp5");
    env.repo.export_tablature("aaa", "one.gp5", &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"content of one.gp5");
    fs::write(&dest, b"old").unwrap();
    env.repo.export_tablature("aaa", "one.gp5", &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"content of one.gp5");
    assert!(env.repo.export_tablature("aaa", "nope.gp5", &dest).is_err());
    assert!(env.repo.export_tablature("aaa", "../track.json", &dest).is_err());
    assert!(env.repo.export_tablature("aaa", "one.gp5", &env.dir("aaa").join("copy.gp5")).is_err());
    assert!(!env.dir("aaa").join("copy.gp5").exists());
    fs::remove_file(&dest).unwrap();
}

#[test]
fn export_tablature_of_missing_file_fails() {
    let env = Env::new();
    env.track("aaa", &["one.gp5"]);
    fs::remove_file(env.dir("aaa").join("one.gp5")).unwrap();
    assert!(env.repo.export_tablature("aaa", "one.gp5", &env.outside.join("x.gp5")).is_err());
    assert!(!env.outside.join("x.gp5").exists());
}

#[test]
fn save_with_missing_old_file_still_works() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5", "two.gp5"]);
    fs::remove_file(env.dir("aaa").join("one.gp5")).unwrap();
    let res = env.repo.save_track(req(&rec, vec![keep("two.gp5")]), &env.lookup()).unwrap();
    assert_eq!(res.track.tablatures, ["two.gp5"]);
    assert!(res.warnings.is_empty());
}

#[test]
fn fixture_library_sample_scans_clean() {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap().flatten() {
            let dest = to.join(e.file_name());
            if e.path().is_dir() {
                copy(&e.path(), &dest);
            } else {
                fs::copy(e.path(), dest).unwrap();
            }
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("library");
    copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-sample"), &root);
    let lib = Repository::new(&root).scan();
    assert_eq!(lib.tracks.len(), 6);
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert!(lib.tracks.iter().all(|t| t.missing.is_empty()));
}

#[test]
fn export_refuses_any_destination_inside_the_repository() {
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let root = env.repo.root.clone();
    fs::create_dir_all(root.join("trash")).unwrap();
    for d in [root.clone(), root.join("trash"), env.dir("aaa")] {
        assert!(env.repo.export_track("aaa", &d).is_err(), "{}", d.display());
    }
    assert!(env.repo.export_tablature("aaa", "one.gp5", &root.join("copy.gp5")).is_err());
    assert!(env.repo.export_tablature("aaa", "one.gp5", &root.join("trash/copy.gp5")).is_err());
    assert!(!root.join("copy.gp5").exists());
    let _ = rec;
}

#[test]
fn cut_bytes_respects_char_boundaries() {
    assert_eq!(cut_bytes("abc", 5), "abc");
    assert_eq!(cut_bytes("日日日", 4), "日");
    assert_eq!(cut_bytes("éé", 3), "é");
}

// ---------- schema v2: stems, original, never-rewrite guarantee ----------

const V1_A: &str = "0199c0a0-0000-7000-8000-000000000001";
const V1_B: &str = "0199c0a0-0000-7000-8000-000000000002";
const V2_BACKING: &str = "0199c0a0-0000-7000-8000-000000000003";
const STEM_ID: &str = "0199c0a0-0000-7000-8000-000000000101";
const STEM_NAMES: [&str; 6] = ["vocals", "drums", "bass", "guitar", "piano", "other"];

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// Every file under `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, d: &Path, m: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                m.insert(format!("{}/", p.strip_prefix(base).unwrap().display()), Vec::new());
                walk(base, &p, m);
            } else {
                m.insert(p.strip_prefix(base).unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap());
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(dir, dir, &mut m);
    m
}

/// A copy of `tests/fixtures/library-v2` plus an "outside" folder that must stay unchanged.
struct V2 {
    _tmp: TempDir,
    outside: PathBuf,
    outside_before: BTreeMap<String, Vec<u8>>,
    repo: Repository,
}

impl V2 {
    fn new() -> V2 {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("library");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("ORIGINAL.flac"), b"not the original").unwrap();
        fs::write(outside.join("notes.gp5"), b"NOTES").unwrap();
        copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-v2"), &root);
        let outside_before = snapshot(&outside);
        V2 { _tmp: tmp, outside, outside_before, repo: Repository::new(root) }
    }
    fn dir(&self, id: &str) -> PathBuf {
        self.repo.root.join("tracks").join(id)
    }
    fn record(&self, id: &str) -> TrackRecord {
        self.repo.scan().tracks.into_iter().find(|t| t.id == id).unwrap()
    }
}

impl Drop for V2 {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(snapshot(&self.outside), self.outside_before, "the outside folder changed");
        }
    }
}

#[test]
fn fixture_library_v2_scans_clean() {
    let v = V2::new();
    let lib = v.repo.scan();
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(lib.tracks.len(), 4);
    assert!(lib.tracks.iter().all(|t| t.missing.is_empty()), "{:?}", lib.tracks);
    let types: Vec<_> = lib.tracks.iter().map(|t| (t.id.as_str(), t.track_type)).collect();
    assert_eq!(
        types,
        vec![
            (V1_A, TrackType::Backing),
            (V1_B, TrackType::Backing),
            (V2_BACKING, TrackType::Backing),
            (STEM_ID, TrackType::Stem)
        ]
    );
}

#[test]
fn scan_list_export_never_rewrite_any_track_json() {
    let v = V2::new();
    let before = snapshot(&v.repo.root);
    for _ in 0..2 {
        let lib = v.repo.scan();
        assert_eq!(lib.tracks.len(), 4);
    }
    let dest = tempfile::tempdir().unwrap();
    for id in [V1_A, V1_B, V2_BACKING, STEM_ID] {
        v.repo.export_track(id, dest.path()).unwrap();
    }
    v.repo.export_tablature(V1_A, "slow-burn.gp5", &dest.path().join("t.gp5")).unwrap();
    assert_eq!(snapshot(&v.repo.root), before, "every byte of the repository is unchanged");
}

#[test]
fn v1_revision_is_the_hash_of_its_bytes_and_is_migrated_in_memory() {
    let v = V2::new();
    let bytes = fs::read(v.dir(V1_A).join("track.json")).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("\"schema_version\": 1"));
    let rec = v.record(V1_A);
    assert_eq!(rec.revision, crate::fsutil::fnv1a64_hex(&bytes));
    assert_eq!(rec.track_type, TrackType::Backing);
    assert_eq!(rec.audio.as_deref(), Some("backing.mp3"));
    assert!(rec.stems.is_empty() && rec.original.is_none() && rec.stem_model.is_none());
    let v2 = fs::read(v.dir(V2_BACKING).join("track.json")).unwrap();
    assert_eq!(v.record(V2_BACKING).revision, crate::fsutil::fnv1a64_hex(&v2));
}

#[test]
fn saving_a_v1_track_writes_schema_2_and_keeps_unknown_fields() {
    let v = V2::new();
    let p = v.dir(V1_A).join("track.json");
    let mut j: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    j["x_note"] = "keep me".into();
    fs::write(&p, serde_json::to_vec_pretty(&j).unwrap()).unwrap();
    let other_before = fs::read(v.dir(V1_B).join("track.json")).unwrap();
    let rec = v.record(V1_A);
    let mut r = req(&rec, vec![keep("slow-burn.gp5"), keep("slow-burn-solo.gp")]);
    r.edits.title = "Slow Burn (edit)".into();
    let res = v.repo.save_track(r, &|_| None).unwrap();
    assert_eq!(res.track.track_type, TrackType::Backing);
    let j: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert_eq!(j["schema_version"], 2);
    assert_eq!(j["type"], "backing");
    assert_eq!(j["stems"], serde_json::json!([]));
    assert_eq!(j["x_note"], "keep me");
    assert_eq!(j["title"], "Slow Burn (edit)");
    assert_eq!(j["audio"], "backing.mp3");
    // another v1 track is untouched by that save
    assert_eq!(fs::read(v.dir(V1_B).join("track.json")).unwrap(), other_before);
}

#[test]
fn stem_track_lists_six_stems_and_the_original() {
    let v = V2::new();
    let rec = v.record(STEM_ID);
    assert_eq!(rec.track_type, TrackType::Stem);
    assert_eq!(rec.audio, None);
    assert_eq!(rec.original.as_deref(), Some("original.flac"));
    assert_eq!(rec.stem_model.as_deref(), Some("htdemucs_6s"));
    let names: Vec<&str> = rec.stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, STEM_NAMES);
    assert!(rec.stems.iter().all(|s| s.file == format!("stems/{}.flac", s.name)));
    assert!(rec.missing.is_empty());
}

#[test]
fn missing_reports_a_deleted_stem_and_original() {
    let v = V2::new();
    fs::remove_file(v.dir(STEM_ID).join("stems/drums.flac")).unwrap();
    fs::remove_file(v.dir(STEM_ID).join("original.flac")).unwrap();
    let rec = v.record(STEM_ID);
    assert_eq!(rec.missing, vec!["original.flac", "stems/drums.flac"]);
    let dest = tempfile::tempdir().unwrap();
    let e = v.repo.export_track(STEM_ID, dest.path()).unwrap_err();
    assert!(e.contains("stems/drums.flac") && e.contains("original.flac"), "{e}");
    assert_eq!(fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[test]
fn export_copies_stems_and_original() {
    let v = V2::new();
    let dest = tempfile::tempdir().unwrap();
    let out = v.repo.export_track(STEM_ID, dest.path()).unwrap();
    assert_eq!(out.file_name().unwrap(), "The Example Band - Test Pressings - Glass Harbour");
    let src = snapshot(&v.dir(STEM_ID));
    assert_eq!(snapshot(&out), src);
    assert!(out.join("stems/vocals.flac").is_file() && out.join("original.flac").is_file());
    assert_eq!(src.keys().filter(|k| k.starts_with("stems/") && k.ends_with(".flac")).count(), 6);
}

#[test]
fn failed_stem_export_leaves_nothing_behind() {
    let v = V2::new();
    fs::remove_file(v.dir(STEM_ID).join("stems/other.flac")).unwrap();
    let dest = tempfile::tempdir().unwrap();
    assert!(v.repo.export_track(STEM_ID, dest.path()).is_err());
    assert_eq!(fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[test]
fn deleting_a_stem_track_moves_the_whole_folder_to_the_trash() {
    let v = V2::new();
    let before = snapshot(&v.dir(STEM_ID));
    let rec = v.record(STEM_ID);
    v.repo.delete_track(STEM_ID, &rec.revision).unwrap();
    assert!(!v.dir(STEM_ID).exists());
    let trash: Vec<_> = fs::read_dir(v.repo.root.join("trash")).unwrap().flatten().collect();
    assert_eq!(trash.len(), 1);
    assert_eq!(snapshot(&trash[0].path()), before);
}

#[test]
fn saving_a_stem_track_keeps_audio_original_and_stems_verbatim() {
    let v = V2::new();
    let files_before = snapshot(&v.dir(STEM_ID));
    let rec = v.record(STEM_ID);
    let mut r = req(&rec, vec![TabEntry::Add { token: "t".into() }]);
    r.edits.title = "Glass Harbour (edit)".into();
    let notes = v.outside.join("notes.gp5");
    let res = v.repo.save_track(r, &|t| (t == "t").then(|| notes.clone())).unwrap();
    let t = res.track;
    assert_eq!((t.track_type, &t.audio, &t.original), (TrackType::Stem, &None, &rec.original));
    assert_eq!(t.stems, rec.stems);
    assert_eq!(t.stem_model, rec.stem_model);
    assert_eq!(t.tablatures, vec!["notes.gp5"]);
    let after = snapshot(&v.dir(STEM_ID));
    for (k, bytes) in &files_before {
        if k != "track.json" {
            assert_eq!(after.get(k), Some(bytes), "{k}");
        }
    }
    let j: serde_json::Value = serde_json::from_slice(&after["track.json"]).unwrap();
    assert_eq!(j["schema_version"], 2);
    assert_eq!(j["type"], "stem");
    assert!(j["audio"].is_null());
    assert_eq!(j["original"], "original.flac");
}

#[test]
fn a_tablature_cannot_take_the_name_of_the_original_or_a_stem() {
    let v = V2::new();
    let rec = v.record(STEM_ID);
    let before = snapshot(&v.dir(STEM_ID));
    let clash = v.outside.join("ORIGINAL.flac");
    let r = req(&rec, vec![TabEntry::Add { token: "t".into() }]);
    let e = v.repo.save_track(r, &|_| Some(clash.clone())).unwrap_err();
    assert!(e.contains("clashes"), "{e}");
    assert_eq!(snapshot(&v.dir(STEM_ID)), before);
}

#[test]
fn unknown_track_type_and_bad_stem_files_are_problems_never_rewritten() {
    let v = V2::new();
    let p = v.dir(STEM_ID).join("track.json");
    let good = fs::read_to_string(&p).unwrap();
    for (from, to) in [
        ("\"type\": \"stem\"", "\"type\": \"video\""),
        ("stems/vocals.flac", "stems/../vocals.flac"),
        ("\"stems/drums.flac\"", "\"drums.flac\""),
    ] {
        let bad = good.replace(from, to);
        assert_ne!(bad, good);
        fs::write(&p, &bad).unwrap();
        let lib = v.repo.scan();
        assert_eq!(lib.tracks.len(), 3, "{to}");
        assert_eq!(lib.problems.len(), 1, "{to}");
        assert_eq!(fs::read_to_string(&p).unwrap(), bad);
        let rec = TrackRecord { id: STEM_ID.into(), ..v.record(V2_BACKING) };
        assert!(v.repo.save_track(req(&rec, vec![]), &|_| None).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), bad);
    }
}

// ---- staged track creation (task 5) ----

mod staging {
    use super::*;

    const ID: &str = "0199c1a2-0000-7000-8000-000000000001";

    fn stem_meta(id: &str, with_original: bool) -> TrackMeta {
        let now = track_meta::now_rfc3339();
        TrackMeta {
            schema_version: track_meta::CURRENT_SCHEMA,
            id: id.into(),
            track_type: TrackType::Stem,
            band: "B".into(),
            album: "A".into(),
            title: "T".into(),
            composers: vec![],
            year: None,
            source_url: None,
            copyright: None,
            audio: None,
            original: with_original.then(|| "original.flac".to_string()),
            stems: ["vocals", "drums"]
                .iter()
                .map(|n| StemEntry { name: n.to_string(), file: format!("stems/{n}.flac") })
                .collect(),
            stem_model: Some("htdemucs_6s".into()),
            tablatures: vec![],
            imported: now.clone(),
            modified: now,
            backings: Vec::new(),
            extra: Default::default(),
        }
    }

    fn fill(st: &Staging) {
        for n in ["vocals", "drums"] {
            let part = st.stem_part_path(n).unwrap();
            fs::write(&part, format!("fLaC {n}")).unwrap();
            st.finish_stem(n).unwrap();
        }
    }

    fn job_audio(env: &Env) -> PathBuf {
        let p = env.sources.join("audio.flac");
        fs::write(&p, "ORIGINAL AUDIO").unwrap();
        p
    }

    #[test]
    fn commit_gives_a_scan_visible_stem_track() {
        let env = Env::new();
        let st = env.repo.begin_staged_track(ID).unwrap();
        assert_eq!(st.dir(), env.repo.root.join("tracks").join(format!(".staging-{ID}")));
        assert!(st.dir().join(STAGING_MARKER).is_file());
        fill(&st);
        // during staging a scan lists nothing and reports no problem
        let lib = env.repo.scan();
        assert!(lib.tracks.is_empty() && lib.problems.is_empty(), "{lib:?}");
        let rec = st.commit(&stem_meta(ID, false)).unwrap();
        assert_eq!(rec.track_type, TrackType::Stem);
        assert!(rec.missing.is_empty() && rec.original.is_none() && rec.audio.is_none());
        assert_eq!(rec.stems.len(), 2);
        let dir = env.dir(ID);
        assert!(dir.join("track.json").is_file() && dir.join("stems/vocals.flac").is_file());
        assert!(!dir.join(STAGING_MARKER).exists());
        assert!(!env.repo.root.join("tracks").join(format!(".staging-{ID}")).exists());
        let lib = env.repo.scan();
        assert_eq!((lib.tracks.len(), lib.problems.len()), (1, 0));
        assert_eq!(lib.tracks[0], rec);
    }

    #[test]
    fn commit_with_the_original_moves_it_into_the_track() {
        let env = Env::new();
        let src = job_audio(&env);
        let mut st = env.repo.begin_staged_track(ID).unwrap();
        fill(&st);
        st.adopt_original(&src).unwrap();
        assert!(!src.exists(), "renamed, not copied");
        let rec = st.commit(&stem_meta(ID, true)).unwrap();
        assert_eq!(rec.original.as_deref(), Some("original.flac"));
        assert!(rec.missing.is_empty());
        assert_eq!(fs::read_to_string(env.dir(ID).join("original.flac")).unwrap(), "ORIGINAL AUDIO");
        fs::write(&src, "ORIGINAL AUDIO").unwrap(); // let Env::drop see an unchanged sources dir
    }

    #[test]
    fn commit_refuses_when_the_track_exists_and_changes_nothing() {
        let env = Env::new();
        env.track(ID, &[]);
        let before = list(&env.repo.root.join("tracks"));
        assert!(env.repo.begin_staged_track(ID).unwrap_err().contains("already exists"));
        assert_eq!(list(&env.repo.root.join("tracks")), before);

        // the track appears while staging: commit refuses, the staging folder stays
        let other = "0199c1a2-0000-7000-8000-000000000002";
        let st = env.repo.begin_staged_track(other).unwrap();
        fill(&st);
        fs::create_dir(env.dir(other)).unwrap();
        fs::write(env.dir(other).join("mine"), "user data").unwrap();
        assert!(st.commit(&stem_meta(other, false)).unwrap_err().contains("already exists"));
        assert_eq!(fs::read_to_string(env.dir(other).join("mine")).unwrap(), "user data");
        assert!(st.dir().join("stems/vocals.flac").is_file());
        st.abandon().unwrap();
        assert!(env.dir(other).join("mine").exists());
    }

    #[test]
    fn commit_validates_and_keeps_the_staging_folder_on_failure() {
        let env = Env::new();
        let st = env.repo.begin_staged_track(ID).unwrap();
        // listed files missing
        assert!(st.commit(&stem_meta(ID, false)).unwrap_err().contains("missing"));
        fill(&st);
        assert!(st.commit(&stem_meta(ID, true)).unwrap_err().contains("original.flac"));
        // wrong id, bad title
        assert!(st.commit(&stem_meta("other", false)).is_err());
        let mut bad = stem_meta(ID, false);
        bad.title = " ".into();
        assert!(st.commit(&bad).is_err());
        assert!(st.dir().join(STAGING_MARKER).is_file());
        assert!(!env.dir(ID).exists());
        st.abandon().unwrap();
        assert!(!env.repo.root.join("tracks").join(format!(".staging-{ID}")).exists());
    }

    #[test]
    fn stem_names_are_validated_and_never_replace() {
        let env = Env::new();
        let st = env.repo.begin_staged_track(ID).unwrap();
        for bad in ["../x", "a/b", "", "Vocals", ".hidden"] {
            assert!(st.stem_part_path(bad).is_err(), "{bad}");
        }
        fs::write(st.stem_part_path("bass").unwrap(), "1").unwrap();
        st.finish_stem("bass").unwrap();
        fs::write(st.stem_part_path("bass").unwrap(), "2").unwrap();
        assert!(st.finish_stem("bass").is_err());
        assert_eq!(fs::read_to_string(st.dir().join("stems/bass.flac")).unwrap(), "1");
        st.abandon().unwrap();
    }

    #[test]
    fn abandon_after_adopt_original_puts_audio_back() {
        let env = Env::new();
        let src = job_audio(&env);
        let mut st = env.repo.begin_staged_track(ID).unwrap();
        fill(&st);
        st.adopt_original(&src).unwrap();
        assert!(st.adopt_original(&src).is_err());
        let dir = st.dir().to_path_buf();
        st.abandon().unwrap();
        assert_eq!(fs::read_to_string(&src).unwrap(), "ORIGINAL AUDIO");
        assert!(!dir.exists());
    }

    #[test]
    fn abandon_keeps_everything_when_the_original_cannot_go_back() {
        let env = Env::new();
        let src = job_audio(&env);
        let mut st = env.repo.begin_staged_track(ID).unwrap();
        st.adopt_original(&src).unwrap();
        fs::write(&src, "NEW FILE IN ITS PLACE").unwrap();
        let dir = st.dir().to_path_buf();
        assert!(st.abandon().is_err());
        assert_eq!(fs::read_to_string(dir.join("original.flac")).unwrap(), "ORIGINAL AUDIO");
        assert_eq!(fs::read_to_string(&src).unwrap(), "NEW FILE IN ITS PLACE");
    }

    #[test]
    fn adopt_original_refuses_non_files_and_symlinks() {
        let env = Env::new();
        let mut st = env.repo.begin_staged_track(ID).unwrap();
        assert!(st.adopt_original(&env.sources.join("missing.flac")).is_err());
        assert!(st.adopt_original(&env.sources).is_err());
        #[cfg(unix)]
        {
            let link = env.sources.join("link.flac");
            std::os::unix::fs::symlink(env.outside.join("audio.mp3"), &link).unwrap();
            assert!(st.adopt_original(&link).is_err());
            fs::remove_file(&link).unwrap();
        }
        st.abandon().unwrap();
    }

    #[test]
    fn begin_refuses_bad_ids_and_an_existing_staging_folder() {
        let env = Env::new();
        for bad in ["", "../x", "a/b", ".x"] {
            assert!(env.repo.begin_staged_track(bad).is_err(), "{bad}");
        }
        let st = env.repo.begin_staged_track(ID).unwrap();
        assert!(env.repo.begin_staged_track(ID).is_err());
        assert!(st.dir().exists());
        st.abandon().unwrap();
    }

    #[test]
    fn abandon_and_stale_cleanup_refuse_what_is_not_ours() {
        let env = Env::new();
        let tracks = env.repo.root.join("tracks");
        // no marker: a user's folder with a staging-like name
        let user = tracks.join(".staging-mine");
        fs::create_dir(&user).unwrap();
        fs::write(user.join("data"), "user").unwrap();
        // a marker folder with an invalid id in its name and a plain file
        let odd = tracks.join(".staging-has space");
        fs::create_dir(&odd).unwrap();
        fs::write(odd.join(STAGING_MARKER), "").unwrap();
        fs::write(tracks.join(".staging-file"), "user").unwrap();
        // ours: stale, with a marker, plus the running one
        let stale = env.repo.begin_staged_track("stale-1").unwrap();
        let running = env.repo.begin_staged_track("running-1").unwrap();
        fs::write(stale.dir().join("stems/x.flac.part"), "x").unwrap();
        assert_eq!(env.repo.clean_stale_staging(Some("running-1")), 1);
        assert!(!stale.dir().exists() && running.dir().exists());
        assert!(user.join("data").exists() && odd.join(STAGING_MARKER).exists());
        assert!(tracks.join(".staging-file").is_file());
        running.abandon().unwrap();
        // the marker is gone: abandon refuses and leaves the folder
        let st = env.repo.begin_staged_track("nomarker").unwrap();
        fs::remove_file(st.dir().join(STAGING_MARKER)).unwrap();
        let dir = st.dir().to_path_buf();
        assert!(st.abandon().is_err());
        assert!(dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_staging_folders_are_never_followed() {
        let env = Env::new();
        let tracks = env.repo.root.join("tracks");
        let target = env.sources.join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join(STAGING_MARKER), "").unwrap();
        fs::write(target.join("user-file"), "keep").unwrap();
        std::os::unix::fs::symlink(&target, tracks.join(".staging-linked")).unwrap();
        assert_eq!(env.repo.clean_stale_staging(None), 0);
        assert!(target.join("user-file").exists());

        // a link inside a real staging folder is removed as a link only
        let st = env.repo.begin_staged_track(ID).unwrap();
        std::os::unix::fs::symlink(&target, st.dir().join("stems/link")).unwrap();
        st.abandon().unwrap();
        assert!(target.join("user-file").exists());
        fs::remove_dir_all(&target).unwrap(); // our own test data
        fs::remove_file(tracks.join(".staging-linked")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn begin_refuses_a_symlinked_tracks_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let root = tmp.path().join("root");
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("tracks")).unwrap();
        assert!(Repository::new(&root).begin_staged_track(ID).is_err());
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }
}

mod backing {
    use super::*;

    const FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
    const MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
    const PLAIN: &str = "0199c0a0-0000-7000-8000-000000000203";

    /// A copy of `tests/fixtures/library-editor` plus an "outside" folder that must not change.
    struct Ed {
        _tmp: TempDir,
        outside: PathBuf,
        before: BTreeMap<String, Vec<u8>>,
        repo: Repository,
    }

    impl Ed {
        fn new() -> Ed {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("library");
            let outside = tmp.path().join("outside");
            fs::create_dir_all(&outside).unwrap();
            fs::write(outside.join("keep.txt"), b"KEEP").unwrap();
            copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor"), &root);
            let before = snapshot(&outside);
            Ed { _tmp: tmp, outside, before, repo: Repository::new(root) }
        }
        fn dir(&self, id: &str) -> PathBuf {
            self.repo.root.join("tracks").join(id)
        }
        fn record(&self, id: &str) -> TrackRecord {
            self.repo.scan().tracks.into_iter().find(|t| t.id == id).unwrap()
        }
        fn json(&self, id: &str) -> serde_json::Value {
            serde_json::from_slice(&fs::read(self.dir(id).join("track.json")).unwrap()).unwrap()
        }
        fn part(&self, id: &str, content: &[u8]) -> PathBuf {
            let p = self.dir(id).join(".calliope-backing-test.part");
            fs::write(&p, content).unwrap();
            p
        }
        fn save(&self, id: &str, variant: Option<&str>, content: &[u8]) -> Result<SaveResult, String> {
            let part = self.part(id, content);
            let stems = self.record(id).stems;
            let r = self.repo.save_backing(BackingSave {
                id,
                variant,
                stems: &stems,
                part: &part,
                mix: serde_json::json!({"stems": [{"name": "vocals", "gain_db": -3.5, "unmuted": true}]}),
                sample_rate: 44100,
                bits: 16,
            });
            if r.is_err() {
                let _ = fs::remove_file(&part); // the caller's job on failure
            }
            r
        }
    }

    impl Drop for Ed {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                assert_eq!(snapshot(&self.outside), self.before, "the outside folder changed");
            }
        }
    }

    #[test]
    fn first_save_creates_the_file_and_one_variant() {
        let e = Ed::new();
        let before = e.json(FOUR);
        e.save(FOUR, None, b"FLAC1").unwrap();
        assert_eq!(fs::read(e.dir(FOUR).join("backings/backing.flac")).unwrap(), b"FLAC1");
        assert!(!e.dir(FOUR).join(".calliope-backing-test.part").exists());
        let j = e.json(FOUR);
        let b = &j["backings"];
        assert_eq!(b.as_array().unwrap().len(), 1);
        assert_eq!((b[0]["id"].as_str(), b[0]["name"].as_str()), (Some("backing"), Some("Backing")));
        assert_eq!(b[0]["file"], "backings/backing.flac");
        assert_eq!(b[0]["created"], b[0]["modified"]);
        assert_eq!(b[0]["created"], j["modified"]);
        assert_eq!((b[0]["sample_rate"].as_u64(), b[0]["bits"].as_u64()), (Some(44100), Some(16)));
        assert_eq!(b[0]["mix"]["stems"][0]["name"], "vocals");
        for k in ["type", "audio", "stems", "title", "imported", "band", "album", "stem_model", "tablatures"] {
            assert_eq!(j[k], before[k], "{k}");
        }
        assert!(e.record(FOUR).missing.is_empty());
        assert!(j["modified"].as_str().unwrap() >= before["modified"].as_str().unwrap());
        assert_ne!(j["modified"], before["modified"]);
    }

    #[test]
    fn second_save_trashes_the_previous_file_and_keeps_created() {
        let e = Ed::new();
        e.save(FOUR, None, b"FLAC1").unwrap();
        let created = e.json(FOUR)["backings"][0]["created"].clone();
        // make the stored timestamps distinguishable
        let tj = e.dir(FOUR).join("track.json");
        let text = fs::read_to_string(&tj).unwrap().replace(created.as_str().unwrap(), "2020-01-01T00:00:00Z");
        fs::write(&tj, text).unwrap();
        e.save(FOUR, Some("backing"), b"FLAC2").unwrap();
        assert_eq!(fs::read(e.dir(FOUR).join("backings/backing.flac")).unwrap(), b"FLAC2");
        let b = &e.json(FOUR)["backings"];
        assert_eq!(b.as_array().unwrap().len(), 1);
        assert_eq!(b[0]["created"], "2020-01-01T00:00:00Z");
        assert_ne!(b[0]["modified"], "2020-01-01T00:00:00Z");
        let trash: Vec<_> = fs::read_dir(e.repo.root.join("trash")).unwrap().flatten().collect();
        assert_eq!(trash.len(), 1);
        let name = trash[0].file_name().to_string_lossy().into_owned();
        assert!(name.ends_with(&format!("-{FOUR}-backing")), "{name}");
        assert_eq!(fs::read(trash[0].path().join("backing.flac")).unwrap(), b"FLAC1");
    }

    #[test]
    fn replacing_the_fixture_variant_works() {
        let e = Ed::new();
        let old = fs::read(e.dir(MIXED).join("backings/backing.flac")).unwrap();
        e.save(MIXED, Some("backing"), b"NEW").unwrap();
        assert_eq!(fs::read(e.dir(MIXED).join("backings/backing.flac")).unwrap(), b"NEW");
        assert_eq!(e.json(MIXED)["backings"][0]["sample_rate"], 44100);
        let trash: Vec<_> = fs::read_dir(e.repo.root.join("trash")).unwrap().flatten().collect();
        assert_eq!(fs::read(trash[0].path().join("backing.flac")).unwrap(), old);
    }

    #[test]
    fn an_unlisted_user_file_is_never_overwritten() {
        let e = Ed::new();
        fs::create_dir(e.dir(FOUR).join("backings")).unwrap();
        fs::write(e.dir(FOUR).join("backings/backing.flac"), b"USER").unwrap();
        e.save(FOUR, None, b"MINE").unwrap();
        assert_eq!(fs::read(e.dir(FOUR).join("backings/backing.flac")).unwrap(), b"USER");
        assert_eq!(fs::read(e.dir(FOUR).join("backings/backing-2.flac")).unwrap(), b"MINE");
        let j = e.json(FOUR);
        assert_eq!(j["backings"][0]["id"], "backing-2");
        assert_eq!(j["backings"][0]["file"], "backings/backing-2.flac");
        assert!(!e.repo.root.join("trash").exists());
    }

    #[test]
    fn a_new_variant_id_next_to_an_existing_one_gets_a_suffix() {
        let e = Ed::new();
        e.save(MIXED, Some("other"), b"X").unwrap(); // not existing: created
        e.save(MIXED, Some("other"), b"Y").unwrap(); // now existing: replaced
        let ids: Vec<_> = e.json(MIXED)["backings"].as_array().unwrap().iter().map(|b| b["id"].clone()).collect();
        assert_eq!(ids, vec!["backing", "other"]);
        // no target while `backing` is already listed: a new variant `backing-2`, never a replace
        e.save(MIXED, None, b"Z").unwrap();
        let j = e.json(MIXED);
        assert_eq!(j["backings"].as_array().unwrap().len(), 3);
        assert_eq!(j["backings"][2]["id"], "backing-2");
    }

    #[test]
    fn a_changed_stem_list_is_a_conflict_and_nothing_changes() {
        let e = Ed::new();
        let part = e.part(FOUR, b"P");
        let mut stems = e.record(FOUR).stems;
        stems.pop();
        let snap = snapshot(&e.repo.root);
        let r = e.repo.save_backing(BackingSave {
            id: FOUR,
            variant: None,
            stems: &stems,
            part: &part,
            mix: serde_json::Value::Null,
            sample_rate: 44100,
            bits: 16,
        });
        assert!(r.unwrap_err().starts_with("conflict:"));
        assert_eq!(snapshot(&e.repo.root), snap);
        // a track that is not a stem track conflicts too
        let part = e.part(PLAIN, b"P");
        let r = e.repo.save_backing(BackingSave {
            id: PLAIN,
            variant: None,
            stems: &[],
            part: &part,
            mix: serde_json::Value::Null,
            sample_rate: 44100,
            bits: 16,
        });
        assert!(r.unwrap_err().starts_with("conflict:"));
    }

    #[test]
    fn edits_made_meanwhile_are_kept() {
        let e = Ed::new();
        let tj = e.dir(FOUR).join("track.json");
        let text = fs::read_to_string(&tj).unwrap().replace("Four Lanes", "Edited On Disk");
        fs::write(&tj, text).unwrap();
        e.save(FOUR, None, b"F").unwrap();
        assert_eq!(e.json(FOUR)["title"], "Edited On Disk");
    }

    #[test]
    fn the_library_save_keeps_backings_and_adds_none_to_other_tracks() {
        let e = Ed::new();
        let plain_before = fs::read(e.dir(PLAIN).join("track.json")).unwrap();
        let scan_before = snapshot(&e.repo.root);
        e.repo.scan();
        assert_eq!(snapshot(&e.repo.root), scan_before);
        for (id, has) in [(PLAIN, false), (MIXED, true)] {
            let rec = e.record(id);
            let mut r = req(&rec, vec![]);
            r.tablatures = rec.tablatures.iter().map(|n| keep(n)).collect();
            r.edits.title = format!("{} x", rec.title);
            let before = e.json(id);
            e.repo.save_track(r, &|_| None).unwrap();
            let after = e.json(id);
            assert_eq!(after.get("backings").is_some(), has, "{id}");
            assert_eq!(after["backings"], before["backings"], "{id}");
        }
        assert!(!String::from_utf8(fs::read(e.dir(PLAIN).join("track.json")).unwrap()).unwrap().contains("backings"));
        let _ = plain_before;
    }

    #[test]
    fn missing_backing_files_are_listed_and_export_includes_them() {
        let e = Ed::new();
        fs::remove_file(e.dir(MIXED).join("backings/backing.flac")).unwrap();
        assert_eq!(e.record(MIXED).missing, vec!["backings/backing.flac"]);
        assert!(e.repo.export_track(MIXED, &e.outside.join("nowhere")).is_err());
        let e = Ed::new();
        let dest = e.outside.parent().unwrap().join("exports");
        fs::create_dir(&dest).unwrap();
        let out = e.repo.export_track(MIXED, &dest).unwrap();
        assert!(out.join("backings/backing.flac").is_file());
        assert!(out.join("stems/vocals.flac").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_backings_folder_is_refused() {
        let e = Ed::new();
        std::os::unix::fs::symlink(&e.outside, e.dir(FOUR).join("backings")).unwrap();
        let err = e.save(FOUR, None, b"X").unwrap_err();
        assert!(err.contains("backings"), "{err}");
        assert_eq!(snapshot(&e.outside), e.before);
        assert!(e.json(FOUR).get("backings").is_none());
        fs::remove_file(e.dir(FOUR).join("backings")).unwrap();
    }

    #[test]
    fn a_failed_metadata_write_leaves_the_old_state() {
        let e = Ed::new();
        e.save(MIXED, Some("backing"), b"NEW").unwrap_or_else(|x| panic!("{x}"));
        let snap = snapshot(&e.dir(MIXED));
        crate::fsutil::FAIL_WRITE_OF.with(|f| *f.borrow_mut() = Some("track.json".into()));
        let r = e.save(MIXED, Some("backing"), b"NEWER");
        crate::fsutil::FAIL_WRITE_OF.with(|f| *f.borrow_mut() = None);
        assert!(r.is_err());
        assert_eq!(snapshot(&e.dir(MIXED)), snap);
    }
}
