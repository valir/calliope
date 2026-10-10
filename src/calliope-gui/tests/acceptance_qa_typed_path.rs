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

// ================================================================== QA tests

fn msg(r: Result<PathBuf, local_source::PathProblem>) -> (String, String) {
    let e = r.unwrap_err();
    (e.reason.to_string(), e.message)
}

#[test]
fn qa_ac1_ac2_home_tilde_and_symlink_paths_import_stems() {
    let server = RealServer::start("ok");
    for (fix, name) in [("tagged.mp3", "qa song.mp3"), ("with-audio.mp4", "qa clip.mp4")] {
        let rig = Rig::new();
        let home = rig.dir.path().join("home");
        fs::create_dir_all(home.join("Music")).unwrap();
        fs::copy(fixture(fix), home.join("Music").join(name)).unwrap();
        let real = home.join("Music").join(name);
        let link = rig.sources.join(format!("link-{name}"));
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let before = fs::read(&real).unwrap();
        let k = kind_of(&real);
        for typed in [
            format!("~/Music/{name}"),
            format!("  '~/Music/{name}'  "),
            link.display().to_string(),
        ] {
            let p = resolve(&typed, k, Some(&home)).unwrap_or_else(|e| panic!("{typed}: {e:?}"));
            rig.start(&p).unwrap();
            let r = rig.ready();
            let m = r.metadata.clone().unwrap();
            rig.state.start_extraction(&r.job, m, Some(&server.url), false).unwrap();
            let saved = rig.wait_saved();
            let t = saved.track.unwrap();
            let tdir = rig.root.join("tracks").join(&t.id);
            let j: serde_json::Value = serde_json::from_slice(&fs::read(tdir.join("track.json")).unwrap()).unwrap();
            assert_eq!(j["type"], "stem");
            assert_eq!(j["stems"].as_array().unwrap().len(), 6);
            assert_eq!(names_in(&tdir.join("stems")).iter().filter(|n| n.ends_with(".flac")).count(), 6);
            assert!(rig.import_tmp().is_empty());
        }
        assert_eq!(rig.tracks().len(), 3);
        assert_eq!(fs::read(&real).unwrap(), before);
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        rig.outside_ok();
    }
}

#[test]
fn qa_exact_rejection_messages() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let home = p.join("h");
    fs::create_dir(&home).unwrap();
    fs::write(p.join("a.mp3"), b"x").unwrap();
    fs::write(p.join("v.mp4"), b"x").unwrap();
    fs::write(p.join("n.txt"), b"x").unwrap();
    fs::create_dir(p.join("dir")).unwrap();
    fs::create_dir(p.join("x.mp3")).unwrap();
    let empty = "Enter the path of a file or choose one with Browse";
    assert_eq!(msg(resolve("", LocalKind::Audio, None)), ("empty".into(), empty.into()));
    assert_eq!(msg(resolve(" \t ", LocalKind::Video, None)).1, empty);
    let inv = ("invalid".to_string(), "The path is not valid".to_string());
    assert_eq!(msg(resolve("/tmp/a\nb.mp3", LocalKind::Audio, None)), inv);
    assert_eq!(msg(resolve("/tmp/a\0b.mp3", LocalKind::Audio, None)), inv);
    assert_eq!(msg(resolve("/tmp/a\tb.mp3", LocalKind::Audio, None)), inv);
    assert_eq!(msg(resolve(&format!("/{}.mp3", "a".repeat(5000)), LocalKind::Audio, None)), inv);
    assert_eq!(msg(resolve("file://otherhost/x.mp3", LocalKind::Audio, None)), inv);
    assert_eq!(msg(resolve("\"\"", LocalKind::Audio, None)).0.len() > 0, true);
    let rel = ("relative".to_string(), "Enter the full path, starting with / or ~/".to_string());
    for t in ["song.mp3", "./song.mp3", "../song.mp3", "~bob/x.mp3", "dir/a.mp3", "mp3"] {
        assert_eq!(msg(resolve(t, LocalKind::Audio, Some(&home))), rel, "{t}");
    }
    assert_eq!(msg(resolve("~/a.mp3", LocalKind::Audio, None)), rel);
    let miss = p.join("nope.mp3");
    assert_eq!(
        msg(resolve(&miss.display().to_string(), LocalKind::Audio, None)),
        ("missing".into(), format!("No file found at {}", miss.display()))
    );
    // typo in a wrong-type name -> missing wins over extension
    assert_eq!(msg(resolve(&p.join("nope.txt").display().to_string(), LocalKind::Audio, None)).0, "missing");
    let dd = p.join("dir");
    assert_eq!(
        msg(resolve(&dd.display().to_string(), LocalKind::Audio, None)),
        ("folder".into(), format!("{} is a folder, not a file", dd.display()))
    );
    assert_eq!(msg(resolve(&p.join("x.mp3").display().to_string(), LocalKind::Audio, None)).0, "folder");
    assert_eq!(
        msg(resolve(&p.join("n.txt").display().to_string(), LocalKind::Audio, None)),
        ("extension".into(), "\"n.txt\" is not an audio file".into())
    );
    assert_eq!(msg(resolve(&p.join("a.mp3").display().to_string(), LocalKind::Video, None)).1, "\"a.mp3\" is not a video file");
    assert_eq!(msg(resolve(&p.join("v.mp4").display().to_string(), LocalKind::Audio, None)).1, "\"v.mp4\" is not an audio file");
    // dangling symlink
    let dl = p.join("dangling.mp3");
    std::os::unix::fs::symlink(p.join("gone"), &dl).unwrap();
    assert_eq!(msg(resolve(&dl.display().to_string(), LocalKind::Audio, None)).0, "missing");
    // unreadable
    let un = p.join("secret.mp3");
    fs::write(&un, b"x").unwrap();
    fs::set_permissions(&un, fs::Permissions::from_mode(0)).unwrap();
    if fs::File::open(&un).is_err() {
        assert_eq!(
            msg(resolve(&un.display().to_string(), LocalKind::Audio, None)),
            ("unreadable".into(), format!("Cannot read {} (permission denied)", un.display()))
        );
    }
    // valid, uppercase extension
    fs::write(p.join("UP.MP3"), b"x").unwrap();
    assert!(resolve(&p.join("UP.MP3").display().to_string(), LocalKind::Audio, None).is_ok());
    // file:// with %20 and localhost
    fs::write(p.join("a b.mp3"), b"x").unwrap();
    assert_eq!(resolve(&format!("file://{}/a%20b.mp3", p.display()), LocalKind::Audio, None).unwrap(), p.join("a b.mp3"));
    assert_eq!(resolve(&format!("file://localhost{}/a%20b.mp3", p.display()), LocalKind::Audio, None).unwrap(), p.join("a b.mp3"));
    assert_eq!(resolve(&format!("'{}'", p.join("a b.mp3").display()), LocalKind::Audio, None).unwrap(), p.join("a b.mp3"));
}

#[test]
fn qa_security_fifo_dev_and_hostile_inputs_do_not_hang_or_write() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    let fifo = p.join("pipe.mp3");
    assert!(Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let before = names_in(p);
    let t = Instant::now();
    let r = resolve(&fifo.display().to_string(), LocalKind::Audio, None);
    assert!(t.elapsed() < Duration::from_secs(2), "FIFO resolve blocked");
    assert_eq!(msg(r).0, "not-file");
    // symlink to a FIFO
    let lf = p.join("lpipe.mp3");
    std::os::unix::fs::symlink(&fifo, &lf).unwrap();
    let t = Instant::now();
    assert_eq!(msg(resolve(&lf.display().to_string(), LocalKind::Audio, None)).0, "not-file");
    assert!(t.elapsed() < Duration::from_secs(2));
    // device
    assert_eq!(msg(resolve("/dev/null", LocalKind::Audio, None)).0, "not-file");
    // symlink loop
    let a = p.join("loop-a.mp3");
    let b = p.join("loop-b.mp3");
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();
    let r = msg(resolve(&a.display().to_string(), LocalKind::Audio, None));
    assert!(r.0 == "unreadable" || r.0 == "missing", "{r:?}");
    // huge / odd inputs
    for s in ["a".repeat(1_000_000), format!("/{}", "x/".repeat(100_000)), "~".repeat(10000), "file:".repeat(2000), "\u{202e}/tmp/x.mp3".into(), "/".into(), "~".into(), "file:///".into()] {
        let t = Instant::now();
        assert!(resolve(&s, LocalKind::Audio, Some(p)).is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
    }
    // long single component (ENAMETOOLONG) is a clean error
    let r = resolve(&format!("/{}.mp3", "a".repeat(300)), LocalKind::Audio, None);
    assert!(r.is_err());
    // nothing was created or changed
    assert_eq!(names_in(p), { let mut b = before; b.extend(["lpipe.mp3".to_string(),"loop-a.mp3".into(),"loop-b.mp3".into()]); b.sort(); b });
}

#[test]
fn qa_state_start_file_failures_leave_no_job_and_no_files() {
    let rig = Rig::new();
    let before = fingerprint(&rig.root);
    // resolve rejects: we never call start_file
    for t in ["", "x.mp3", "/nonexistent/z.mp3"] {
        assert!(resolve(t, LocalKind::Audio, None).is_err());
    }
    assert!(rig.state.snapshot().is_none() && !rig.state.is_active());
    assert_eq!(fingerprint(&rig.root), before);
    // audio extension but not audio content: job fails with today's text, tmp cleaned
    let p = rig.put("not-audio.mp3", "not-audio.mp3");
    let ok = resolve(&p.display().to_string(), LocalKind::Audio, None).unwrap();
    rig.start(&ok).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    assert!(rig.tracks().is_empty());
    assert!(rig.import_tmp().is_empty(), "{:?}", rig.import_tmp());
}

#[test]
fn qa_req3_video_without_audio_and_first_stream() {
    let rig = Rig::new();
    let p = rig.put("no-audio.mp4", "silent.mp4");
    rig.start(&resolve(&p.display().to_string(), LocalKind::Video, None).unwrap()).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    assert_eq!(s.error.unwrap().message, "Selected file silent.mp4 has no audio track");
    assert!(rig.tracks().is_empty() && rig.import_tmp().is_empty());
}

/// Defect probe: a percent-encoded control character in a file:// link is decoded after the
/// control-character check, so it is not reported as "The path is not valid".
#[test]
fn qa_percent_encoded_control_chars_in_file_urls_are_invalid() {
    for t in ["file:///tmp/a%00b.mp3", "file:///tmp/a%0Ab.mp3"] {
        assert_eq!(msg(resolve(t, LocalKind::Audio, None)), ("invalid".into(), "The path is not valid".into()), "{t}");
    }
}
