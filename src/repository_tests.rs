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
    fs::write(tracks.join("newer/track.json"), r#"{"schema_version": 2, "id": "newer"}"#).unwrap();
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
    // Only track.json.tmp is blocked (a directory by that name), so the copy succeeds first.
    let env = Env::new();
    let rec = env.track("aaa", &["one.gp5"]);
    let dir = env.dir("aaa");
    fs::create_dir(dir.join("track.json.tmp")).unwrap();
    let r = env.repo.save_track(req(&rec, vec![keep("one.gp5"), TabEntry::Add { token: "new".into() }]), &env.lookup());
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
    let newer = fs::read_to_string(&p).unwrap().replace("\"schema_version\": 1", "\"schema_version\": 2");
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
