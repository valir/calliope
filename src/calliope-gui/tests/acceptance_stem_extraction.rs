//! QA acceptance, data-safety and security probes for gui-stem-extraction (Rust side, headless).
//!
//! The crate is a binary, so the pure modules are compiled into this test crate with `#[path]`
//! (their own unit tests run here a second time; that is the price, and the precedent of
//! `acceptance_tracks_repository.rs`). Everything runs on temp repositories under the system
//! temp dir, with the REAL `calliope-stems` binary on 127.0.0.1 and the stub separator, the
//! fake yt-dlp (refuses every host that is not `.example`) and the local ffmpeg/ffprobe on
//! fixtures. A sibling "outside" folder and the user's source files must never change. No test
//! reaches the internet, the LAN or the real model.
//!
//! Crash tests re-run this test binary as a child (`qa_child`) and SIGKILL it mid-import.
#![allow(dead_code, unused_imports, unused_variables, clippy::type_complexity, clippy::all)]

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

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use import_job::*;
use repository::Repository;
use tools::Tools;
use track_meta::{TrackEdits, TrackType};

// ------------------------------------------------------------------ helpers

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

/// Knobs of the scripted (malicious) edge-AI server.
#[derive(Clone)]
struct AiConfig {
    job_id: String,
    stems: Vec<String>,
    /// stem name -> body
    body: Arc<dyn Fn(&str) -> Vec<u8> + Send + Sync>,
    /// seconds each stem body takes (dribbled in 1 KiB chunks)
    slow_stem_ms: u64,
    max_upload: u64,
    final_state: &'static str,
    error: Option<String>,
    progress: serde_json::Value,
}

impl AiConfig {
    fn good() -> AiConfig {
        AiConfig {
            job_id: "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b".into(),
            stems: ["vocals", "drums", "bass", "guitar", "piano", "other"].iter().map(|s| s.to_string()).collect(),
            body: Arc::new(|n| fs::read(fixture(&format!("stems/{n}.flac"))).unwrap_or_else(|_| b"fLaC-generic-body".to_vec())),
            slow_stem_ms: 0,
            max_upload: 300 << 20,
            final_state: "done",
            error: None,
            progress: serde_json::json!(1.0),
        }
    }
}

struct FakeAi {
    addr: SocketAddr,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for FakeAi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

impl FakeAi {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    fn saw(&self, needle: &str) -> bool {
        self.hits.lock().unwrap().iter().any(|l| l.contains(needle))
    }
}

fn fake_ai(cfg: AiConfig) -> FakeAi {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    let hits = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (h2, s2) = (hits.clone(), stop.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            if s2.load(Ordering::SeqCst) {
                return;
            }
            let Ok(mut s) = conn else { continue };
            let (cfg, hits) = (cfg.clone(), h2.clone());
            std::thread::spawn(move || {
                s.set_read_timeout(Some(Duration::from_secs(30))).ok();
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(1) => head.push(b[0]),
                        _ => return,
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let line = text.split("\r\n").next().unwrap_or("").to_string();
                hits.lock().unwrap().push(line.clone());
                let clen: usize = text
                    .lines()
                    .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                    .unwrap_or(0);
                let send = |s: &mut TcpStream, status: &str, ctype: &str, body: &[u8]| {
                    let h = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(h.as_bytes());
                    let _ = s.write_all(body);
                };
                let json = |s: &mut TcpStream, status: &str, v: serde_json::Value| send(s, status, "application/json", v.to_string().as_bytes());
                if line.starts_with("GET /v1/health") {
                    json(&mut s, "200 OK", serde_json::json!({"service":"calliope-stems","api":1,"version":"x","models":["htdemucs_6s"],
                        "default_model":"htdemucs_6s","busy":false,"max_duration_s":900,"max_upload_bytes":cfg.max_upload}));
                } else if line.starts_with("POST /v1/jobs") {
                    let mut left = clen;
                    let mut buf = vec![0u8; 65536];
                    while left > 0 {
                        match s.read(&mut buf[..left.min(65536)]) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => left -= n,
                        }
                    }
                    json(&mut s, "202 Accepted", serde_json::json!({"job": cfg.job_id, "state": "queued"}));
                } else if line.starts_with("DELETE") {
                    let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                } else if line.contains("/stems/") {
                    let name = line.split("/stems/").nth(1).and_then(|r| r.split(' ').next()).unwrap_or("");
                    let body = (cfg.body)(name);
                    let h = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(h.as_bytes());
                    for chunk in body.chunks(1024) {
                        if s.write_all(chunk).is_err() {
                            return;
                        }
                        if cfg.slow_stem_ms > 0 {
                            std::thread::sleep(Duration::from_millis(cfg.slow_stem_ms));
                        }
                    }
                } else if line.starts_with("GET /v1/jobs/") {
                    json(&mut s, "200 OK", serde_json::json!({"job": cfg.job_id, "state": cfg.final_state, "progress": cfg.progress,
                        "stems": if cfg.final_state == "done" { serde_json::json!(cfg.stems) } else { serde_json::Value::Null },
                        "error": cfg.error}));
                } else {
                    json(&mut s, "404 Not Found", serde_json::json!({"error":"x"}));
                }
            });
        }
    });
    FakeAi { addr, hits, stop }
}

// ------------------------------------------------------------------ rig

struct Rig {
    dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    sources: PathBuf,
    state: ImportState,
    tools: Tools,
    events: Arc<Mutex<Vec<ImportEvent>>>,
    sink: Sink,
    ytlog: PathBuf,
}

impl Rig {
    /// `v1`: the repository starts as a copy of tests/fixtures/library-sample (schema 1 tracks).
    fn new(v1: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let outside = dir.path().join("outside");
        let sources = dir.path().join("sources");
        for d in [&outside, &sources] {
            fs::create_dir_all(d).unwrap();
        }
        if v1 {
            copy_dir(&project().join("tests/fixtures/library-sample"), &root);
        } else {
            fs::create_dir_all(&root).unwrap();
            Repository::new(&root).ensure_layout().unwrap();
        }
        fs::write(outside.join("precious.txt"), "do not touch").unwrap();
        for f in ["tagged.mp3", "tagged.ogg", "with-audio.mp4", "no-audio.mp4", "long.flac", "untagged.flac", "not-audio.mp3"] {
            fs::copy(fixture(f), sources.join(f)).unwrap();
        }
        // wrapper so that each rig has its own fake-yt-dlp log
        let ytlog = dir.path().join("ytdlp.log");
        let wrapper = dir.path().join("yt-dlp");
        fs::write(
            &wrapper,
            format!("#!/bin/sh\nexport FAKE_YTDLP_LOG='{}'\nexec python3 '{}' \"$@\"\n", ytlog.display(), project().join("tests/support/bin/yt-dlp").display()),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let tools = Tools { yt_dlp: Some(wrapper), ffmpeg: Some(tool("ffmpeg")), ffprobe: Some(tool("ffprobe")) };
        let repo_lock = Arc::new(Mutex::new(()));
        let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
            let _g = repo_lock.lock().unwrap_or_else(|e| e.into_inner());
            f()
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let e2 = events.clone();
        let sink: Sink = Arc::new(move |ev| e2.lock().unwrap().push(ev));
        let state = ImportState::new(tools.clone(), lock, Duration::from_millis(40));
        Rig { dir, root, outside, sources, state, tools, events, sink, ytlog }
    }

    fn needle(&self) -> String {
        self.dir.path().display().to_string()
    }

    fn url(&self, url: &str, resume: bool) -> Result<JobSnapshot, String> {
        self.state.start_url(&self.root, url, resume, self.sink.clone())
    }
    fn file_at(&self, path: &Path) -> Result<JobSnapshot, String> {
        let kind = if path.extension().is_some_and(|e| e == "mp4") { FileKind::Video } else { FileKind::Audio };
        self.state.start_file(&self.root, kind, path, self.sink.clone())
    }
    fn file(&self, name: &str) -> Result<JobSnapshot, String> {
        self.file_at(&self.sources.join(name))
    }

    fn wait(&self, what: &str, pred: impl Fn(&JobSnapshot) -> bool) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(s) = self.state.snapshot() {
                if pred(&s) {
                    return s;
                }
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; at {:?}", self.state.snapshot().map(|s| (s.phase, s.error)));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn idle(&self) -> JobSnapshot {
        self.wait("idle", |_| !self.state.is_running())
    }
    fn ready(&self) -> JobSnapshot {
        let s = self.idle();
        assert_eq!(s.phase, Phase::Ready, "{:?}", s.error);
        s
    }
    fn extract(&self, s: &JobSnapshot, server_url: &str, keep: bool) -> Result<JobSnapshot, String> {
        self.state.start_extraction(&s.job, s.metadata.clone().unwrap(), Some(server_url), keep)
    }
    fn tracks(&self) -> Vec<String> {
        names_in(&self.root.join("tracks"))
    }
    fn staging(&self) -> Vec<String> {
        self.tracks().into_iter().filter(|n| n.starts_with(".staging")).collect()
    }
    fn import_tmp(&self) -> Vec<String> {
        names_in(&self.root.join("import-tmp"))
    }
    fn ytdlp_runs(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(&self.ytlog)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
    fn outside_ok(&self) {
        assert_eq!(fs::read_to_string(self.outside.join("precious.txt")).unwrap(), "do not touch");
        assert_eq!(names_in(&self.outside), ["precious.txt"]);
    }
    fn no_stray_processes(&self) {
        until("no process mentioning the rig remains", 10, || {
            procs_mentioning(&self.needle()).iter().all(|(_, c)| c.contains("calliope-stems") && false)
        });
    }
}

const WATCH: &str = "https://media.example/watch?v=ok";

/// True when some `*.part` file below import-tmp holds at least one byte.
fn part_has_bytes(rig: &Rig) -> bool {
    rig.import_tmp().iter().any(|d| {
        let dir = rig.root.join("import-tmp").join(d);
        names_in(&dir).iter().any(|n| n.ends_with(".part") && fs::metadata(dir.join(n)).map(|m| m.len() > 0).unwrap_or(false))
    })
}

fn v1_ids() -> Vec<String> {
    names_in(&project().join("tests/fixtures/library-sample/tracks"))
}

// ====================================================================== data safety

#[test]
fn v1_repository_is_never_rewritten_by_scan_prepare_import_cancel_or_discard() {
    let server = RealServer::start("ok");
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let src_before = fingerprint(&rig.sources);

    // scan / list
    let lib = Repository::new(&rig.root).scan();
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(lib.tracks.len(), v1_ids().len());
    assert!(lib.tracks.iter().all(|t| t.track_type == TrackType::Backing));
    assert_eq!(fingerprint(&rig.root), before, "scan changed the repository");

    // prepare_url is a pure question
    let p = rig.state.prepare_url(&rig.root, WATCH);
    // 
    let mut now = fingerprint(&rig.root);
    now.remove("import-tmp");
    assert_eq!(now, before, "prepare_url changed something: {:?}", p.status);

    // file import -> discard
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    rig.state.discard(&ready.job).unwrap();
    // url import -> cancel mid-way -> discard of nothing
    rig.url(WATCH, false).unwrap();
    let r = rig.ready();
    rig.state.discard(&r.job).unwrap();

    // full import, cancelled during the server step, then completed
    let server_hang = RealServer::start("hang");
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server_hang.url, false).unwrap();
    rig.wait("working", |s| s.phase == Phase::Working);
    rig.state.cancel(&ready.job).unwrap();
    rig.wait("back to ready", |s| s.phase == Phase::Ready && !rig.state.is_running());
    assert!(rig.staging().is_empty(), "staging left after a cancel");
    rig.extract(&ready, &server.url, true).unwrap();
    let done = rig.wait("saved", |s| s.phase == Phase::Saved);
    let new_id = done.track.unwrap().id;

    // every pre-existing file is byte-identical with the same mtime; only the new track was added
    let after = fingerprint(&rig.root);
    for (k, v) in &before {
        assert_eq!(after.get(k), Some(v), "pre-existing entry changed or vanished: {k}");
    }
    for k in after.keys() {
        if before.contains_key(k) {
            continue;
        }
        let ok = k == "import-tmp" || k.starts_with(&format!("tracks/{new_id}"));
        assert!(ok, "unexpected new entry: {k}");
    }
    assert_eq!(fingerprint(&rig.sources), src_before, "a source file changed");
    rig.outside_ok();
    // The v1 tracks are still schema 1 on disk.
    for id in v1_ids() {
        let j: serde_json::Value = serde_json::from_slice(&fs::read(rig.root.join("tracks").join(&id).join("track.json")).unwrap()).unwrap();
        assert_eq!(j["schema_version"], 1, "{id} was rewritten");
    }
    assert!(Repository::new(&rig.root).scan().problems.is_empty());
}

#[test]
fn cleanup_never_deletes_what_is_not_calliopes() {
    let rig = Rig::new(true);
    let root = &rig.root;
    let tracks = root.join("tracks");
    let tmp = root.join("import-tmp");
    fs::create_dir_all(&tmp).unwrap();
    fs::write(root.join("README.txt"), "mine").unwrap();
    fs::create_dir_all(root.join("trash/x")).unwrap();
    fs::write(root.join("trash/x/f"), "mine").unwrap();
    // look-alikes WITHOUT the marker / job.json
    let sid = "0190b1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b";
    fs::create_dir_all(tracks.join(format!(".staging-{sid}"))).unwrap();
    fs::write(tracks.join(format!(".staging-{sid}/keep.txt")), "mine").unwrap();
    fs::create_dir_all(tracks.join(".staging-mine")).unwrap();
    fs::write(tracks.join(".staging-mine/keep.txt"), "mine").unwrap();
    fs::write(tracks.join(".staging-file.txt"), "mine").unwrap();
    fs::create_dir_all(tracks.join("not-a-track")).unwrap();
    fs::write(tracks.join("not-a-track/keep.txt"), "mine").unwrap();
    fs::write(tmp.join("notes.txt"), "mine").unwrap();
    let url_lookalike = tmp.join("url-0123456789abcdef");
    fs::create_dir_all(&url_lookalike).unwrap();
    fs::write(url_lookalike.join("keep.txt"), "mine").unwrap();
    let file_lookalike = tmp.join("file-0190b1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b");
    fs::create_dir_all(&file_lookalike).unwrap();
    fs::write(file_lookalike.join("keep.txt"), "mine").unwrap();
    // job.json present but name does not match the pattern
    let odd = tmp.join("my-import");
    fs::create_dir_all(&odd).unwrap();
    fs::write(odd.join("job.json"), "{}").unwrap();
    fs::write(odd.join("keep.txt"), "mine").unwrap();
    // symlinks: to an outside folder that holds the markers
    let target = rig.outside.join("victim");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join(".calliope-staging"), "").unwrap();
    fs::write(target.join("job.json"), "{}").unwrap();
    fs::write(target.join("data.txt"), "mine").unwrap();
    symlink(&target, tracks.join(format!(".staging-0190b1c2-3d4e-7f50-8a6b-000000000001"))).unwrap();
    symlink(&target, tmp.join("file-0190b1c2-3d4e-7f50-8a6b-000000000002")).unwrap();
    symlink(&target, tmp.join("url-aaaaaaaaaaaaaaaa")).unwrap();
    // A genuinely stale folder of ours, with a symlink inside pointing outside
    let ours = tmp.join("file-0190b1c2-3d4e-7f50-8a6b-000000000003");
    fs::create_dir_all(&ours).unwrap();
    fs::write(ours.join("job.json"), "{}").unwrap();
    symlink(&target, ours.join("inner-link")).unwrap();
    let ours_staging = tracks.join(".staging-0190b1c2-3d4e-7f50-8a6b-000000000004");
    fs::create_dir_all(&ours_staging).unwrap();
    fs::write(ours_staging.join(".calliope-staging"), "").unwrap();
    symlink(&target, ours_staging.join("inner-link")).unwrap();

    let before = fingerprint(root);
    let out_before = fingerprint(&rig.outside);
    // any new import start runs the stale cleanup
    rig.file("untagged.flac").unwrap();
    rig.ready();

    let after = fingerprint(root);
    // our own stale folders are gone ...
    assert!(!ours.exists() && !ours_staging.exists(), "stale Calliope folders should be removed");
    // ... and nothing else that existed is missing
    for (k, v) in &before {
        if k.starts_with("import-tmp/file-0190b1c2-3d4e-7f50-8a6b-000000000003")
            || k.starts_with("tracks/.staging-0190b1c2-3d4e-7f50-8a6b-000000000004")
        {
            continue;
        }
        assert_eq!(after.get(k), Some(v), "foreign entry changed or deleted: {k}");
    }
    assert_eq!(fingerprint(&rig.outside), out_before, "something outside the repository changed (symlink followed?)");
    assert_eq!(fs::read_to_string(target.join("data.txt")).unwrap(), "mine");
}

#[test]
fn symlinked_import_tmp_or_tracks_is_refused_and_the_target_untouched() {
    let rig = Rig::new(false);
    let victim = rig.dir.path().join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("data.txt"), "mine").unwrap();
    symlink(&victim, rig.root.join("import-tmp")).unwrap();
    let r = rig.file("untagged.flac");
    assert!(r.is_err(), "import started through a symlinked import-tmp");
    let r = rig.url(WATCH, false);
    assert!(r.is_err());
    assert_eq!(names_in(&victim), ["data.txt"], "the symlink target was written to");
    rig.outside_ok();
}

#[test]
fn start_with_missing_or_foreign_root_fails_without_creating_anything() {
    let rig = Rig::new(false);
    let missing = rig.dir.path().join("not-there");
    let e = rig.state.start_file(&missing, FileKind::Audio, &rig.sources.join("untagged.flac"), rig.sink.clone()).unwrap_err();
    assert!(e.contains("missing") || e.contains("repository"), "{e}");
    assert!(!missing.exists());
    // a folder that is not a Calliope repository (has foreign files)
    let foreign = rig.dir.path().join("foreign");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("my-photo.jpg"), "x").unwrap();
    let before = fingerprint(&foreign);
    let r = rig.state.start_file(&foreign, FileKind::Audio, &rig.sources.join("untagged.flac"), rig.sink.clone());
    assert!(r.is_err(), "an import started in a folder that is not a Calliope repository");
    assert_eq!(fingerprint(&foreign), before);
}

#[test]
fn user_source_files_are_read_only_even_in_a_read_only_folder() {
    let server = RealServer::start("ok");
    let rig = Rig::new(false);
    let ro = rig.dir.path().join("ro");
    fs::create_dir_all(&ro).unwrap();
    fs::copy(fixture("tagged.mp3"), ro.join("song.mp3")).unwrap();
    fs::set_permissions(ro.join("song.mp3"), fs::Permissions::from_mode(0o444)).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    let before = fingerprint(&ro);
    rig.file_at(&ro.join("song.mp3")).unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server.url, false).unwrap();
    rig.wait("saved", |s| s.phase == Phase::Saved);
    assert_eq!(fingerprint(&ro), before);
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
}

// ====================================================================== argv / hostile file names

#[test]
fn file_names_that_look_like_options_or_shell_are_plain_data() {
    let server = RealServer::start("ok");
    let rig = Rig::new(false);
    let canary = rig.dir.path().join("pwned");
    let names = [
        "-n.mp3".to_string(),
        "--help.flac".to_string(),
        "-i evil.flac".to_string(),
        "-version".to_string() + ".ogg",
        "a;touch pwned;.flac".to_string(),
        "$(touch pwned).flac".to_string(),
        "`touch pwned`.flac".to_string(),
        "it's \"quoted\".flac".to_string(),
        "weird name with  spaces and ünïcode ♪.flac".to_string(),
        "colon:in:name.flac".to_string(),
        "file:x.flac".to_string(),
        "pipe|name.flac".to_string(),
        "100%.flac".to_string(),
        "new\nline.flac".to_string(),
    ];
    let dir = rig.dir.path().join("odd names");
    fs::create_dir_all(&dir).unwrap();
    for n in &names {
        let src = if n.ends_with(".mp3") { "tagged.mp3" } else if n.ends_with(".ogg") { "tagged.ogg" } else { "untagged.flac" };
        fs::copy(fixture(src), dir.join(n)).unwrap();
    }
    let before = fingerprint(&dir);
    for n in &names {
        std::env::set_current_dir(&dir).ok(); // a relative `-n.mp3` would hit cwd
        rig.file_at(&dir.join(n)).unwrap_or_else(|e| panic!("{n:?}: {e}"));
        let s = rig.idle();
        assert_eq!(s.phase, Phase::Ready, "{n:?}: {:?}", s.error);
        let m = s.metadata.clone().unwrap();
        assert!(!m.title.is_empty(), "{n:?}");
        assert!(!m.title.chars().any(|c| c.is_control()), "control characters in the title for {n:?}");
        rig.state.discard(&s.job).unwrap();
        assert!(!canary.exists(), "{n:?} executed something");
    }
    assert_eq!(fingerprint(&dir), before, "a source changed");
    rig.outside_ok();
    // one complete flow with an option-looking name
    rig.file_at(&dir.join("-n.mp3")).unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server.url, false).unwrap();
    let t = rig.wait("saved", |s| s.phase == Phase::Saved).track.unwrap();
    assert_eq!(t.stems.len(), 6);
    assert!(!canary.exists());
}

#[test]
fn urls_reach_yt_dlp_as_one_argv_entry_after_double_dash() {
    let rig = Rig::new(false);
    let urls = [
        "https://media.example/watch?v=--exec=touch%20pwned",
        "https://media.example/watch?v=a;touch pwned".replace(' ', "%20").as_str().to_owned().leak() as &str,
        "https://media.example/watch?v=$(touch%20pwned)",
        "https://media.example/watch?v=`id`&list=-x",
        "  https://media.example/watch?v=trim#frag  ",
        "https://MEDIA.EXAMPLE/watch?v=case",
    ];
    for u in urls {
        let s = rig.url(u, false).unwrap_or_else(|e| panic!("{u}: {e}"));
        rig.ready();
        rig.state.discard(&s.job).unwrap();
    }
    let runs = rig.ytdlp_runs();
    assert_eq!(runs.len(), urls.len());
    for r in &runs {
        let argv: Vec<&str> = r["argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        let sep = argv.iter().position(|a| *a == "--").expect("-- present");
        assert_eq!(argv.len(), sep + 2, "exactly one entry after --: {argv:?}");
        assert!(argv.contains(&"--ignore-config"));
        assert!(argv[sep + 1].starts_with("http"));
        assert!(!argv[..sep].iter().any(|a| a.contains("touch") || a.contains("media.example")), "{argv:?}");
        assert!(argv[sep + 1].find('#').is_none(), "fragment must be dropped: {}", argv[sep + 1]);
    }
    assert!(!rig.dir.path().join("pwned").exists());
    assert!(!Path::new("pwned").exists());
}

#[test]
fn invalid_urls_start_nothing_and_touch_nothing() {
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let bad = [
        "", "   ", "not a url", "example.com/x", "ftp://media.example/x", "file:///etc/passwd", "javascript:alert(1)", "data:text/plain,hi",
        "http://", "http://user:pass@media.example/", "http://user@media.example/", "-o /tmp/x", "--exec=id",
        "-", "http://media.example/a b", "http://media.example/\u{0}", "http://media.example/\u{7f}", "http://media.ex\tample/", "http://[::1", "http://media.example:99999/",
        "gopher://media.example", "ws://media.example/", "//media.example/x",
    ];
    for u in bad {
        let p = rig.state.prepare_url(&rig.root, u);
        assert_eq!(p.status, UrlPrepStatus::Invalid, "prepare accepted {u:?}");
        assert_eq!(p.message, "Entered URL is invalid", "{u:?}");
        let r = rig.url(u, false);
        assert_eq!(r.unwrap_err(), "Entered URL is invalid", "{u:?}");
    }
    let long = format!("https://media.example/{}", "a".repeat(2000));
    assert!(rig.url(&long, false).is_err());
    assert_eq!(fingerprint(&rig.root), before);
    assert!(rig.ytdlp_runs().is_empty());
    assert!(!rig.state.is_active());
}

// ====================================================================== spec flows

#[test]
fn url_flow_end_to_end_with_edits_and_temp_removal() {
    let server = RealServer::start("ok");
    let rig = Rig::new(true);
    rig.url(WATCH, false).unwrap();
    let ready = rig.ready();
    // temp file inside the repository
    let tmp_dirs = rig.import_tmp();
    assert_eq!(tmp_dirs.len(), 1);
    let jd = rig.root.join("import-tmp").join(&tmp_dirs[0]);
    assert!(names_in(&jd).iter().any(|n| n.starts_with("download.")), "{:?}", names_in(&jd));
    let m = ready.metadata.clone().unwrap();
    assert_eq!((m.band.as_str(), m.title.as_str(), m.year), ("The Example Band", "Night Drive", Some(2020)));
    // user edits
    let mut edits = m.clone();
    edits.album = "Edited Album".into();
    edits.title = "  Edited Title  ".into();
    let started = rig.state.start_extraction(&ready.job, edits, Some(&server.url), false).unwrap();
    assert_eq!(started.phase, Phase::Uploading);
    let done = rig.wait("saved", |s| s.phase == Phase::Saved);
    let t = done.track.unwrap();
    assert_eq!((t.title.as_str(), t.album.as_str()), ("Edited Title", "Edited Album"));
    assert_eq!(t.track_type, TrackType::Stem);
    let json: serde_json::Value = serde_json::from_slice(&fs::read(rig.root.join("tracks").join(&t.id).join("track.json")).unwrap()).unwrap();
    assert_eq!(json["type"], "stem");
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["title"], "Edited Title");
    assert_eq!(json["stems"].as_array().unwrap().len(), 6);
    // temp removed
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    assert!(rig.staging().is_empty());
    // "Working..." was reported only via the Working phase
    let phases: Vec<String> = rig.events.lock().unwrap().iter().map(|e| serde_json::to_value(e).unwrap()["phase"].as_str().unwrap().to_string()).collect();
    let pos = |p: &str| phases.iter().position(|x| x == p);
    assert!(pos("working").is_some() && pos("queued") < pos("working") && pos("working") < pos("receiving") && pos("receiving") < pos("saved"), "{phases:?}");
    // server job deleted
    until("server job gone", 10, || server.job_dirs().is_empty());
    rig.outside_ok();
}

#[test]
fn http_403_message_and_failed_download_keeps_resume_state() {
    let rig = Rig::new(false);
    rig.url("https://media.example/http403", false).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(e.message, "Error 403 when attempting download");
    assert_eq!(e.http_status, Some(403));
    assert!(!rig.state.is_active(), "after a download failure a new import must be possible");
    rig.url("https://media.example/offline", false).unwrap();
    let s = rig.idle();
    assert!(s.error.unwrap().message.starts_with("Download failed"));
}

#[test]
fn resume_and_start_over_and_the_prompt() {
    let rig = Rig::new(false);
    let url = "https://media.example/slow?v=1";
    assert_eq!(rig.state.prepare_url(&rig.root, url).status, UrlPrepStatus::Ready);
    let s = rig.url(url, false).unwrap();
    // wait for some bytes then cancel
    until("a .part file with bytes", 20, || part_has_bytes(&rig));
    rig.state.cancel(&s.job).unwrap();
    rig.wait("cancelled", |s| s.phase == Phase::Cancelled);
    let p = rig.state.prepare_url(&rig.root, url);
    assert_eq!(p.status, UrlPrepStatus::Partial);
    assert_eq!(p.message, "Incomplete download file from the same URL found");
    assert!(p.partial_bytes > 0);
    // a different URL is not offered the partial; the fragment does not matter
    assert_eq!(rig.state.prepare_url(&rig.root, "https://media.example/slow?v=2").status, UrlPrepStatus::Ready);
    assert_eq!(rig.state.prepare_url(&rig.root, &format!("{url}#frag")).status, UrlPrepStatus::Partial);
    // Resume
    rig.url(url, true).unwrap();
    rig.ready();
    let runs = rig.ytdlp_runs();
    let last = runs.last().unwrap();
    assert!(last["resume_from"].as_u64().unwrap_or(0) > 0, "resume must continue the partial: {last}");
    let argv: Vec<&str> = last["argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(argv.contains(&"--continue") && !argv.contains(&"--no-continue"));

    // Start Over (on a fresh partial)
    let rig = Rig::new(false);
    let s = rig.url(url, false).unwrap();
    until("a .part file with bytes", 20, || part_has_bytes(&rig));
    rig.state.cancel(&s.job).unwrap();
    rig.wait("cancelled", |s| s.phase == Phase::Cancelled);
    rig.url(url, false).unwrap();
    rig.ready();
    let runs = rig.ytdlp_runs();
    let last = runs.last().unwrap();
    assert_eq!(last["resume_from"].as_u64().unwrap_or(0), 0, "Start Over must restart at 0: {last}");
    let argv: Vec<&str> = last["argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(argv.contains(&"--no-continue"));
}

#[test]
fn local_files_metadata_and_errors() {
    let rig = Rig::new(false);
    for (f, title, band) in [("tagged.mp3", "Glass Harbour", "The Example Band"), ("tagged.ogg", "Glass Harbour", "The Example Band"), ("untagged.flac", "untagged", ""), ("with-audio.mp4", "Clip Title", "")] {
        rig.file(f).unwrap();
        let s = rig.ready();
        let m = s.metadata.unwrap();
        assert_eq!((m.title.as_str(), m.band.as_str()), (title, band), "{f}");
        rig.state.discard(&s.job).unwrap();
    }
    for (f, msg) in [
        ("no-audio.mp4", "Selected file no-audio.mp4 has no audio track"),
        ("not-audio.mp3", "Selected file not-audio.mp3 could not be read as audio or video"),
        ("long.flac", "Tracks longer than 15 minutes are not supported"),
    ] {
        rig.file(f).unwrap();
        let s = rig.idle();
        assert_eq!(s.phase, Phase::Failed, "{f}");
        assert_eq!(s.error.unwrap().message, msg, "{f}");
        assert!(!rig.state.is_active());
    }
    // a failed preparation leaves no temp folder
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    rig.outside_ok();
}

#[test]
fn unusual_source_files_are_handled_without_hanging() {
    let rig = Rig::new(false);
    let d = rig.dir.path().join("odd");
    fs::create_dir_all(&d).unwrap();
    // missing, directory, empty file, fifo, dangling symlink, text, relative path
    fs::create_dir(d.join("dir.mp3")).unwrap();
    fs::write(d.join("empty.mp3"), b"").unwrap();
    assert!(Command::new("mkfifo").arg(d.join("fifo.mp3")).status().unwrap().success());
    symlink(d.join("nowhere"), d.join("dangling.mp3")).unwrap();
    fs::write(d.join("text.flac"), "plain text").unwrap();
    for n in ["missing.mp3", "dir.mp3", "empty.mp3", "fifo.mp3", "dangling.mp3", "text.flac"] {
        let t = Instant::now();
        let r = rig.file_at(&d.join(n));
        if r.is_ok() {
            let s = rig.idle();
            assert_eq!(s.phase, Phase::Failed, "{n}");
            let m = s.error.unwrap().message;
            assert!(m.starts_with("Selected file") && m.contains(n), "{n}: {m}");
        }
        assert!(t.elapsed() < Duration::from_secs(20), "{n} took {:?}", t.elapsed());
        assert!(!rig.state.is_active(), "{n} left the import busy");
    }
    assert!(rig.state.start_file(&rig.root, FileKind::Audio, Path::new("relative.mp3"), rig.sink.clone()).is_err());
    assert!(rig.state.start_file(&rig.root, FileKind::Audio, Path::new("/"), rig.sink.clone()).is_err());
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    rig.no_stray_processes();
}

#[test]
fn a_sparse_file_over_1_gib_is_refused_before_reading() {
    let rig = Rig::new(false);
    let big = rig.dir.path().join("huge.flac");
    let f = fs::File::create(&big).unwrap();
    f.set_len((1u64 << 30) + 1).unwrap();
    drop(f);
    let t = Instant::now();
    rig.file_at(&big).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    assert!(s.error.unwrap().message.contains("larger than 1 GiB"));
    assert!(t.elapsed() < Duration::from_secs(10));
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
}

#[test]
fn hostile_tags_are_cleaned_and_bounded() {
    let rig = Rig::new(false);
    let out = rig.dir.path().join("evil-tags.flac");
    let long = "A".repeat(5000);
    let evil = "<img src=x onerror=alert(1)>\u{1b}[31mred\u{7}\ttab";
    let st = Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(fixture("untagged.flac"))
        .args(["-metadata", &format!("title={evil}\nsecond line"), "-metadata", &format!("artist={long}"), "-metadata", "date=99999999",
            "-metadata", "composer=A;B/C;;;/", "-metadata", "album=../../etc/passwd", "-c:a", "flac"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success());
    rig.file_at(&out).unwrap();
    let s = rig.ready();
    let m = s.metadata.clone().unwrap();
    assert!(m.title.chars().all(|c| !c.is_control()), "{:?}", m.title);
    assert!(m.title.chars().count() <= 200 && m.band.chars().count() <= 200, "{} {}", m.title.len(), m.band.len());
    assert_eq!(m.composers, ["A", "B", "C"]);
    assert!(m.year.map_or(true, |y| (1..=9999).contains(&y)), "{:?}", m.year);
    assert_eq!(m.album, "../../etc/passwd"); // text is text; it is only ever a JSON string
    // The edits are accepted by the strict validation (the edit pane would send them)
    let server = RealServer::start("ok");
    rig.extract(&s, &server.url, false).unwrap();
    let t = rig.wait("saved", |s| s.phase == Phase::Saved).track.unwrap();
    // the album never became a path
    assert!(rig.root.join("tracks").join(&t.id).is_dir());
    assert_eq!(names_in(&rig.root.join("tracks")).len(), 1);
    assert!(!rig.dir.path().join("etc").exists());
}

#[test]
fn the_15_minute_limit_boundaries_and_nothing_is_uploaded_when_too_long() {
    let server = RealServer::start("ok");
    let rig = Rig::new(false);
    let mk = |secs: f64, name: &str| {
        let p = rig.dir.path().join(name);
        let st = Command::new(tool("ffmpeg"))
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", &secs.to_string(), "-c:a", "flac"])
            .arg(&p)
            .status()
            .unwrap();
        assert!(st.success());
        p
    };
    // exactly 900 s is accepted end to end
    let exact = mk(900.0, "exact.flac");
    rig.file_at(&exact).unwrap();
    let s = rig.ready();
    rig.extract(&s, &server.url, false).unwrap();
    let done = rig.wait("done", |s| s.phase == Phase::Saved || s.phase == Phase::Failed);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    // 901 s is refused on our side; the server only ever saw health/none
    let over = mk(901.0, "over.flac");
    let posts_before = server.count("method=POST");
    rig.file_at(&over).unwrap();
    let s = rig.idle();
    assert_eq!(s.phase, Phase::Failed);
    assert_eq!(s.error.unwrap().message, "Tracks longer than 15 minutes are not supported");
    assert_eq!(server.count("method=POST"), posts_before);
    // 900.5 s: client and server must agree (either both accept or the client refuses)
    let edge = mk(900.5, "edge.flac");
    rig.file_at(&edge).unwrap();
    let s = rig.idle();
    if s.phase == Phase::Ready {
        rig.extract(&s, &server.url, false).unwrap();
        let d = rig.wait("end", |s| s.phase == Phase::Saved || s.phase == Phase::Failed);
        assert_eq!(d.phase, Phase::Saved, "the client accepted 900.5 s but the server refused: {:?}", d.error);
    } else {
        assert_eq!(s.error.unwrap().message, "Tracks longer than 15 minutes are not supported");
    }
}

// ====================================================================== hostile edge-AI server

fn run_against(ai: &FakeAi, rig: &Rig) -> JobSnapshot {
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &ai.url(), false).unwrap();
    rig.wait("end", |s| matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running())
}

fn assert_nothing_saved(rig: &Rig, tracks_before: usize) {
    assert!(rig.staging().is_empty(), "staging left: {:?}", rig.staging());
    assert_eq!(rig.tracks().len(), tracks_before, "a track appeared: {:?}", rig.tracks());
    rig.outside_ok();
}

#[test]
fn hostile_stem_names_from_the_server_never_become_paths() {
    let bad_lists: Vec<Vec<&str>> = vec![
        vec!["../../../evil"], vec![".."], vec!["a/b"], vec!["Vocals"], vec!["vocals.flac"], vec![""], vec!["vocals\u{0}x"],
        vec!["con"; 0], vec!["-rf"; 0],
    ];
    for names in bad_lists.into_iter().filter(|l| !l.is_empty()) {
        let mut cfg = AiConfig::good();
        cfg.stems = names.iter().map(|s| s.to_string()).collect();
        let ai = fake_ai(cfg);
        let rig = Rig::new(true);
        let before = fingerprint(&rig.root);
        let s = run_against(&ai, &rig);
        assert_eq!(s.phase, Phase::Failed, "{names:?}");
        assert!(!ai.saw("/stems/"), "stems must not even be requested for {names:?}: {:?}", ai.hits.lock().unwrap());
        assert_nothing_saved(&rig, v1_ids().len());
        // the prepared audio is kept so the user can retry; the repo's tracks are untouched
        let after = fingerprint(&rig.root);
        for (k, v) in &before {
            assert_eq!(after.get(k), Some(v), "{k}");
        }
        assert!(!rig.dir.path().join("evil.flac").exists() && !rig.dir.path().parent().unwrap().join("evil.flac").exists());
    }
}

#[test]
fn duplicate_empty_and_too_many_stems_are_refused() {
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("duplicate", vec!["vocals".into(), "vocals".into()]),
        ("empty", vec![]),
        ("seventeen", (0..17).map(|i| format!("s{i}")).collect()),
    ];
    for (name, stems) in cases {
        let mut cfg = AiConfig::good();
        cfg.stems = stems;
        let ai = fake_ai(cfg);
        let rig = Rig::new(true);
        let s = run_against(&ai, &rig);
        assert_eq!(s.phase, Phase::Failed, "{name}");
        assert_nothing_saved(&rig, v1_ids().len());
    }
    // 16 stems are fine
    let mut cfg = AiConfig::good();
    cfg.stems = (0..16).map(|i| format!("s{i}")).collect();
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let s = run_against(&ai, &rig);
    assert_eq!(s.phase, Phase::Saved, "{:?}", s.error);
    assert_eq!(s.track.unwrap().stems.len(), 16);
}

#[test]
fn non_flac_stem_body_fails_cleanly_and_keeps_the_audio_for_a_retry() {
    let mut cfg = AiConfig::good();
    cfg.body = Arc::new(|n| if n == "drums" { b"<html>captive portal</html>".to_vec() } else { fs::read(fixture(&format!("stems/{n}.flac"))).unwrap() });
    let ai = fake_ai(cfg);
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let s = run_against(&ai, &rig);
    assert_eq!(s.phase, Phase::Failed);
    assert_nothing_saved(&rig, v1_ids().len());
    // retry is possible: the audio is still there
    let again = rig.state.start_extraction(&s.job, s.metadata.clone().unwrap(), Some(&ai.url()), false);
    assert!(again.is_ok(), "retry refused: {:?}", again.err());
    rig.wait("end", |s| s.phase == Phase::Failed && !rig.state.is_running());
    assert_nothing_saved(&rig, v1_ids().len());
    // discard cleans the temp folder
    rig.state.discard(&s.job).unwrap();
    assert_eq!(rig.import_tmp(), Vec::<String>::new());
    for (k, v) in &before {
        if !k.starts_with("import-tmp") {
            assert_eq!(fingerprint(&rig.root).get(k), Some(v), "{k}");
        }
    }
}

#[test]
fn hostile_job_ids_and_failure_states() {
    for id in ["../../etc", "..", "", "-rf", "A-B", "a/b", "%2e%2e"] {
        let mut cfg = AiConfig::good();
        cfg.job_id = id.to_string();
        let ai = fake_ai(cfg);
        let rig = Rig::new(false);
        let s = run_against(&ai, &rig);
        assert_eq!(s.phase, Phase::Failed, "{id:?}");
        assert!(!ai.saw(&format!("/v1/jobs/{id}")) || id.is_empty(), "{id:?} was used in a URL");
        assert_nothing_saved(&rig, 0);
    }
    for (state, expect) in [("failed", "the separator exploded"), ("cancelled", "cancelled")] {
        let mut cfg = AiConfig::good();
        cfg.final_state = state;
        cfg.error = if state == "failed" { Some("the separator exploded".into()) } else { None };
        let ai = fake_ai(cfg);
        let rig = Rig::new(false);
        let s = run_against(&ai, &rig);
        assert_eq!(s.phase, Phase::Failed);
        assert!(s.error.unwrap().message.contains(expect), "{state}");
        assert_nothing_saved(&rig, 0);
    }
    // a gigantic, markup-laden server error text is passed on as text; its size is bounded
    let mut cfg = AiConfig::good();
    cfg.final_state = "failed";
    cfg.error = Some(format!("<script>alert(1)</script>{}", "x".repeat(50_000)));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let s = run_against(&ai, &rig);
    // (a long server message is cut: see server_error_text_is_truncated)
    assert_eq!(s.phase, Phase::Failed);
}

#[test]
fn server_error_text_is_truncated() {
    let mut cfg = AiConfig::good();
    cfg.final_state = "failed";
    cfg.error = Some(format!("<script>alert(1)</script>{}", "x".repeat(50_000)));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let s = run_against(&ai, &rig);
    let m = s.error.unwrap().message;
    assert!(m.chars().count() <= 300, "an unbounded server message ({} bytes) goes to the UI", m.len());
}

#[test]
fn a_server_with_a_tiny_upload_limit_gets_nothing_uploaded() {
    let mut cfg = AiConfig::good();
    cfg.max_upload = 100;
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let s = run_against(&ai, &rig);
    assert_eq!(s.phase, Phase::Failed);
    assert!(!ai.saw("POST"), "an upload was attempted although the file exceeds the advertised limit");
}

#[test]
fn out_of_range_progress_from_the_server_is_survivable() {
    let mut cfg = AiConfig::good();
    cfg.progress = serde_json::json!(1234.5);
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let s = run_against(&ai, &rig);
    // either outcome is fine, but a saved track must be valid
    if s.phase == Phase::Saved {
        assert!(Repository::new(&rig.root).scan().problems.is_empty());
    }
}

#[test]
fn unreachable_or_unconfigured_server_keeps_the_edit_pane_usable() {
    let rig = Rig::new(false);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    assert!(rig.state.start_extraction(&ready.job, ready.metadata.clone().unwrap(), None, false).is_err());
    assert!(rig.state.start_extraction(&ready.job, ready.metadata.clone().unwrap(), Some("  "), false).is_err());
    // closed port
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    rig.extract(&ready, &format!("http://{closed}"), false).unwrap();
    let s = rig.wait("failed", |s| s.phase == Phase::Failed && !rig.state.is_running());
    assert!(s.error.unwrap().message.contains("reach"), "message should say the server cannot be reached");
    assert_nothing_saved(&rig, 0);
    // invalid edits never start anything
    let mut bad = ready.metadata.clone().unwrap();
    bad.title = "   ".into();
    assert!(rig.state.start_extraction(&ready.job, bad, Some(&format!("http://{closed}")), false).is_err());
    let mut bad = ready.metadata.clone().unwrap();
    bad.year = Some(0);
    assert!(rig.state.start_extraction(&ready.job, bad, Some(&format!("http://{closed}")), false).is_err());
}

// ====================================================================== concurrency / lifecycle

#[test]
fn one_job_at_a_time_and_wrong_ids_are_rejected() {
    let server = RealServer::start("hang");
    let rig = Rig::new(false);
    rig.file("untagged.flac").unwrap();
    assert_eq!(rig.file("tagged.mp3").unwrap_err(), "An import is already running");
    assert_eq!(rig.url(WATCH, false).unwrap_err(), "An import is already running");
    let ready = rig.ready();
    assert!(rig.state.start_extraction("nope", ready.metadata.clone().unwrap(), Some(&server.url), false).is_err());
    assert!(rig.state.cancel("nope").is_err());
    assert!(rig.state.discard("nope").is_err());
    // still busy while waiting for edits
    assert_eq!(rig.state.prepare_url(&rig.root, WATCH).status, UrlPrepStatus::Busy);
    rig.extract(&ready, &server.url, false).unwrap();
    assert!(rig.extract(&ready, &server.url, false).is_err(), "double Extract");
    rig.wait("working", |s| s.phase == Phase::Working);
    assert!(rig.state.discard(&ready.job).is_err(), "discard while running");
    assert!(rig.state.shutdown(Duration::from_secs(5)));
    until("server job deleted by shutdown", 10, || server.job_dirs().is_empty());
}

#[test]
fn cancel_storm_leaves_no_zombies_no_staging_and_a_consistent_state() {
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    for i in 0..12 {
        let r = if i % 2 == 0 { rig.url("https://media.example/slow?v=storm", i % 4 == 0) } else { rig.file("long.flac") };
        let s = r.unwrap_or_else(|e| panic!("iteration {i}: {e}"));
        std::thread::sleep(Duration::from_millis((i * 37) % 150));
        let _ = rig.state.cancel(&s.job);
        let end = rig.wait("idle", |_| !rig.state.is_running());
        if !rig.state.is_active() {
            continue;
        }
        if end.phase == Phase::Ready {
            rig.state.discard(&s.job).unwrap();
        } else {
            // a cancelled url job stays over; starting the next one must work
        }
    }
    assert!(rig.staging().is_empty());
    rig.no_stray_processes();
    // the user's v1 tracks are untouched
    let after = fingerprint(&rig.root);
    for (k, v) in &before {
        assert_eq!(after.get(k), Some(v), "{k}");
    }
    // import-tmp holds at most Calliope's own resumable url folder
    assert!(rig.import_tmp().iter().all(|n| n.starts_with("url-")), "{:?}", rig.import_tmp());
}

// ====================================================================== crash (SIGKILL) tests

/// The child side: runs one import scenario forever. Does nothing unless QA_CHILD is set.
#[test]
fn qa_child() {
    let Ok(mode) = std::env::var("QA_CHILD") else { return };
    let root = PathBuf::from(std::env::var("QA_ROOT").unwrap());
    let server = std::env::var("QA_SERVER").unwrap_or_default();
    let sources = PathBuf::from(std::env::var("QA_SOURCES").unwrap());
    let wrapper = PathBuf::from(std::env::var("QA_YTDLP").unwrap());
    let tools = Tools { yt_dlp: Some(wrapper), ffmpeg: Some(tool("ffmpeg")), ffprobe: Some(tool("ffprobe")) };
    let lock: Arc<dyn RepoLock> = Arc::new(|f: &mut dyn FnMut()| f());
    let state = ImportState::new(tools, lock, Duration::from_millis(40));
    let sink: Sink = Arc::new(|_| {});
    let wait_ready = |state: &ImportState| -> JobSnapshot {
        loop {
            if let Some(s) = state.snapshot() {
                if s.phase == Phase::Ready {
                    return s;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    match mode.as_str() {
        "url-slow" => {
            state.start_url(&root, "https://media.example/slow?v=crash", false, sink).unwrap();
        }
        "file-long" => {
            state.start_file(&root, FileKind::Audio, &sources.join("exact.flac"), sink).unwrap();
        }
        _ => {
            state.start_file(&root, FileKind::Audio, &sources.join("untagged.flac"), sink).unwrap();
            let r = wait_ready(&state);
            state.start_extraction(&r.job, r.metadata.unwrap(), Some(&server), mode == "keep").unwrap();
        }
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

struct Crash {
    rig: Rig,
    child: Child,
    server: Option<RealServer>,
    fake: Option<FakeAi>,
}

impl Drop for Crash {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        kill_all_mentioning(&self.rig.needle());
    }
}

fn spawn_child(rig: &Rig, mode: &str, server_url: &str) -> Child {
    let exe = std::env::current_exe().unwrap();
    Command::new(exe)
        .args(["--exact", "qa_child", "--nocapture", "--test-threads=1"])
        .env("QA_CHILD", mode)
        .env("QA_ROOT", &rig.root)
        .env("QA_SERVER", server_url)
        .env("QA_SOURCES", &rig.sources)
        .env("QA_YTDLP", rig.tools.yt_dlp.as_ref().unwrap())
        .env("CALLIOPE_QA_RIG", rig.needle())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn sigkill(child: &mut Child) {
    let _ = Command::new("kill").args(["-9", &child.id().to_string()]).status();
    let _ = child.wait();
}

/// What must hold after the app died at any point.
fn after_crash_checks(rig: &Rig, before: &BTreeMap<String, String>, v1_count: usize) {
    // the library is intact and shows no half track
    let lib = Repository::new(&rig.root).scan();
    assert!(lib.problems.is_empty(), "problems after the crash: {:?}", lib.problems);
    assert_eq!(lib.tracks.len(), v1_count, "a partial track is visible");
    // all pre-existing files are byte-identical
    let now = fingerprint(&rig.root);
    for (k, v) in before {
        assert_eq!(now.get(k), Some(v), "{k} changed by the crash");
    }
    // no child process of the dead app survives (PDEATHSIG)
    until("children of the killed app gone", 10, || {
        procs_mentioning(&rig.needle()).iter().all(|(_, c)| c.contains("calliope-stems"))
    });
    // the next app run: cleans its own leftovers, never the user's
    fs::write(rig.root.join("tracks/.staging-mine-no-marker.txt"), "mine").unwrap();
    let state = ImportState::new(rig.tools.clone(), Arc::new(|f: &mut dyn FnMut()| f()), Duration::from_millis(40));
    state.start_file(&rig.root, FileKind::Audio, &rig.sources.join("untagged.flac"), Arc::new(|_| {})).unwrap();
    let mut waited = 0;
    while state.is_running() && waited < 3000 {
        std::thread::sleep(Duration::from_millis(10));
        waited += 10;
    }
    assert!(rig.staging().iter().all(|n| n == ".staging-mine-no-marker.txt"), "stale staging not cleaned: {:?}", rig.staging());
    assert!(rig.import_tmp().iter().all(|n| n.starts_with("url-") || n.starts_with("file-")));
    assert_eq!(fs::read_to_string(rig.root.join("tracks/.staging-mine-no-marker.txt")).unwrap(), "mine");
    let s = state.snapshot().unwrap();
    state.discard(&s.job).ok();
    rig.outside_ok();
}

#[test]
fn sigkill_during_a_url_download_keeps_a_resumable_partial_and_the_library_intact() {
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let mut child = spawn_child(&rig, "url-slow", "");
    until("a .part file with bytes", 30, || part_has_bytes(&rig));
    sigkill(&mut child);
    until("yt-dlp gone", 10, || procs_mentioning(&rig.needle()).is_empty());
    // the partial is offered for resume by the next run
    let p = rig.state.prepare_url(&rig.root, "https://media.example/slow?v=crash");
    assert_eq!(p.status, UrlPrepStatus::Partial);
    let now = fingerprint(&rig.root);
    for (k, v) in &before {
        assert_eq!(now.get(k), Some(v), "{k}");
    }
    // resume really resumes
    rig.url("https://media.example/slow?v=crash", true).unwrap();
    rig.ready();
    assert!(rig.ytdlp_runs().last().unwrap()["resume_from"].as_u64().unwrap_or(0) > 0);
    let s = rig.state.snapshot().unwrap();
    rig.state.discard(&s.job).unwrap();
    rig.outside_ok();
}

#[test]
fn sigkill_during_conversion_leaves_nothing_visible() {
    let rig = Rig::new(true);
    // a 900 s source takes ffmpeg a moment
    let p = rig.sources.join("exact.flac");
    assert!(Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-t", "900", "-c:a", "flac"])
        .arg(&p)
        .status()
        .unwrap()
        .success());
    let before = fingerprint(&rig.root);
    let mut child = spawn_child(&rig, "file-long", "");
    until("ffmpeg working", 30, || procs_mentioning(&rig.needle()).iter().any(|(_, c)| c.contains("-c:a flac")));
    sigkill(&mut child);
    after_crash_checks(&rig, &before, v1_ids().len());
}

#[test]
fn sigkill_while_the_server_works_leaves_nothing_visible_and_the_server_job_is_not_left_running_forever() {
    let server = RealServer::start("hang");
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let mut child = spawn_child(&rig, "extract", &server.url);
    until("server running the separator", 30, || server.saw("state=running"));
    sigkill(&mut child);
    after_crash_checks(&rig, &before, v1_ids().len());
    // The dead app cannot DELETE its job. The server keeps it until the separator timeout or the
    // retention; that is the documented design. The job folder is still marked (cleanable).
    let dirs = server.job_dirs();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    assert!(server.work.join("jobs").join(&dirs[0]).join(".calliope-stems-job").is_file());
}

#[test]
fn sigkill_while_receiving_stems_leaves_no_visible_partial_track() {
    let mut cfg = AiConfig::good();
    cfg.slow_stem_ms = 150;
    cfg.body = Arc::new(|n| {
        let mut b = fs::read(fixture(&format!("stems/{n}.flac"))).unwrap();
        b.resize(60_000, 0);
        b
    });
    let ai = fake_ai(cfg);
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let mut child = spawn_child(&rig, "keep", &ai.url());
    until("a staging folder with a .part", 40, || {
        rig.staging().iter().any(|d| names_in(&rig.root.join("tracks").join(d).join("stems")).iter().any(|n| n.ends_with(".part")))
    });
    // while it is mid-download the library must not show it
    let lib = Repository::new(&rig.root).scan();
    assert_eq!(lib.tracks.len(), v1_ids().len());
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    sigkill(&mut child);
    assert!(!rig.staging().is_empty(), "the crash should leave the staging folder behind (cleaned next start)");
    after_crash_checks(&rig, &before, v1_ids().len());
}

// ====================================================================== v2 metadata, hostile track.json

#[test]
fn hostile_v2_track_json_is_a_problem_and_never_followed() {
    let rig = Rig::new(false);
    let tracks = rig.root.join("tracks");
    let mk = |id: &str, json: serde_json::Value, files: &[&str]| {
        let d = tracks.join(id);
        fs::create_dir_all(&d).unwrap();
        for f in files {
            let p = d.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"fLaC").unwrap();
        }
        fs::write(d.join("track.json"), serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    };
    let base = |extra: serde_json::Value| {
        let mut v = serde_json::json!({"schema_version":2,"id":"","type":"stem","band":"b","album":"a","title":"t","composers":[],
            "year":null,"source_url":null,"copyright":null,"audio":null,"original":null,"tablatures":[],
            "imported":"2026-10-06T18:00:00Z","modified":"2026-10-06T18:00:00Z","stems":[{"name":"vocals","file":"stems/vocals.flac"}],"stem_model":null});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };
    let good = "0190b1c2-3d4e-7f50-8a6b-000000000001";
    mk(good, { let mut j = base(serde_json::json!({})); j["id"] = good.into(); j }, &["stems/vocals.flac"]);
    let hostile: Vec<(&str, serde_json::Value)> = vec![
        ("0190b1c2-3d4e-7f50-8a6b-000000000002", serde_json::json!({"stems":[{"name":"vocals","file":"stems/../../outside/x.flac"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000003", serde_json::json!({"stems":[{"name":"vocals","file":"/etc/passwd"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000004", serde_json::json!({"stems":[{"name":"../x","file":"stems/x.flac"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000005", serde_json::json!({"stems":[{"name":"vocals","file":"vocals.flac"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000006", serde_json::json!({"stems":[{"name":"vocals","file":"stems/a.flac"},{"name":"vocals","file":"stems/b.flac"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000007", serde_json::json!({"stems":[{"name":"a","file":"stems/X.flac"},{"name":"b","file":"stems/x.flac"}]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000008", serde_json::json!({"stems":[]})),
        ("0190b1c2-3d4e-7f50-8a6b-000000000009", serde_json::json!({"type":"video"})),
        ("0190b1c2-3d4e-7f50-8a6b-00000000000a", serde_json::json!({"original":"../x.flac"})),
        ("0190b1c2-3d4e-7f50-8a6b-00000000000b", serde_json::json!({"original":"/abs.flac"})),
        ("0190b1c2-3d4e-7f50-8a6b-00000000000c", serde_json::json!({"stems":(0..17).map(|i| serde_json::json!({"name":format!("s{i}"),"file":format!("stems/s{i}.flac")})).collect::<Vec<_>>()})),
        ("0190b1c2-3d4e-7f50-8a6b-00000000000e", serde_json::json!({"schema_version":3})),
        ("0190b1c2-3d4e-7f50-8a6b-00000000000f", serde_json::json!({"audio":"x.mp3","type":"backing"})),
    ];
    for (id, extra) in &hostile {
        let mut j = base(extra.clone());
        j["id"] = (*id).into();
        mk(id, j, &[]);
    }
    let lib = Repository::new(&rig.root).scan();
    let ok_ids: Vec<&str> = lib.tracks.iter().map(|t| t.id.as_str()).collect();
    // the good one loads; of the hostile ones only a "backing track with x.mp3 missing" may load (as `missing`)
    assert!(ok_ids.contains(&good));
    for (id, extra) in &hostile {
        let loaded = ok_ids.contains(id);
        if id.ends_with("00f") || id.ends_with("00c") {
            // a backing track with a missing file loads (as `missing`); 17 stems are read leniently
            continue;
        }
        assert!(!loaded, "hostile track.json loaded: {extra}");
    }
    assert!(lib.problems.len() >= hostile.len() - 2, "{} problems for {} hostile tracks", lib.problems.len(), hostile.len());
    // nothing was rewritten
    for (id, _) in &hostile {
        let j: serde_json::Value = serde_json::from_slice(&fs::read(tracks.join(id).join("track.json")).unwrap()).unwrap();
        assert!(j["id"] == *id);
    }
}

#[test]
fn a_v1_track_with_a_type_key_is_handled_per_the_rule() {
    let rig = Rig::new(false);
    let tracks = rig.root.join("tracks");
    let id_ok = "0190b1c2-3d4e-7f50-8a6b-0000000000a1";
    let id_bad = "0190b1c2-3d4e-7f50-8a6b-0000000000a2";
    for (id, ty) in [(id_ok, "backing"), (id_bad, "stem")] {
        let d = tracks.join(id);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.mp3"), "x").unwrap();
        let j = serde_json::json!({"schema_version":1,"id":id,"type":ty,"band":"b","album":"a","title":"t","composers":[],"year":null,
            "source_url":null,"copyright":null,"audio":"a.mp3","tablatures":[],"imported":"2026-10-06T18:00:00Z","modified":"2026-10-06T18:00:00Z"});
        fs::write(d.join("track.json"), serde_json::to_vec_pretty(&j).unwrap()).unwrap();
    }
    let before = fingerprint(&rig.root);
    let lib = Repository::new(&rig.root).scan();
    assert!(lib.tracks.iter().any(|t| t.id == id_ok));
    assert!(!lib.tracks.iter().any(|t| t.id == id_bad));
    assert!(lib.problems.iter().any(|p| p.dir.contains(id_bad)));
    assert_eq!(fingerprint(&rig.root), before);
}

// ====================================================================== missing-limit findings

#[test]
fn yt_dlp_has_a_download_size_and_duration_cap() {
    let args = download::ytdlp_args(Path::new("/tmp/x"), "https://media.example/x", false);
    assert!(args.windows(2).any(|w| w[0] == "--max-filesize" && w[1] == "1G"), "no --max-filesize in {args:?}");
    assert!(args.windows(2).any(|w| w[0] == "--match-filter" && w[1].contains("!is_live") && w[1].contains("900")), "{args:?}");
}

#[test]
fn prepare_url_does_not_create_import_tmp() {
    let rig = Rig::new(true);
    let _ = rig.state.prepare_url(&rig.root, WATCH);
    assert!(!rig.root.join("import-tmp").exists());
}

// ====================================================================== fix round 1 probes

#[test]
fn ytdlp_skip_outputs_show_the_right_message_and_leave_no_job() {
    let rig = Rig::new(false);
    for (path, msg) in [
        ("live", "Download refused: the video is a live stream or longer than 15 minutes"),
        ("long", "Download refused: the video is a live stream or longer than 15 minutes"),
        ("huge", "Download refused: the file is larger than 1 GiB"),
    ] {
        let url = format!("https://media.example/{path}");
        rig.url(&url, false).unwrap();
        let s = rig.idle();
        assert_eq!(s.phase, Phase::Failed, "{path}");
        assert_eq!(s.error.as_ref().unwrap().message, msg, "{path}");
        assert!(!rig.state.is_active(), "{path}: a job is still active");
        // no media file kept anywhere below import-tmp
        for d in rig.import_tmp() {
            let dir = rig.root.join("import-tmp").join(&d);
            let inner = names_in(&dir);
            assert!(!inner.iter().any(|n| n.starts_with("download.") || n == "audio.flac"), "{path}: {inner:?}");
        }
        assert!(rig.tracks().is_empty(), "{path}");
        // the app is not stuck: the next URL starts
        let ok = rig.url(WATCH, false).unwrap();
        rig.ready();
        rig.state.discard(&ok.job).unwrap();
    }
    rig.outside_ok();
}

#[test]
fn match_filter_and_max_filesize_arrive_as_single_argv_entries_before_the_double_dash() {
    let rig = Rig::new(false);
    let s = rig.url(WATCH, false).unwrap();
    rig.ready();
    rig.state.discard(&s.job).unwrap();
    let runs = rig.ytdlp_runs();
    assert_eq!(runs.len(), 1);
    let argv: Vec<&str> = runs[0]["argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let sep = argv.iter().position(|a| *a == "--").unwrap();
    let i = argv.iter().position(|a| *a == "--match-filter").expect("--match-filter");
    assert_eq!(argv[i + 1], "!is_live & duration <=? 900", "{argv:?}");
    assert!(i + 1 < sep);
    let j = argv.iter().position(|a| *a == "--max-filesize").expect("--max-filesize");
    assert_eq!(argv[j + 1], "1G");
    assert!(j + 1 < sep);
    assert_eq!(argv.iter().filter(|a| a.contains("is_live")).count(), 1);
}

fn loopback_trap() -> (SocketAddr, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    assert!(addr.ip().is_loopback());
    l.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (h2, s2) = (hits.clone(), stop.clone());
    std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if l.accept().is_ok() {
                h2.fetch_add(1, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    (addr, hits, stop)
}

#[test]
fn ffmpeg_never_follows_playlist_concat_or_hls_inputs_to_other_protocols() {
    let (addr, hits, stop) = loopback_trap();
    let rig = Rig::new(false);
    let outside_file = rig.outside.join("precious.txt");
    let cases: Vec<(&str, String)> = vec![
        ("pl.m3u8", format!("#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nhttp://{addr}/seg0.ts\n#EXT-X-ENDLIST\n")),
        ("pl2.m3u8", format!("#EXTM3U\n#EXTINF:10,\ntcp://{addr}\n")),
        ("pl.ffconcat", format!("ffconcat version 1.0\nfile 'http://{addr}/a.mp3'\n")),
        ("pl.txt", format!("ffconcat version 1.0\nfile '{}'\n", outside_file.display())),
        ("pl.sdp", format!("v=0\no=- 0 0 IN IP4 127.0.0.1\ns=x\nc=IN IP4 127.0.0.1\nt=0 0\nm=audio {} RTP/AVP 0\n", addr.port())),
    ];
    for (name, body) in cases {
        for ext in ["", ".mp3", ".flac", ".mp4"] {
            let p = rig.sources.join(format!("{name}{ext}"));
            fs::write(&p, &body).unwrap();
            let before = fs::read(&p).unwrap();
            let r = rig.file_at(&p);
            let s = match r {
                Ok(_) => rig.idle(),
                Err(e) => {
                    let _ = e;
                    continue;
                }
            };
            assert_eq!(s.phase, Phase::Failed, "{name}{ext}: {:?}", s.error);
            assert_eq!(fs::read(&p).unwrap(), before);
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::SeqCst);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "ffmpeg/ffprobe connected to the loopback trap");
    rig.outside_ok();
    assert!(rig.staging().is_empty());
}

/// A WAV whose header declares `declared_s` seconds of 8 kHz mono 16-bit audio but whose body holds `real_s`.
fn lying_wav(path: &Path, declared_s: u32, real_s: u32) {
    let rate = 8000u32;
    let data_declared = declared_s * rate * 2;
    let mut f = fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_declared).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * 2).to_le_bytes()).unwrap();
    f.write_all(&2u16.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_declared.to_le_bytes()).unwrap();
    let chunk: Vec<u8> = (0..rate * 2).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8 / 64).collect();
    for _ in 0..real_s {
        f.write_all(&chunk).unwrap();
    }
}

#[test]
fn a_source_whose_header_understates_its_length_never_yields_a_flac_over_15_minutes() {
    let rig = Rig::new(false);
    // an mp3 whose Xing header describes 10 s, followed by 20 more minutes of frames
    let mk = |secs: &str, xing: &str, name: &str| {
        let p = rig.dir.path().join(name);
        let st = Command::new(tool("ffmpeg"))
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=22050", "-t", secs, "-ac", "1", "-c:a", "libmp3lame", "-b:a", "16k", "-write_xing", xing])
            .arg(&p)
            .status()
            .unwrap();
        assert!(st.success());
        p
    };
    let head = mk("10", "1", "head.mp3");
    let body = mk("1200", "0", "body.mp3");
    let liar = rig.sources.join("liar.mp3");
    let mut bytes = fs::read(&head).unwrap();
    bytes.extend(fs::read(&body).unwrap());
    fs::write(&liar, bytes).unwrap();
    let declared: f64 = {
        let o = Command::new(tool("ffprobe")).args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(&liar).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
    };
    eprintln!("ffprobe declared duration of the liar: {declared}");
    rig.file_at(&liar).unwrap();
    let s = rig.idle();
    match s.phase {
        Phase::Failed => assert_eq!(s.error.unwrap().message, "Tracks longer than 15 minutes are not supported"),
        Phase::Ready => {
            let flac = rig.root.join("import-tmp").join(&rig.import_tmp()[0]).join("audio.flac");
            let out = Command::new(tool("ffprobe")).args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(&flac).output().unwrap();
            let dur: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
            eprintln!("understated-header FLAC duration: {dur} (declared {declared})");
            assert!(dur <= 906.0, "converted FLAC is {dur} s long");
        }
        other => panic!("unexpected phase {other:?}"),
    }
    assert!(rig.staging().is_empty());
}

#[test]
fn conversion_is_capped_even_when_the_probe_was_told_a_short_length() {
    // ffprobe cannot be fooled by the files I can build, so lie to to_flac directly: a probe of a
    // 10 s file, then a 20 minute source
    let d = tempfile::tempdir().unwrap();
    let mk = |secs: &str, name: &str| {
        let p = d.path().join(name);
        let st = Command::new(tool("ffmpeg"))
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=8000", "-t", secs, "-ac", "1", "-c:a", "pcm_s16le"])
            .arg(&p)
            .status()
            .unwrap();
        assert!(st.success());
        p
    };
    let short = mk("10", "short.wav");
    let long = mk("1200", "long.wav");
    let probe = media::probe(&tool("ffprobe"), &short, &media::Cancel::new()).unwrap();
    let job = tempfile::tempdir().unwrap();
    let flac = media::to_flac(&tool("ffmpeg"), &long, &probe, job.path(), &media::Cancel::new()).unwrap();
    let o = Command::new(tool("ffprobe")).args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(&flac).output().unwrap();
    let dur: f64 = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap();
    assert!(dur <= 906.0 && dur > 890.0, "FLAC is {dur} s");
}

#[test]
fn cancel_while_receiving_stems_stops_quickly_and_leaves_no_part_or_track() {
    let mut cfg = AiConfig::good();
    cfg.slow_stem_ms = 40;
    cfg.body = Arc::new(|_| {
        let mut b = b"fLaC".to_vec();
        b.extend(std::iter::repeat(7u8).take(3_000_000)); // ~30 s dribbled
        b
    });
    let ai = fake_ai(cfg);
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &ai.url(), false).unwrap();
    rig.wait("receiving", |s| s.phase == Phase::Receiving);
    std::thread::sleep(Duration::from_millis(400));
    let t = Instant::now();
    rig.state.cancel(&ready.job).unwrap();
    rig.wait("stopped", |s| !rig.state.is_running());
    assert!(t.elapsed() < Duration::from_secs(5), "cancel took {:?}", t.elapsed());
    assert!(rig.staging().is_empty(), "{:?}", rig.staging());
    assert_eq!(rig.tracks().len(), v1_ids().len());
    // no *.part anywhere below the repo
    fn parts(d: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                parts(&p, out);
            } else if p.to_string_lossy().ends_with(".part") {
                out.push(p);
            }
        }
    }
    let mut found = vec![];
    parts(&rig.root, &mut found);
    assert!(found.is_empty(), "{found:?}");
    for (k, v) in &before {
        if !k.starts_with("import-tmp") {
            assert_eq!(fingerprint(&rig.root).get(k), Some(v), "{k}");
        }
    }
    rig.no_stray_processes();
}

#[test]
fn prepare_url_creates_nothing_on_disk() {
    let rig = Rig::new(false);
    let before = fingerprint(&rig.root);
    assert!(!rig.root.join("import-tmp").exists());
    for u in [WATCH, "https://media.example/other", "not a url", ""] {
        let _ = rig.state.prepare_url(&rig.root, u);
    }
    assert!(!rig.root.join("import-tmp").exists(), "prepare_url created import-tmp");
    assert_eq!(fingerprint(&rig.root), before);
    // an unrelated root path that does not exist is not created either
    let ghost = rig.dir.path().join("ghost");
    let _ = rig.state.prepare_url(&ghost, WATCH);
    assert!(!ghost.exists());
    // with a real partial it reports Partial and still changes nothing
    rig.url("https://media.example/slow?v=p", false).unwrap();
    until("part bytes", 20, || part_has_bytes(&rig));
    let s = rig.state.snapshot().unwrap();
    rig.state.cancel(&s.job).unwrap();
    rig.idle();
    let snap = fingerprint(&rig.root);
    let prep = rig.state.prepare_url(&rig.root, "https://media.example/slow?v=p");
    assert_eq!(format!("{:?}", prep.status), "Partial");
    assert_eq!(fingerprint(&rig.root), snap);
}

#[test]
fn fragment_files_are_not_taken_as_the_download() {
    let d = tempfile::tempdir().unwrap();
    for n in ["download.f251.webm", "download.f137.mp4", "download.f140-1.m4a", "download.webm.part", "download.f251.webm.part", "download.webm.ytdl", "download.mp4.part-Frag3", "download.temp"] {
        fs::write(d.path().join(n), b"x").unwrap();
    }
    assert_eq!(download::find_download(d.path()), None);
    assert!(import_tmp::is_finished_download("download.webm"));
    assert!(import_tmp::is_finished_download("download.mp4"));
    assert!(import_tmp::is_finished_download("download.opus"));
    assert!(!import_tmp::is_finished_download("download.f251.webm"));
    assert!(!import_tmp::is_finished_download("downloadx.webm"));
    assert!(!import_tmp::is_finished_download("download."));
}
