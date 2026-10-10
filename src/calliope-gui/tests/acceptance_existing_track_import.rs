//! Headless acceptance for gui-existing-track-import: a typed path (plain, quoted, file://) goes
//! through `local_source::resolve` -> `start_file` -> ready -> `start_extraction` -> saved, with the
//! REAL `calliope-stems` on 127.0.0.1 (stub separator, `ok` mode) and the local ffmpeg/ffprobe on
//! fixtures copied into a temp dir. User files and a sibling "outside" folder must never change.
#![allow(dead_code, unused_imports, clippy::all)]

#[path = "../src/fsutil.rs"]
mod fsutil;
#[path = "../src/track_meta.rs"]
mod track_meta;
#[path = "../src/repository.rs"]
mod repository;
#[path = "../src/import_tmp.rs"]
mod import_tmp;
#[path = "../src/media.rs"]
mod media;
#[path = "../src/download.rs"]
mod download;
#[path = "../src/tools.rs"]
mod tools;
#[path = "../src/stem_audio.rs"]
mod stem_audio;
#[path = "../src/import_job.rs"]
mod import_job;
#[path = "../src/local_source.rs"]
mod local_source;

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
