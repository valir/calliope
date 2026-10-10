//! QA acceptance tests (independent angle) for requirement 9 of specs/gui-stem-extraction.md:
//! empty = audible < 15 s total (100 ms windows above -40 dBFS, loudest channel). The same six
//! stems are imported against six kinds of server (real server with levels, real server
//! `--no-stem-levels`, scripted: no key / server-peaks-era `stem_peaks` / level -50 / window 50);
//! the verdict must be identical everywhere, only the download list differs. Harness copied from
//! acceptance_server_levels.rs. Loopback only, no audio device, no real model.
//! Same structure as `acceptance_stem_extraction.rs`: the pure modules are compiled in with
//! `#[path]`; imports run against the REAL `calliope-stems` binary with the stub separator
//! (`sparse` / `silent` modes) or a scripted loopback server whose stem bodies are FLAC files
//! built here with `flacenc`. Loopback only, no audio device (ManualBackend), no real model.
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
#[path = "../src/transport.rs"]
mod transport;
#[path = "../src/mixer.rs"]
mod mixer;
#[path = "../src/stem_audio.rs"]
mod stem_audio;
#[path = "../src/audio_out.rs"]
mod audio_out;
#[path = "../src/backing_render.rs"]
mod backing_render;
#[path = "../src/editor.rs"]
mod editor;

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

use audio_out::ManualBackend;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use import_job::*;
use repository::Repository;
use stem_audio::{check_reported, check_stem, StemCheck, EMPTY_STEM_MIN_AUDIBLE_MS};
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
    /// raw `stem_levels` value put in the done status (None = key omitted, like an old server)
    levels: Option<serde_json::Value>,
    /// raw `stem_peaks` value (the server-peaks-era key, which the app ignores)
    peaks: Option<serde_json::Value>,
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
            levels: None,
            peaks: None,
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
    /// names of the stems fetched, in request order
    fn fetched(&self) -> Vec<String> {
        self.hits.lock().unwrap().iter().filter(|l| l.starts_with("GET") && l.contains("/stems/")).map(|l| l.split("/stems/").nth(1).unwrap().split(' ').next().unwrap().to_string()).collect()
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
                    let mut v = serde_json::json!({"job": cfg.job_id, "state": cfg.final_state, "progress": cfg.progress,
                        "stems": if cfg.final_state == "done" { serde_json::json!(cfg.stems) } else { serde_json::Value::Null },
                        "error": cfg.error});
                    if let Some(l) = &cfg.levels {
                        v["stem_levels"] = l.clone();
                    }
                    if let Some(p) = &cfg.peaks {
                        v["stem_peaks"] = p.clone();
                    }
                    json(&mut s, "200 OK", v);
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
    /// QA: take the snapshot AFTER seeing the job stopped (the copied `wait("idle", ..)` read the
    /// snapshot first, so a job finishing in between returned a stale phase: the known flake).
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


// ====================================================================== FLAC builders

fn flac_bytes(samples: &[i32], channels: usize, bits: usize, rate: usize) -> Vec<u8> {
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, channels, bits, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    sink.as_slice().to_vec()
}

/// Mono, `frames` long, zeros with `v` at `at`.
fn spike(v: i32, bits: usize, frames: usize, at: usize) -> Vec<u8> {
    let mut s = vec![0; frames];
    s[at] = v;
    flac_bytes(&s, 1, bits, 8000)
}

fn real_vocals() -> Vec<u8> {
    fs::read(fixture("stems/vocals.flac")).unwrap()
}

fn start_custom(separator: &Path, extra: &[&str]) -> RealServer {
    let outer = tempfile::tempdir().unwrap();
    let work = outer.path().join("work");
    fs::create_dir_all(&work).unwrap();
    let mut child = Command::new(stems_binary())
        .arg("--separator")
        .arg(separator)
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(&work)
        .args(extra)
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

fn start_real(mode: &str, extra: &[&str]) -> RealServer {
    let s = start_custom(&project().join("tests/support/stub-separator"), extra);
    fs::write(s.work.join("stub-mode"), format!("{mode}\n")).unwrap();
    s
}

impl RealServer {
    /// stem names requested with GET /v1/jobs/<id>/stems/<name>
    fn stem_gets(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("method=GET") && l.contains("/stems/") && l.contains("status=200"))
            .map(|l| l.split("/stems/").nth(1).unwrap().split_whitespace().next().unwrap().to_string())
            .collect()
    }
}

/// Reported measure: (audible_ms, level_dbfs, window_ms).
type Report = (u64, i32, u32);
const AT: (i32, u32) = (-40, 100);

fn rep(ms: u64) -> Option<Report> {
    Some((ms, AT.0, AT.1))
}

/// Scripted server: stems = (name, body, reported measure or None).
fn scripted(items: Vec<(&str, Vec<u8>, Option<Report>)>, report: bool) -> FakeAi {
    let mut cfg = AiConfig::good();
    cfg.stems = items.iter().map(|i| i.0.to_string()).collect();
    let bodies: BTreeMap<String, Vec<u8>> = items.iter().map(|i| (i.0.to_string(), i.1.clone())).collect();
    cfg.body = Arc::new(move |n| bodies.get(n).cloned().unwrap_or_default());
    if report {
        let list: Vec<serde_json::Value> = items
            .iter()
            .filter_map(|(n, _, r)| {
                r.map(|(ms, level, window)| serde_json::json!({"name": n, "audible_ms": ms, "level_dbfs": level, "window_ms": window, "peak_dbfs": null}))
            })
            .collect();
        cfg.levels = Some(serde_json::json!(list));
    }
    fake_ai(cfg)
}

fn outcome(done: &JobSnapshot) -> (Vec<String>, Vec<(String, u64)>) {
    (
        done.track.as_ref().map(kept_names).unwrap_or_default(),
        done.dropped.iter().map(|d| (d.name.clone(), d.audible_ms)).collect(),
    )
}
fn kept_names(t: &repository::TrackRecord) -> Vec<String> {
    t.stems.iter().map(|s| s.name.clone()).collect()
}
fn import_with(rig: &Rig, url: &str) -> JobSnapshot {
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, url, false).unwrap();
    rig.wait("saved or failed", |s| matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running())
}
fn assert_nothing_left(rig: &Rig) {
    assert!(rig.staging().is_empty(), "{:?}", rig.staging());
    assert!(rig.tracks().is_empty(), "{:?}", rig.tracks());
    rig.outside_ok();
}

/// A one-second stem of digital silence (0 ms audible).
fn zeros() -> Vec<u8> {
    spike(0, 16, 8000, 0)
}

fn activity(name: &str) -> Vec<u8> {
    fs::read(fixture(&format!("stems-activity/{name}"))).unwrap()
}


// ====================================================================== independent stems

fn stereo_44k(l: i32, r: i32, secs: usize) -> Vec<u8> {
    let mut s = Vec::with_capacity(44100 * secs * 2);
    for _ in 0..44100 * secs {
        s.push(l);
        s.push(r);
    }
    flac_bytes(&s, 2, 16, 44100)
}

/// (name, body, true audible ms)
fn six() -> Vec<(&'static str, Vec<u8>, u64)> {
    vec![
        ("vocals", activity("phrases.flac"), 16_000),
        ("drums", activity("audible-15000ms.flac"), 15_000),
        ("bass", activity("audible-14900ms.flac"), 14_900),
        ("guitar", activity("bursts.flac"), 9_500),
        ("piano", stereo_44k(300, 330, 16), 16_000), // only the right channel is above -40 dBFS
        ("other", spike(0, 16, 8000, 0), 0),
    ]
}
const KEPT: [&str; 3] = ["vocals", "drums", "piano"];
const DROPPED: [(&str, u64); 3] = [("bass", 14_900), ("guitar", 9_500), ("other", 0)];

#[derive(Clone, Copy, Debug)]
enum Flavour {
    WithLevels,
    NoKey,
    PeaksEra,
    OtherLevel,
    OtherWindow,
}

fn fake(flavour: Flavour) -> FakeAi {
    let items = six();
    let mut cfg = AiConfig::good();
    cfg.stems = items.iter().map(|i| i.0.to_string()).collect();
    let bodies: BTreeMap<String, Vec<u8>> = items.iter().map(|i| (i.0.to_string(), i.1.clone())).collect();
    cfg.body = Arc::new(move |n| bodies.get(n).cloned().unwrap_or_default());
    let list = |level: i32, window: u32| {
        // times are multiples of the window so check_stem_levels accepts them
        serde_json::json!(items.iter().map(|(n, _, ms)| serde_json::json!({"name": n, "audible_ms": ms / window as u64 * window as u64, "level_dbfs": level, "window_ms": window, "peak_dbfs": -3.0})).collect::<Vec<_>>())
    };
    match flavour {
        Flavour::WithLevels => cfg.levels = Some(list(-40, 100)),
        Flavour::NoKey => {}
        Flavour::PeaksEra => {
            // what the server-peaks build sent: peaks that would have KEPT the loud bursts
            cfg.peaks = Some(serde_json::json!(items.iter().map(|(n, ..)| serde_json::json!({"name": n, "peak_dbfs": -3.0})).collect::<Vec<_>>()));
        }
        Flavour::OtherLevel => cfg.levels = Some(list(-50, 100)),
        Flavour::OtherWindow => cfg.levels = Some(list(-40, 50)),
    }
    fake_ai(cfg)
}

fn separator_with(bodies: &[(&str, Vec<u8>)]) -> (PathBuf, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let mut script = String::from("#!/bin/bash\nmkdir -p \"$2\"\n");
    for (n, b) in bodies {
        let p = d.path().join(format!("{n}.flac"));
        fs::write(&p, b).unwrap();
        script.push_str(&format!("cp '{}' \"$2/{n}.flac\"\n", p.display()));
    }
    script.push_str("echo 'progress 1'\n");
    let sp = d.path().join("sep.sh");
    fs::write(&sp, script).unwrap();
    fs::set_permissions(&sp, fs::Permissions::from_mode(0o755)).unwrap();
    (sp, d)
}

fn dropped_pairs() -> Vec<(String, u64)> {
    DROPPED.iter().map(|(n, m)| (n.to_string(), *m)).collect()
}

#[test]
fn local_check_agrees_with_the_known_audible_times() {
    let d = tempfile::tempdir().unwrap();
    for (n, body, ms) in six() {
        let p = d.path().join(format!("{n}.flac"));
        fs::write(&p, body).unwrap();
        match check_stem(&p).unwrap() {
            StemCheck::Audible { audible_ms } => assert!(ms >= EMPTY_STEM_MIN_AUDIBLE_MS && audible_ms >= EMPTY_STEM_MIN_AUDIBLE_MS, "{n}"),
            StemCheck::Empty { audible_ms, .. } => assert_eq!((ms < EMPTY_STEM_MIN_AUDIBLE_MS, audible_ms), (true, ms), "{n}"),
        }
    }
    assert_eq!(EMPTY_STEM_MIN_AUDIBLE_MS, 15_000);
}

#[test]
fn boundary_14_9_vs_15_0_via_the_local_check_and_via_a_server_report() {
    let d = tempfile::tempdir().unwrap();
    for (f, empty) in [("audible-14900ms.flac", true), ("audible-15000ms.flac", false)] {
        assert_eq!(matches!(check_stem(&fixture(&format!("stems-activity/{f}"))).unwrap(), StemCheck::Empty { .. }), empty, "{f}");
    }
    let lv = |ms| calliope_lib::stems_api::StemLevel { name: "x".into(), audible_ms: ms, level_dbfs: -40, window_ms: 100, peak_dbfs: None };
    assert!(matches!(check_reported(&lv(14_900)), Some(StemCheck::Empty { .. })));
    assert!(matches!(check_reported(&lv(15_000)), Some(StemCheck::Audible { .. })));
    assert!(check_reported(&calliope_lib::stems_api::StemLevel { level_dbfs: -50, ..lv(0) }).is_none());
    assert!(check_reported(&calliope_lib::stems_api::StemLevel { window_ms: 50, ..lv(0) }).is_none());
    let _ = d;
}

#[test]
fn identical_verdict_for_every_kind_of_server_only_the_download_list_differs() {
    let mut outcomes = Vec::new();
    for fl in [Flavour::WithLevels, Flavour::NoKey, Flavour::PeaksEra, Flavour::OtherLevel, Flavour::OtherWindow] {
        let ai = fake(fl);
        let rig = Rig::new(false);
        let done = import_with(&rig, &ai.url());
        assert_eq!(done.phase, Phase::Saved, "{fl:?}: {:?}", done.error);
        let (kept, dropped) = outcome(&done);
        assert_eq!(kept, KEPT, "{fl:?}");
        assert_eq!(dropped, dropped_pairs(), "{fl:?}");
        match fl {
            Flavour::WithLevels => assert_eq!(ai.fetched(), KEPT, "reported-empty stems must never be requested"),
            _ => assert_eq!(ai.fetched(), ["vocals", "drums", "bass", "guitar", "piano", "other"], "{fl:?}: everything downloaded"),
        }
        assert!(rig.staging().is_empty() && rig.import_tmp().is_empty(), "{fl:?}");
        outcomes.push(outcome(&done));
    }
    assert!(outcomes.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn real_server_with_and_without_levels_gives_the_same_track_and_the_same_numbers() {
    let bodies: Vec<(&str, Vec<u8>)> = six().into_iter().map(|(n, b, _)| (n, b)).collect();
    let (sep, _keep) = separator_with(&bodies);
    let mut res = Vec::new();
    for extra in [&[][..], &["--no-stem-levels"][..]] {
        let server = start_custom(&sep, extra);
        let rig = Rig::new(false);
        let done = import_with(&rig, &server.url);
        assert_eq!(done.phase, Phase::Saved, "{extra:?}: {:?}", done.error);
        if extra.is_empty() {
            assert_eq!(server.stem_gets(), KEPT, "never downloaded: the request log must not list the dropped stems");
            assert!(!server.log.lock().unwrap().iter().any(|l| l.contains("/stems/bass") || l.contains("/stems/guitar") || l.contains("/stems/other")));
            // the server's own numbers are exactly the ones the app reports as dropped
            for (n, ms) in DROPPED {
                assert!(server.saw(&format!("stem={n} audible_ms={ms} ")), "{n}");
            }
        } else {
            assert_eq!(server.stem_gets().len(), 6);
        }
        res.push(outcome(&done));
    }
    assert_eq!(res[0], res[1], "server-measured and app-measured verdicts (and audible_ms) must agree bit for bit");
}

#[test]
fn all_empty_fails_with_the_exact_message_for_every_kind_of_server_and_keeps_audio_for_retry() {
    let zero = || spike(0, 16, 8000, 0);
    let names = ["vocals", "drums", "bass", "guitar", "piano", "other"];
    // quiet-but-not-silent (-45 dBFS-ish tone) and loud bursts are empty too
    let bodies: Vec<(&str, Vec<u8>)> = names.iter().enumerate().map(|(i, n)| (*n, if i % 2 == 0 { zero() } else { activity("bursts.flac") })).collect();
    let (sep, _keep) = separator_with(&bodies);
    let msg = "Every stem is empty (less than 15 s above -40 dBFS), so no track was saved";
    for extra in [&[][..], &["--no-stem-levels"][..]] {
        let server = start_custom(&sep, extra);
        let rig = Rig::new(false);
        let done = import_with(&rig, &server.url);
        assert_eq!(done.phase, Phase::Failed, "{extra:?}");
        let e = done.error.unwrap();
        assert_eq!((e.stage, e.message.as_str()), (Stage::Server, msg), "{extra:?}");
        if extra.is_empty() {
            assert!(server.stem_gets().is_empty(), "{:?}", server.stem_gets());
        } else {
            assert_eq!(server.stem_gets().len(), 6);
        }
        assert!(rig.staging().is_empty() && rig.tracks().is_empty());
        assert_eq!(rig.import_tmp().len(), 1, "prepared audio kept for a retry");
    }
}

#[test]
fn a_part_with_pauses_and_a_loud_peak_artifact_stem_are_judged_by_time_not_peak() {
    // sparse: guitar = 8 x 2 s phrases (16 s) kept; other = bursts with a -8 dBFS peak, dropped
    let server = start_real("sparse", &[]);
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let (kept, dropped) = outcome(&done);
    assert!(kept.contains(&"guitar".to_string()) && !kept.contains(&"other".to_string()));
    assert!(dropped.contains(&("other".to_string(), 9_500)));
    assert!(!server.stem_gets().contains(&"other".to_string()));
}

#[test]
fn an_empty_stem_is_dropped_even_when_the_server_calls_it_audible_with_a_loud_peak() {
    // server claims everything is 20 s audible: the local proof of emptiness wins
    let mut cfg = AiConfig::good();
    let items = six();
    cfg.stems = items.iter().map(|i| i.0.to_string()).collect();
    let bodies: BTreeMap<String, Vec<u8>> = items.iter().map(|i| (i.0.to_string(), i.1.clone())).collect();
    cfg.body = Arc::new(move |n| bodies.get(n).cloned().unwrap_or_default());
    cfg.levels = Some(serde_json::json!(items.iter().map(|(n, ..)| serde_json::json!({"name": n, "audible_ms": 20_000, "level_dbfs": -40, "window_ms": 100, "peak_dbfs": -1.0})).collect::<Vec<_>>()));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done), (KEPT.iter().map(|s| s.to_string()).collect(), dropped_pairs()));
    assert_eq!(ai.fetched().len(), 6);
}

// ====================================================================== flake evidence

/// FLAKE ROOT CAUSE (import_job_tests `empty_stems_are_dropped_from_the_saved_track` "no saved event",
/// `cancel_during_extraction_goes_back_to_the_edit_pane`): `finish_extract` publishes the new
/// snapshot (phase Saved / Ready, running=false) BEFORE it calls the sink. A poller that waits for
/// `!is_running()` and then reads the event list can run in the gap. A slow sink widens the gap:
/// the snapshot is already final while the event has not arrived.
#[test]
fn snapshot_is_final_before_the_saved_event_is_delivered_so_pollers_must_wait_for_the_event() {
    let ai = fake(Flavour::WithLevels);
    let rig = Rig::new(false);
    let slow_events: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let e2 = slow_events.clone();
    let slow_sink: Sink = Arc::new(move |ev| {
        if matches!(ev, ImportEvent::Saved { .. }) {
            std::thread::sleep(Duration::from_millis(400));
        }
        e2.lock().unwrap().push(ev);
    });
    rig.state.start_file(&rig.root, FileKind::Audio, &rig.sources.join("untagged.flac"), slow_sink).unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &ai.url(), false).unwrap();
    let snap = rig.wait("saved and stopped", |s| s.phase == Phase::Saved && !rig.state.is_running());
    assert_eq!(snap.phase, Phase::Saved);
    let seen_saved_event_at_that_moment = slow_events.lock().unwrap().iter().any(|e| matches!(e, ImportEvent::Saved { .. }));
    assert!(!seen_saved_event_at_that_moment, "the race window is real: state final, event still pending");
    until("the Saved event arrives", 10, || slow_events.lock().unwrap().iter().any(|e| matches!(e, ImportEvent::Saved { .. })));
}
