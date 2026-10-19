//! Headless acceptance for gui-existing-track-import: a typed path (plain, quoted, file://) goes
//! through `local_source::resolve` -> `start_file` -> ready -> `start_extraction` -> saved, with the
//! REAL `calliope-stems` on 127.0.0.1 (stub separator, `ok` mode) and the local ffmpeg/ffprobe on
//! fixtures copied into a temp dir. User files and a sibling "outside" folder must never change.
#![allow(dead_code, unused_imports, clippy::all)]

use calliope_gui::fsutil;
use calliope_gui::track_meta;
use calliope_gui::repository;
use calliope_gui::import_tmp;
use calliope_gui::media;
use calliope_gui::download;
use calliope_gui::tools;
use calliope_gui::stem_audio;
use calliope_gui::import_job;
use calliope_gui::local_source;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use import_job::*;
use local_source::{resolve, LocalKind};
use repository::Repository;
use tools::Tools;
use track_meta::TrackType;

fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn fixture(name: &str) -> PathBuf {
    project().join("tests/fixtures/import").join(name)
}
fn tool(name: &str) -> PathBuf {
    tools::find_in_path(name, &std::env::var_os("PATH").unwrap_or_default()).unwrap_or_else(|| panic!("{name} is needed"))
}
fn stems_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let bin = exe.parent().unwrap().parent().unwrap().join("calliope-stems");
    assert!(bin.is_file(), "{} is missing: cargo build -p calliope-stems", bin.display());
    bin
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// path -> description (kind, size, mtime ns, content hash for files, link target for links).
fn fingerprint(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
            let md = fs::symlink_metadata(&p).unwrap();
            let mt = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            if md.file_type().is_symlink() {
                out.insert(rel, format!("link -> {:?}", fs::read_link(&p).unwrap()));
            } else if md.is_dir() {
                out.insert(rel.clone(), "dir".into());
                walk(base, &p, out);
            } else {
                let bytes = fs::read(&p).unwrap_or_default();
                let mut h: u64 = 0xcbf29ce484222325;
                for b in &bytes {
                    h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
                }
                out.insert(rel, format!("file {} {mt} {h:x} mode={:o}", bytes.len(), md.permissions().mode()));
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(dir, dir, &mut m);
    m
}

/// Pids whose command line or environment mentions `needle`.
fn procs_mentioning(needle: &str) -> Vec<(u32, String)> {
    let me = std::process::id();
    let mut v = Vec::new();
    for e in fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        if let Ok(st) = fs::read_to_string(e.path().join("stat")) {
            if st.rsplit(") ").next().is_some_and(|r| r.starts_with('Z')) {
                continue;
            }
        }
        let cmd = fs::read(e.path().join("cmdline")).unwrap_or_default();
        let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ");
        let env = fs::read(e.path().join("environ")).unwrap_or_default();
        if cmd.contains(needle) || String::from_utf8_lossy(&env).contains(needle) {
            v.push((pid, cmd));
        }
    }
    v
}

fn kill_all_mentioning(needle: &str) {
    for _ in 0..3 {
        for (pid, _) in procs_mentioning(needle) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        if procs_mentioning(needle).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}
// ------------------------------------------------------------------ servers

/// The real server with the stub separator; killed when dropped.
struct RealServer {
    child: Child,
    url: String,
    work: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
    _outer: tempfile::TempDir,
}

impl Drop for RealServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        kill_all_mentioning(&self._outer.path().display().to_string());
    }
}

impl RealServer {
    fn start(mode: &str) -> RealServer {
        let outer = tempfile::tempdir().unwrap();
        let work = outer.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("stub-mode"), format!("{mode}\n")).unwrap();
        let mut child = Command::new(stems_binary())
            .arg("--separator")
            .arg(project().join("tests/support/stub-separator"))
            .args(["--listen", "127.0.0.1:0", "--work-dir"])
            .arg(&work)
            .env_remove("STUB_SEPARATOR_MODE")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        let addr: SocketAddr = loop {
            line.clear();
            assert!(lines.read_line(&mut line).unwrap() > 0, "server exited before listening");
            if let Some(rest) = line.split("listening addr=").nth(1) {
                break rest.split_whitespace().next().unwrap().parse().unwrap();
            }
        };
        assert!(addr.ip().is_loopback());
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            while lines.read_line(&mut s).unwrap_or(0) > 0 {
                l2.lock().unwrap().push(s.trim_end().to_string());
                s.clear();
            }
        });
        RealServer { child, url: format!("http://{addr}"), work, log, _outer: outer }
    }
    fn saw(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }
    fn count(&self, needle: &str) -> usize {
        self.log.lock().unwrap().iter().filter(|l| l.contains(needle)).count()
    }
    fn job_dirs(&self) -> Vec<String> {
        names_in(&self.work.join("jobs"))
    }
}

// ------------------------------------------------------------------ rig

struct Rig {
    dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    sources: PathBuf,
    state: ImportState,
    sink: Sink,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let outside = dir.path().join("outside");
        let sources = dir.path().join("sources");
        for d in [&outside, &sources] {
            fs::create_dir_all(d).unwrap();
        }
        Repository::new(&root).ensure_layout().unwrap();
        fs::write(outside.join("precious.txt"), "do not touch").unwrap();
        let tools = Tools { yt_dlp: None, ffmpeg: Some(tool("ffmpeg")), ffprobe: Some(tool("ffprobe")) };
        let repo_lock = Arc::new(Mutex::new(()));
        let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
            let _g = repo_lock.lock().unwrap_or_else(|e| e.into_inner());
            f()
        });
        let sink: Sink = Arc::new(|_| {});
        let state = ImportState::new(tools, lock, Duration::from_millis(40));
        Rig { dir, root, outside, sources, state, sink }
    }

    /// Copies a fixture into the sources folder under `as_name`.
    fn put(&self, fixture_name: &str, as_name: &str) -> PathBuf {
        let p = self.sources.join(as_name);
        fs::copy(fixture(fixture_name), &p).unwrap();
        p
    }

    fn start(&self, path: &Path) -> Result<JobSnapshot, String> {
        let kind = if path.extension().is_some_and(|e| e == "mp4") { FileKind::Video } else { FileKind::Audio };
        self.state.start_file(&self.root, kind, path, self.sink.clone())
    }

    fn idle(&self) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(90);
        loop {
            if !self.state.is_running() {
                if let Some(s) = self.state.snapshot() {
                    return s;
                }
            }
            assert!(Instant::now() < end, "timed out waiting for idle");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn ready(&self) -> JobSnapshot {
        let s = self.idle();
        assert_eq!(s.phase, Phase::Ready, "{:?}", s.error);
        s
    }
    fn wait_saved(&self) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(s) = self.state.snapshot() {
                if s.phase == Phase::Saved {
                    return s;
                }
                assert!(s.phase != Phase::Failed, "{:?}", s.error);
            }
            assert!(Instant::now() < end, "timed out waiting for saved");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn tracks(&self) -> Vec<String> {
        names_in(&self.root.join("tracks"))
    }
    fn import_tmp(&self) -> Vec<String> {
        names_in(&self.root.join("import-tmp"))
    }
    fn outside_ok(&self) {
        assert_eq!(fs::read_to_string(self.outside.join("precious.txt")).unwrap(), "do not touch");
        assert_eq!(names_in(&self.outside), ["precious.txt"]);
    }
}

fn kind_of(path: &Path) -> LocalKind {
    if path.extension().is_some_and(|e| e == "mp4") { LocalKind::Video } else { LocalKind::Audio }
}

fn flac_seconds(p: &Path) -> f64 {
    let r = claxon::FlacReader::open(p).unwrap();
    let si = r.streaminfo();
    si.samples.expect("samples in STREAMINFO") as f64 / si.sample_rate as f64
}

// ------------------------------------------------------------------ tests

#[test]
fn typed_path_forms_import_audio_and_video() {
    let server = RealServer::start("ok");
    for (fix, spaced, title, band) in [
        ("tagged.mp3", "My Song.mp3", "Glass Harbour", "The Example Band"),
        ("with-audio.mp4", "My Clip.mp4", "Clip Title", ""),
    ] {
        let rig = Rig::new();
        let plain = rig.put(fix, fix);
        let spaced_path = rig.put(fix, spaced);
        let forms = [
            ("plain", plain.display().to_string(), plain.clone()),
            ("quoted", format!("\"{}\"", spaced_path.display()), spaced_path.clone()),
            (
                "file-url",
                url::Url::from_file_path(&spaced_path).unwrap().to_string(),
                spaced_path.clone(),
            ),
        ];
        assert!(forms[2].1.contains("%20"), "{}", forms[2].1);
        let outside_before = fingerprint(&rig.outside);
        let mut seen = 0;
        for (label, typed, file) in forms {
            let before = fs::read(&file).unwrap();
            let resolved = resolve(&typed, kind_of(&file), None).unwrap_or_else(|e| panic!("{fix} {label}: {e:?}"));
            assert_eq!(resolved, file, "{fix} {label}");
            rig.start(&resolved).unwrap();
            let ready = rig.ready();
            let m = ready.metadata.clone().unwrap();
            rig.state.start_extraction(&ready.job, m, Some(&server.url), false).unwrap();
            let saved = rig.wait_saved();
            let t = saved.track.unwrap();
            seen += 1;

            let dirs = rig.tracks();
            assert_eq!(dirs.len(), seen, "{fix} {label}: {dirs:?}");
            assert!(dirs.contains(&t.id));
            let tdir = rig.root.join("tracks").join(&t.id);
            let j: serde_json::Value = serde_json::from_slice(&fs::read(tdir.join("track.json")).unwrap()).unwrap();
            assert_eq!(j["type"], "stem", "{fix} {label}");
            assert_eq!(j["schema_version"], 2);
            assert_eq!(j["title"], title, "{fix} {label}");
            if !band.is_empty() {
                assert_eq!(j["band"], band);
            }
            let listed = j["stems"].as_array().unwrap();
            assert_eq!(listed.len(), 6, "{fix} {label}");
            let flacs: Vec<String> = names_in(&tdir.join("stems")).into_iter().filter(|n| n.ends_with(".flac")).collect();
            assert_eq!(flacs.len(), 6, "{fix} {label}: {flacs:?}");
            assert_eq!(t.track_type, TrackType::Stem);

            assert_eq!(rig.import_tmp(), Vec::<String>::new(), "{fix} {label}");
            assert_eq!(fs::read(&file).unwrap(), before, "{fix} {label}: source changed");
            assert_eq!(fingerprint(&rig.outside), outside_before);
        }
        rig.outside_ok();
    }
}

#[test]
fn no_audio_video_resolves_but_the_job_fails() {
    let rig = Rig::new();
    let p = rig.put("no-audio.mp4", "no-audio.mp4");
    let resolved = resolve(&p.display().to_string(), LocalKind::Video, None).unwrap();
    rig.start(&resolved).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    assert_eq!(s.error.unwrap().message, "Selected file no-audio.mp4 has no audio track");
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    assert!(rig.tracks().is_empty());
}

#[test]
fn a_video_with_two_audio_streams_imports_the_first_one() {
    let rig = Rig::new();
    let p = rig.sources.join("two-audio.mp4");
    let st = Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=64x64:r=5:d=5"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"])
        .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=5"])
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", "mpeg4", "-c:a", "aac"])
        .arg(&p)
        .status()
        .unwrap();
    assert!(st.success());
    let resolved = resolve(&p.display().to_string(), LocalKind::Video, None).unwrap();
    rig.start(&resolved).unwrap();
    rig.ready();
    let dirs: Vec<String> = rig.import_tmp().into_iter().filter(|n| n.starts_with("file-")).collect();
    assert_eq!(dirs.len(), 1, "{:?}", rig.import_tmp());
    let secs = flac_seconds(&rig.root.join("import-tmp").join(&dirs[0]).join("audio.flac"));
    assert!((2.8..3.3).contains(&secs), "expected the 3 s stream, got {secs} s");
}

#[test]
fn rejected_paths_start_no_job_and_create_nothing() {
    let rig = Rig::new();
    let txt = rig.sources.join("notes.txt");
    fs::write(&txt, "x").unwrap();
    let dir = rig.sources.join("folder.mp3");
    fs::create_dir(&dir).unwrap();
    let before = fingerprint(&rig.root);
    let sources_before = fingerprint(&rig.sources);
    let cases = [
        "".to_string(),
        "   ".to_string(),
        "relative/song.mp3".to_string(),
        rig.sources.join("missing.mp3").display().to_string(),
        dir.display().to_string(),
        txt.display().to_string(),
        "file://".to_string(),
    ];
    for c in cases {
        assert!(resolve(&c, LocalKind::Audio, None).is_err(), "{c:?} was accepted");
    }
    assert!(!rig.state.is_active());
    assert!(rig.state.snapshot().is_none());
    assert_eq!(fingerprint(&rig.root), before);
    assert_eq!(fingerprint(&rig.sources), sources_before);
    rig.outside_ok();
}

// ------------------------------------------------------------------ gui-import-local-batch

use calliope_gui::ipc::{ImportKind, do_import_directory, do_import_path, do_pick_import_file, PickKind, RepoState};
use calliope_gui::picker::{DialogKind, ScriptedPicker};
use calliope_gui::settings::SettingsStore;

/// A `Rig` plus what the IPC functions need: a scripted picker (kept by handle), a settings
/// store in a temp file, a recording event sink and a home folder.
struct Ipc<'a> {
    rig: &'a Rig,
    store: SettingsStore,
    picker: Arc<ScriptedPicker>,
    st: RepoState,
    events: Arc<Mutex<Vec<ImportEvent>>>,
    sink: Sink,
    home: PathBuf,
    settings_file: PathBuf,
}

impl<'a> Ipc<'a> {
    fn new(rig: &'a Rig, answers: &str, server: Option<&RealServer>) -> Ipc<'a> {
        let answers_file = rig.dir.path().join("answers");
        fs::write(&answers_file, answers).unwrap();
        let settings_file = rig.dir.path().join("settings.json");
        let store = SettingsStore::open(settings_file.clone());
        if let Some(s) = server {
            store.set_edge_ai_url(&s.url).unwrap();
        }
        let picker = Arc::new(ScriptedPicker::new(answers_file));
        let st = RepoState::new(rig.root.clone(), Box::new(picker.clone()));
        let events = Arc::new(Mutex::new(Vec::<ImportEvent>::new()));
        let e2 = events.clone();
        let sink: Sink = Arc::new(move |ev| e2.lock().unwrap().push(ev));
        let home = rig.dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        Ipc { rig, store, picker, st, events, sink, home, settings_file }
    }
    fn browse(&self, kind: PickKind, typed: &str) -> Option<calliope_gui::ipc::PickedFile> {
        do_pick_import_file(&self.st, &self.store, kind, typed, Some(self.home.clone())).unwrap()
    }
    fn last_start(&self) -> (DialogKind, Option<PathBuf>) {
        self.picker.starts().pop().expect("a dialog was opened")
    }
    fn import_file(&self, path: &Path) -> Result<JobSnapshot, String> {
        do_import_path(&self.st, &self.rig.state, &self.store, ImportKind::Audio, &path.display().to_string(), Some(self.home.clone()), self.sink.clone())
    }
    fn import_dir(&self, path: &Path) -> Result<JobSnapshot, String> {
        do_import_directory(&self.st, &self.rig.state, &self.store, &path.display().to_string(), Some(self.home.clone()), self.sink.clone())
    }
    fn events(&self) -> Vec<ImportEvent> {
        self.events.lock().unwrap().clone()
    }
    fn batch_file_names(&self) -> Vec<String> {
        self.events().into_iter().filter_map(|e| if let ImportEvent::BatchFile { name, .. } = e { Some(name) } else { None }).collect()
    }
    fn wait_batch_over(&self) -> JobSnapshot {
        self.rig.idle()
    }
}

/// `<tmp>/music/rock/song.mp3` (tagged) and `<tmp>/music/jazz/`.
fn music_tree(rig: &Rig) -> (PathBuf, PathBuf) {
    let rock = rig.dir.path().join("music/rock");
    fs::create_dir_all(&rock).unwrap();
    fs::create_dir_all(rig.dir.path().join("music/jazz")).unwrap();
    let song = rock.join("song.mp3");
    fs::copy(fixture("tagged.mp3"), &song).unwrap();
    (rock, song)
}

#[test]
fn ac_browse_opens_the_typed_files_folder() {
    let rig = Rig::new();
    let (rock, song) = music_tree(&rig);
    let ipc = Ipc::new(&rig, "import-audio CANCEL\nimport-video CANCEL\n", None);
    assert!(ipc.browse(PickKind::Audio, &song.display().to_string()).is_none());
    assert_eq!(ipc.last_start(), (DialogKind::ImportAudio, Some(rock.clone())));
    // a quoted path works the same way, for the video dialog too
    ipc.browse(PickKind::Video, &format!("\"{}\"", song.display()));
    assert_eq!(ipc.last_start(), (DialogKind::ImportVideo, Some(rock)));
    assert_eq!(ipc.store.get().last_import_dir, None, "Browse alone remembers nothing");
}

#[test]
fn ac_browse_opens_the_typed_directory() {
    let rig = Rig::new();
    let (_, _) = music_tree(&rig);
    let jazz = rig.dir.path().join("music/jazz");
    let ipc = Ipc::new(&rig, "import-audio CANCEL\nimport-directory CANCEL\n", None);
    ipc.browse(PickKind::Audio, &jazz.display().to_string());
    assert_eq!(ipc.last_start(), (DialogKind::ImportAudio, Some(jazz.clone())));
    ipc.browse(PickKind::Directory, &jazz.display().to_string());
    assert_eq!(ipc.last_start(), (DialogKind::ImportDirectory, Some(jazz)));
}

#[test]
fn ac_browse_opens_the_last_location_also_after_restart() {
    let rig = Rig::new();
    let (rock, song) = music_tree(&rig);
    let answers = "import-audio CANCEL\nimport-video CANCEL\n";
    {
        let ipc = Ipc::new(&rig, answers, None);
        ipc.import_file(&song).unwrap();
        rig.ready();
        assert_eq!(ipc.store.get().last_import_dir, Some(rock.clone()));
    }
    // app restart: a new store reads the same file
    let ipc = Ipc::new(&rig, answers, None);
    assert_eq!(ipc.settings_file, rig.dir.path().join("settings.json"));
    assert_eq!(ipc.store.get().last_import_dir, Some(rock.clone()));
    ipc.browse(PickKind::Audio, "");
    assert_eq!(ipc.last_start(), (DialogKind::ImportAudio, Some(rock.clone())));
    ipc.browse(PickKind::Video, &rig.dir.path().join("music/nope.mp3").display().to_string());
    assert_eq!(ipc.last_start(), (DialogKind::ImportVideo, Some(rock)));
}

#[test]
fn ac_browse_opens_home_when_nothing_was_imported() {
    let rig = Rig::new();
    let ipc = Ipc::new(&rig, "import-audio CANCEL\nimport-directory CANCEL\n", None);
    ipc.browse(PickKind::Audio, "");
    assert_eq!(ipc.last_start(), (DialogKind::ImportAudio, Some(ipc.home.clone())));
    ipc.browse(PickKind::Directory, "relative/nothing");
    assert_eq!(ipc.last_start(), (DialogKind::ImportDirectory, Some(ipc.home.clone())));
}

#[test]
fn ac_directory_browse_uses_the_folder_dialog() {
    let rig = Rig::new();
    let (rock, _) = music_tree(&rig);
    let ipc = Ipc::new(&rig, &format!("import-directory {}\n", rock.display()), None);
    let picked = ipc.browse(PickKind::Directory, "").unwrap();
    assert_eq!(picked.path, rock.display().to_string());
    assert_eq!(picked.name, "rock");
    assert_eq!(ipc.last_start().0, DialogKind::ImportDirectory);
}

fn album(rig: &Rig) -> PathBuf {
    let dir = rig.dir.path().join("music/album");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("b.mp3")).unwrap();
    fs::copy(fixture("untagged.flac"), dir.join("a.flac")).unwrap();
    fs::write(dir.join("cover.jpg"), b"not an image").unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("sub/c.mp3")).unwrap();
    dir
}

#[test]
fn ac_directory_import_order_and_ignored_entries() {
    let server = RealServer::start("ok");
    let rig = Rig::new();
    let dir = album(&rig);
    let before = fingerprint(&dir);
    let ipc = Ipc::new(&rig, "", Some(&server));
    ipc.import_dir(&dir).unwrap();
    let done = ipc.wait_batch_over();
    assert_eq!(done.phase, Phase::Finished, "{:?}", done.error);
    assert_eq!(ipc.batch_file_names(), ["a.flac", "b.mp3"]);
    let items = done.batch.unwrap().items;
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| matches!(i.outcome, BatchOutcome::Saved { .. })));
    assert_eq!(rig.tracks().len(), 2);
    let titles: Vec<String> = rig
        .tracks()
        .iter()
        .map(|id| {
            let j: serde_json::Value = serde_json::from_slice(&fs::read(rig.root.join("tracks").join(id).join("track.json")).unwrap()).unwrap();
            assert_eq!(j["type"], "stem");
            j["title"].as_str().unwrap().to_string()
        })
        .collect();
    assert!(titles.contains(&"a".to_string()) && titles.contains(&"Glass Harbour".to_string()), "{titles:?}");
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    assert_eq!(fingerprint(&dir), before, "sources changed");
    assert_eq!(ipc.store.get().last_import_dir, Some(dir));
    rig.outside_ok();
}

#[test]
fn ac_directory_without_audio_files() {
    let rig = Rig::new();
    let dir = rig.dir.path().join("music/images");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("cover.jpg"), b"x").unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("sub/c.mp3")).unwrap();
    fs::copy(fixture("with-audio.mp4"), dir.join("clip.mp4")).unwrap();
    let ipc = Ipc::new(&rig, "", None);
    let before = fingerprint(&rig.root);
    assert_eq!(ipc.import_dir(&dir).unwrap_err(), format!("Directory {} has no audio files", dir.display()));
    assert!(!rig.state.is_active());
    assert!(rig.state.snapshot().is_none());
    assert!(ipc.events().is_empty());
    assert_eq!(fingerprint(&rig.root), before);
    assert_eq!(ipc.store.get().last_import_dir, None);
}

#[test]
fn ac_second_of_three_fails() {
    let server = RealServer::start("seq:ok,fail,ok");
    let rig = Rig::new();
    let dir = rig.dir.path().join("music/three");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(fixture("untagged.flac"), dir.join("1.flac")).unwrap();
    fs::copy(fixture("tagged.ogg"), dir.join("2.ogg")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("3.mp3")).unwrap();
    let ipc = Ipc::new(&rig, "", Some(&server));
    ipc.import_dir(&dir).unwrap();
    let done = ipc.wait_batch_over();
    assert_eq!(done.phase, Phase::Finished);
    let items = done.batch.unwrap().items;
    assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["1.flac", "2.ogg", "3.mp3"]);
    assert!(matches!(items[0].outcome, BatchOutcome::Saved { .. }));
    assert!(matches!(items[2].outcome, BatchOutcome::Saved { .. }));
    match &items[1].outcome {
        BatchOutcome::Failed { stage, message } => {
            assert_eq!(*stage, Stage::Server);
            assert!(!message.is_empty());
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(rig.tracks().len(), 2);
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    let finished: Vec<_> = ipc.events().into_iter().filter(|e| matches!(e, ImportEvent::Finished { .. })).collect();
    assert_eq!(finished.len(), 1);
}

#[test]
fn ac_cancel_on_file_two_of_three() {
    let server = RealServer::start("seq:ok,hang");
    let rig = Rig::new();
    let dir = rig.dir.path().join("music/three");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(fixture("untagged.flac"), dir.join("1.flac")).unwrap();
    fs::copy(fixture("tagged.ogg"), dir.join("2.ogg")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("3.mp3")).unwrap();
    let ipc = Ipc::new(&rig, "", Some(&server));
    let snap = ipc.import_dir(&dir).unwrap();
    until("file 2 working", 60, || {
        rig.state.snapshot().is_some_and(|s| s.phase == Phase::Working && s.batch.is_some_and(|b| b.current == Some(1)))
    });
    rig.state.cancel(&snap.job).unwrap();
    let s = ipc.wait_batch_over();
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(!rig.state.is_active(), "idle again");
    let b = s.batch.unwrap();
    assert_eq!((b.current, b.items.len()), (None, 1));
    assert_eq!(rig.tracks().len(), 1);
    assert_eq!(ipc.batch_file_names(), ["1.flac", "2.ogg"], "file 3 never started");
    until("the server job to be deleted", 10, || server.job_dirs().is_empty());
    assert!(server.count("method=DELETE") >= 2);
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
}

#[test]
fn ac_progress_names_the_current_file() {
    let server = RealServer::start("seq:ok,hang");
    let rig = Rig::new();
    let dir = album(&rig);
    let ipc = Ipc::new(&rig, "", Some(&server));
    let snap = ipc.import_dir(&dir).unwrap();
    assert_eq!(snap.source.label, "album");
    until("file 2 working", 60, || {
        rig.state.snapshot().is_some_and(|s| s.phase == Phase::Working && s.batch.is_some_and(|b| b.current == Some(1)))
    });
    let s = rig.state.snapshot().unwrap();
    let b = s.batch.unwrap();
    assert_eq!((b.current, b.files.len(), b.files[1].as_str()), (Some(1), 2, "b.mp3"));
    assert_eq!(b.items.len(), 1);
    assert_eq!(s.phase, Phase::Working);
    rig.state.cancel(&snap.job).unwrap();
    ipc.wait_batch_over();
}

// ------------------------------------------------------------------ QA additions

/// Browse with a `~` path, a relative path and a remembered folder that was deleted: no error,
/// silent fall-back, and the stale setting is forgotten (also on disk).
#[test]
fn ac_browse_tilde_relative_and_deleted_last_folder() {
    let rig = Rig::new();
    let (rock, song) = music_tree(&rig);
    let answers = "import-audio CANCEL\n".repeat(5);
    let ipc = Ipc::new(&rig, &answers, None);
    // `~/..` is understood like for Extract
    fs::create_dir_all(ipc.home.join("Songs")).unwrap();
    fs::copy(fixture("tagged.mp3"), ipc.home.join("Songs/t.mp3")).unwrap();
    ipc.browse(PickKind::Audio, "~/Songs/t.mp3");
    assert_eq!(ipc.last_start(), (DialogKind::ImportAudio, Some(ipc.home.join("Songs"))));
    ipc.browse(PickKind::Audio, "~/Songs");
    assert_eq!(ipc.last_start().1, Some(ipc.home.join("Songs")));
    // accepted import remembers rock; a relative typed path falls back to it
    ipc.import_file(&song).unwrap();
    rig.ready();
    ipc.browse(PickKind::Audio, "music/rock/song.mp3");
    assert_eq!(ipc.last_start().1, Some(rock.clone()), "relative paths are not resolved");
    // the folder disappears: home is used, the setting is forgotten, nothing is an error
    fs::remove_dir_all(&rock).unwrap();
    assert!(ipc.browse(PickKind::Audio, "").is_none());
    assert_eq!(ipc.last_start().1, Some(ipc.home.clone()));
    assert_eq!(ipc.store.get().last_import_dir, None);
    let on_disk: serde_json::Value = serde_json::from_slice(&fs::read(&ipc.settings_file).unwrap()).unwrap();
    assert!(on_disk["last_import_dir"].is_null(), "{on_disk}");
}

/// Directory with spaces in its path, typed with `~`; upper/mixed-case extensions; hidden
/// files and video files skipped; names sorted case-insensitively.
#[test]
fn ac_directory_with_spaces_uppercase_extensions_and_hidden_files() {
    let server = RealServer::start("ok");
    let rig = Rig::new();
    let ipc = Ipc::new(&rig, "", Some(&server));
    let dir = ipc.home.join("My Music/Rock Album");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(fixture("untagged.flac"), dir.join("Song One.FLAC")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("B.Mp3")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("._B.mp3")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join(".hidden.mp3")).unwrap();
    fs::copy(fixture("with-audio.mp4"), dir.join("clip.MP4")).unwrap();
    let before = fingerprint(&dir);
    let typed = "~/My Music/Rock Album";
    do_import_directory(&ipc.st, &rig.state, &ipc.store, typed, Some(ipc.home.clone()), ipc.sink.clone()).unwrap();
    let done = ipc.wait_batch_over();
    assert_eq!(done.phase, Phase::Finished, "{:?}", done.error);
    assert_eq!(ipc.batch_file_names(), ["B.Mp3", "Song One.FLAC"]);
    assert_eq!(done.batch.as_ref().unwrap().dir, dir.display().to_string());
    assert_eq!(rig.tracks().len(), 2);
    assert_eq!(ipc.store.get().last_import_dir, Some(dir.clone()));
    assert_eq!(fingerprint(&dir), before, "sources changed");
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
}

/// A directory of only hidden / uppercase-non-audio files has no audio; bad paths are
/// rejected with the exact texts and never change the remembered folder.
#[test]
fn ac_directory_rejections_do_not_touch_the_remembered_folder() {
    let rig = Rig::new();
    let (rock, song) = music_tree(&rig);
    let ipc = Ipc::new(&rig, "", None);
    ipc.store.set_last_import_dir(Some(rock.clone())).unwrap();
    let hidden = rig.dir.path().join("music/hidden");
    fs::create_dir_all(&hidden).unwrap();
    fs::copy(fixture("tagged.mp3"), hidden.join(".song.mp3")).unwrap();
    fs::write(hidden.join("README.TXT"), b"x").unwrap();
    assert_eq!(ipc.import_dir(&hidden).unwrap_err(), format!("Directory {} has no audio files", hidden.display()));
    let missing = rig.dir.path().join("music/nope");
    assert_eq!(ipc.import_dir(&missing).unwrap_err(), format!("No folder found at {}", missing.display()));
    assert_eq!(ipc.import_dir(&song).unwrap_err(), format!("{} is not a folder", song.display()));
    assert_eq!(ipc.import_dir(Path::new("")).unwrap_err(), "Enter the path of a folder or choose one with Browse");
    assert!(ipc.import_dir(Path::new("music/rock")).is_err(), "relative");
    assert_eq!(ipc.store.get().last_import_dir, Some(rock));
    assert!(rig.state.snapshot().is_none());
    assert!(ipc.events().is_empty());
}

/// Cancel while the LAST file works: the first two are kept; a second directory import while
/// the batch runs is refused as busy and is not remembered.
#[test]
fn ac_cancel_on_the_last_file_and_busy_refusal() {
    let server = RealServer::start("seq:ok,ok,hang");
    let rig = Rig::new();
    let dir = rig.dir.path().join("music/three");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(fixture("untagged.flac"), dir.join("1.flac")).unwrap();
    fs::copy(fixture("tagged.ogg"), dir.join("2.ogg")).unwrap();
    fs::copy(fixture("tagged.mp3"), dir.join("3.mp3")).unwrap();
    let other = album(&rig);
    let ipc = Ipc::new(&rig, "", Some(&server));
    let snap = ipc.import_dir(&dir).unwrap();
    until("file 3 working", 90, || {
        rig.state.snapshot().is_some_and(|s| s.phase == Phase::Working && s.batch.is_some_and(|b| b.current == Some(2)))
    });
    assert_eq!(ipc.import_dir(&other).unwrap_err(), "An import is already running");
    assert_eq!(ipc.store.get().last_import_dir, Some(dir.clone()), "a refused import is not remembered");
    rig.state.cancel(&snap.job).unwrap();
    let s = ipc.wait_batch_over();
    assert_eq!(s.phase, Phase::Cancelled);
    let b = s.batch.unwrap();
    assert_eq!((b.current, b.items.len(), b.files.len()), (None, 2, 3));
    assert!(b.items.iter().all(|i| matches!(i.outcome, BatchOutcome::Saved { .. })));
    assert_eq!(rig.tracks().len(), 2);
    assert!(ipc.events().iter().any(|e| matches!(e, ImportEvent::Cancelled { .. })));
    assert!(!ipc.events().iter().any(|e| matches!(e, ImportEvent::Finished { .. })));
    until("the server job to be deleted", 10, || server.job_dirs().is_empty());
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    // the slot is free again
    assert!(!rig.state.is_active());
}

/// Requirement 10: started / saved / failed / cancelled lines for every file, and the batch
/// totals, are written to the log. The scenario runs in a child copy of this test binary so
/// its stderr can be captured.
#[test]
fn ac_batch_log_lines_for_started_saved_failed_cancelled() {
    if std::env::var("ACQ_BATCH_LOG_CHILD").is_ok() {
        let server = RealServer::start("seq:ok,fail,hang");
        let rig = Rig::new();
        let dir = rig.dir.path().join("music/three");
        fs::create_dir_all(&dir).unwrap();
        fs::copy(fixture("untagged.flac"), dir.join("1.flac")).unwrap();
        fs::copy(fixture("tagged.ogg"), dir.join("2.ogg")).unwrap();
        fs::copy(fixture("tagged.mp3"), dir.join("3.mp3")).unwrap();
        let ipc = Ipc::new(&rig, "", Some(&server));
        let snap = ipc.import_dir(&dir).unwrap();
        until("file 3 working", 90, || {
            rig.state.snapshot().is_some_and(|s| s.phase == Phase::Working && s.batch.is_some_and(|b| b.current == Some(2)))
        });
        rig.state.cancel(&snap.job).unwrap();
        ipc.wait_batch_over();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "ac_batch_log_lines_for_started_saved_failed_cancelled", "--nocapture", "--test-threads=1"])
        .env("ACQ_BATCH_LOG_CHILD", "1")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "child failed:\n{err}");
    let has = |needle: &[&str]| err.lines().any(|l| l.contains("calliope: import batch") && needle.iter().all(|n| l.contains(n)));
    assert!(has(&["files=3"]), "{err}");
    for (i, n) in [(1, "1.flac"), (2, "2.ogg"), (3, "3.mp3")] {
        assert!(has(&[&format!("file={i}/3 name={n} result=started")]), "started {n}\n{err}");
    }
    assert!(has(&["file=1/3 name=1.flac result=saved id="]), "{err}");
    assert!(has(&["file=2/3 name=2.ogg result=failed stage=server message="]), "{err}");
    assert!(has(&["file=3/3 name=3.mp3 result=cancelled"]), "{err}");
    assert!(has(&["cancelled saved=1 skipped=1"]), "{err}");
}
