//! QA acceptance tests for requirement 9 of specs/gui-stem-extraction.md (server-reported stem
//! peaks; the app doesn't download stems reported below -50 dBFS). Harness copied from
//! acceptance_silent_stems.rs; adds a scripted server that serves crafted `stem_peaks`.
//!
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
use stem_audio::{silent_peak, Level, SILENT_STEM_DBFS};
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
    /// raw `stem_peaks` value put in the done status (None = key omitted, like an old server)
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

/// Scripted server: stems = (name, body, reported peak (peak, bits) or None).
fn scripted(items: Vec<(&str, Vec<u8>, Option<(u64, u32)>)>, report: bool) -> FakeAi {
    let mut cfg = AiConfig::good();
    cfg.stems = items.iter().map(|i| i.0.to_string()).collect();
    let bodies: BTreeMap<String, Vec<u8>> = items.iter().map(|i| (i.0.to_string(), i.1.clone())).collect();
    cfg.body = Arc::new(move |n| bodies.get(n).cloned().unwrap_or_default());
    if report {
        let list: Vec<serde_json::Value> = items
            .iter()
            .filter_map(|(n, _, r)| {
                r.map(|(p, b)| {
                    let db = if p == 0 { serde_json::Value::Null } else { serde_json::json!(20.0 * (p as f64 / 2f64.powi(b as i32 - 1)).log10()) };
                    serde_json::json!({"name": n, "peak": p, "bits": b, "peak_dbfs": db})
                })
            })
            .collect();
        cfg.peaks = Some(serde_json::json!(list));
    }
    fake_ai(cfg)
}

fn outcome(done: &JobSnapshot) -> (Vec<String>, Vec<(String, Option<f64>)>) {
    (
        done.track.as_ref().map(kept_names).unwrap_or_default(),
        done.dropped.iter().map(|d| (d.name.clone(), d.peak_dbfs)).collect(),
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

// ====================================================================== same decision, whoever measures

#[test]
fn decision_is_identical_for_server_and_local_peaks_at_the_boundaries() {
    // (bits, peak, silent?)
    let cases: [(usize, i32, bool); 8] = [
        (16, 103, true),
        (16, -103, true),
        (16, 104, false),
        (16, -104, false),
        (24, 26527, true),
        (24, -26527, true),
        (24, 26528, false),
        (24, -26528, false),
    ];
    for (bits, v, silent) in cases {
        let items = || {
            vec![
                ("vocals", real_vocals(), Some((20000u64, 16u32))),
                ("drums", spike(v, bits, 8000, 100), Some((v.unsigned_abs() as u64, bits as u32))),
            ]
        };
        let with = scripted(items(), true);
        let without = scripted(items(), false);
        let (rig_a, rig_b) = (Rig::new(false), Rig::new(false));
        let a = import_with(&rig_a, &with.url());
        let b = import_with(&rig_b, &without.url());
        assert_eq!(a.phase, Phase::Saved, "{bits}/{v}: {:?}", a.error);
        assert_eq!(b.phase, Phase::Saved, "{bits}/{v}: {:?}", b.error);
        assert_eq!(outcome(&a), outcome(&b), "server vs local differ at {bits}-bit {v}");
        assert_eq!(a.dropped.is_empty(), !silent, "{bits}/{v}");
        // downloads
        assert_eq!(without.fetched(), ["vocals", "drums"], "old server: everything fetched");
        let want: Vec<&str> = if silent { vec!["vocals"] } else { vec!["vocals", "drums"] };
        assert_eq!(with.fetched(), want, "{bits}/{v}: silent stems must not be fetched");
        assert!(rig_a.staging().is_empty() && rig_b.staging().is_empty());
        if silent {
            // no .part ever left; the kept track has exactly vocals
            assert_eq!(outcome(&a).0, ["vocals"]);
        }
    }
}

#[test]
fn stem_order_and_dropped_order_follow_the_server_list() {
    let items = vec![
        ("vocals", real_vocals(), Some((20000, 16))),
        ("drums", spike(5, 16, 8000, 3), Some((5, 16))),
        ("bass", real_vocals(), Some((20000, 16))),
        ("guitar", spike(0, 16, 8000, 0), Some((0, 16))),
        ("piano", real_vocals(), Some((20000, 16))),
        ("other", spike(50, 16, 8000, 1), Some((50, 16))),
    ];
    let ai = scripted(items, true);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done).0, ["vocals", "bass", "piano"]);
    assert_eq!(done.dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["drums", "guitar", "other"]);
    assert_eq!(done.dropped[1].peak_dbfs, None);
    let db = done.dropped[0].peak_dbfs.unwrap();
    assert!((db - 20.0 * (5f64 / 32768.0).log10()).abs() < 1e-9, "{db}");
    assert_eq!(ai.fetched(), ["vocals", "bass", "piano"]);
    assert_eq!(done.stems_done, 6, "progress counts skipped stems");
}

#[test]
fn partial_report_fetches_the_unreported_stems_and_checks_them_locally() {
    let items = vec![
        ("vocals", real_vocals(), None),                                  // not reported, loud: kept
        ("drums", spike(0, 16, 8000, 0), None),                           // not reported, silent: fetched then dropped locally
        ("bass", spike(0, 16, 8000, 0), Some((0, 16))),                   // reported silent: not fetched
    ];
    let ai = scripted(items, true);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done).0, ["vocals"]);
    assert_eq!(done.dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["drums", "bass"]);
    assert_eq!(ai.fetched(), ["vocals", "drums"]);
}

// ====================================================================== old server / compat

#[test]
fn old_server_without_the_key_downloads_and_checks_everything() {
    // the real server with the off switch is the "old server"
    let old = start_real("sparse", &["--no-stem-levels"]);
    let new = start_real("sparse", &[]);
    let (ra, rb) = (Rig::new(false), Rig::new(false));
    let a = import_with(&ra, &old.url);
    let b = import_with(&rb, &new.url);
    assert_eq!(a.phase, Phase::Saved, "{:?}", a.error);
    assert_eq!(outcome(&a), outcome(&b), "same result");
    assert_eq!(outcome(&a).0, ["vocals", "drums", "bass", "guitar"]);
    let mut got = old.stem_gets();
    got.sort();
    assert_eq!(got, ["bass", "drums", "guitar", "other", "piano", "vocals"], "old server: all 6 downloaded");
    assert_eq!(new.stem_gets(), ["vocals", "drums", "bass", "guitar"], "new server: piano/other never downloaded");
    assert!(!new.stem_gets().iter().any(|n| n == "piano" || n == "other"));
}

#[test]
fn null_or_empty_stem_peaks_behave_like_an_old_server() {
    for peaks in [serde_json::Value::Null, serde_json::json!([])] {
        let mut cfg = AiConfig::good();
        cfg.stems = vec!["vocals".into(), "drums".into()];
        let (v, d) = (real_vocals(), spike(0, 16, 8000, 0));
        cfg.body = Arc::new(move |n| if n == "vocals" { v.clone() } else { d.clone() });
        cfg.peaks = Some(peaks.clone());
        let ai = fake_ai(cfg);
        let rig = Rig::new(false);
        let done = import_with(&rig, &ai.url());
        assert_eq!(done.phase, Phase::Saved, "{peaks}: {:?}", done.error);
        assert_eq!(ai.fetched(), ["vocals", "drums"], "{peaks}");
        assert_eq!(outcome(&done).0, ["vocals"]);
        assert_eq!(done.dropped.len(), 1);
    }
}

#[test]
fn entries_with_extra_fields_or_no_peak_dbfs_are_accepted_and_peak_dbfs_is_ignored() {
    let mut cfg = AiConfig::good();
    cfg.stems = vec!["vocals".into(), "drums".into()];
    let (v, d) = (real_vocals(), spike(0, 16, 8000, 0));
    cfg.body = Arc::new(move |n| if n == "vocals" { v.clone() } else { d.clone() });
    // drums: peak says silent while the human field lies (-3 dB): the integer decides
    cfg.peaks = Some(serde_json::json!([
        {"name":"vocals","peak":20000,"bits":16,"future":"x"},
        {"name":"drums","peak":0,"bits":16,"peak_dbfs":-3.0}
    ]));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(ai.fetched(), ["vocals"]);
    assert_eq!(done.dropped[0].peak_dbfs, None, "dropped dB comes from the integer peak, not the wire dB");
}

// ====================================================================== lying / hostile server

#[test]
fn a_server_that_calls_a_loud_stem_silent_gets_it_dropped_unseen_by_design() {
    // Plan 2.4 "Data safety": the app drops on the configured server's word. Documented behaviour.
    let items = vec![
        ("vocals", real_vocals(), Some((0, 16))),
        ("drums", real_vocals(), Some((20000, 16))),
    ];
    let ai = scripted(items, true);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done).0, ["drums"]);
    assert_eq!(done.dropped[0].name, "vocals");
    assert_eq!(ai.fetched(), ["drums"]);
}

#[test]
fn a_server_that_calls_a_silent_stem_audible_is_overruled_by_the_local_check() {
    let items = vec![
        ("vocals", real_vocals(), Some((20000, 16))),
        ("drums", spike(0, 16, 8000, 0), Some((30000, 16))),
    ];
    let ai = scripted(items, true);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done).0, ["vocals"]);
    assert_eq!(done.dropped.len(), 1);
    assert_eq!(ai.fetched(), ["vocals", "drums"]);
}

#[test]
fn everything_reported_silent_fails_with_zero_downloads_and_no_residue() {
    let items: Vec<_> = ["vocals", "drums", "bass", "guitar", "piano", "other"].iter().map(|n| (*n, real_vocals(), Some((0u64, 16u32)))).collect();
    let ai = scripted(items, true);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Failed);
    let e = done.error.unwrap();
    assert_eq!(e.stage, Stage::Server);
    assert_eq!(e.message, "Every stem is silent (below -50 dBFS), so no track was saved");
    assert!(ai.fetched().is_empty(), "{:?}", ai.fetched());
    assert_nothing_left(&rig);
    assert!(ai.saw("DELETE"), "server job still deleted");
}

#[test]
fn malformed_peak_lists_are_rejected_before_any_download() {
    let bad: Vec<(&str, serde_json::Value)> = vec![
        ("unknown name", serde_json::json!([{"name":"kazoo","peak":0,"bits":16,"peak_dbfs":null}])),
        ("duplicate", serde_json::json!([{"name":"vocals","peak":0,"bits":16},{"name":"vocals","peak":5,"bits":16}])),
        ("bits 3", serde_json::json!([{"name":"vocals","peak":0,"bits":3}])),
        ("bits 33", serde_json::json!([{"name":"vocals","peak":0,"bits":33}])),
        ("bits 0", serde_json::json!([{"name":"vocals","peak":0,"bits":0}])),
        ("peak too big", serde_json::json!([{"name":"vocals","peak":32769,"bits":16}])),
        ("negative peak", serde_json::json!([{"name":"vocals","peak":-1,"bits":16}])),
        ("string peak", serde_json::json!([{"name":"vocals","peak":"0","bits":16}])),
        ("float peak", serde_json::json!([{"name":"vocals","peak":0.5,"bits":16}])),
        ("huge peak", serde_json::json!([{"name":"vocals","peak":18446744073709551615u64,"bits":16}])),
        ("missing bits", serde_json::json!([{"name":"vocals","peak":0}])),
        ("object instead of list", serde_json::json!({"vocals":0})),
        ("string name list", serde_json::json!(["vocals"])),
        ("path-like name", serde_json::json!([{"name":"../x","peak":0,"bits":16}])),
        ("too many", serde_json::Value::Array((0..50).map(|_| serde_json::json!({"name":"vocals","peak":0,"bits":16})).collect())),
    ];
    for (what, peaks) in bad {
        let mut cfg = AiConfig::good();
        cfg.stems = vec!["vocals".into(), "drums".into()];
        cfg.body = Arc::new(|_| real_vocals());
        cfg.peaks = Some(peaks);
        let ai = fake_ai(cfg);
        let rig = Rig::new(false);
        let done = import_with(&rig, &ai.url());
        assert_eq!(done.phase, Phase::Failed, "{what}: a malformed list must not be trusted");
        eprintln!("QA: malformed '{what}' -> {:?}", done.error.as_ref().map(|e| (&e.stage, &e.message)));
        assert!(ai.fetched().is_empty(), "{what}: downloads happened {:?}", ai.fetched());
        assert_nothing_left(&rig);
    }
}

#[test]
fn bad_peak_list_on_a_failed_or_running_status_is_not_validated_as_done() {
    // a failed job that carries stem_peaks of garbage still reports the job's own error
    let mut cfg = AiConfig::good();
    cfg.final_state = "failed";
    cfg.error = Some("separator exploded".into());
    cfg.peaks = Some(serde_json::json!([{"name":"x","peak":1,"bits":99}]));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url());
    assert_eq!(done.phase, Phase::Failed);
    eprintln!("QA: failed job + garbage peaks -> {:?}", done.error.as_ref().map(|e| e.message.clone()));
    assert_nothing_left(&rig);
}

#[test]
fn reported_peak_for_a_stem_missing_from_the_list_is_rejected_even_when_silent() {
    let mut cfg = AiConfig::good();
    cfg.stems = vec!["vocals".into(), "drums".into()];
    cfg.body = Arc::new(|_| real_vocals());
    cfg.peaks = Some(serde_json::json!([{"name":"piano","peak":0,"bits":16}]));
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    assert_eq!(import_with(&rig, &ai.url()).phase, Phase::Failed);
    assert!(ai.fetched().is_empty());
}

// ====================================================================== the real server end to end

#[test]
fn real_server_all_silent_has_zero_stem_downloads() {
    let server = start_real("silent", &[]);
    let rig = Rig::new(true);
    let done = import_with(&rig, &server.url);
    assert_eq!(done.phase, Phase::Failed);
    assert_eq!(done.error.unwrap().message, "Every stem is silent (below -50 dBFS), so no track was saved");
    assert!(server.stem_gets().is_empty(), "{:?}", server.stem_gets());
    let tmp = rig.import_tmp();
    assert_eq!(tmp.len(), 1, "audio.flac kept: {tmp:?}");
    assert!(names_in(&rig.root.join("import-tmp").join(&tmp[0])).contains(&"audio.flac".to_string()));
    assert!(rig.staging().is_empty());
    until("server job removed", 10, || server.job_dirs().is_empty());
}

#[test]
fn real_server_sparse_never_serves_piano_or_other() {
    let server = start_real("sparse", &[]);
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(server.stem_gets(), ["vocals", "drums", "bass", "guitar"]);
    assert_eq!(done.dropped.len(), 2);
    assert!(rig.staging().is_empty() && rig.import_tmp().is_empty());
}

#[test]
fn real_server_undecodable_stem_is_kept_and_downloaded() {
    let server = start_real("undecodable", &[]);
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(outcome(&done).0, ["vocals", "drums", "bass", "guitar", "piano"]);
    assert_eq!(done.dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["other"]);
    assert!(server.stem_gets().contains(&"piano".to_string()));
    assert!(!server.stem_gets().contains(&"other".to_string()));
}

/// A separator that copies six pre-built long stems.
fn long_separator(secs: u32) -> (PathBuf, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let stem = d.path().join("long.flac");
    let st = Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-t", &secs.to_string(), "-sample_fmt", "s16", "-c:a", "flac"])
        .arg(&stem)
        .status()
        .unwrap();
    assert!(st.success());
    let script = d.path().join("sep.sh");
    fs::write(
        &script,
        format!("#!/bin/bash\nmkdir -p \"$2\"\nfor n in vocals drums bass guitar piano other; do cp '{}' \"$2/$n.flac\"; done\necho 'progress 1'\n", stem.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, d)
}

#[test]
fn cancel_while_the_server_is_measuring_leaves_nothing() {
    let (sep, _keep) = long_separator(1500);
    let server = start_custom(&sep, &[]);
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server.url, true).unwrap();
    until("server measuring", 60, || server.saw("stem=vocals peak=") || !rig.state.is_running());
    if server.saw("measured=6/6") {
        eprintln!("QA: measuring finished before cancel could be issued; weak run");
    }
    let t = Instant::now();
    rig.state.cancel(&ready.job).unwrap();
    rig.wait("stopped", |_| !rig.state.is_running());
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    eprintln!("QA: app cancel during server measuring -> stopped after {:?}", t.elapsed());
    assert!(rig.staging().is_empty());
    assert_eq!(rig.tracks().len(), v1_ids().len());
    assert!(server.stem_gets().is_empty(), "no stem downloaded after a cancel in measuring: {:?}", server.stem_gets());
    until("server job removed", 15, || server.job_dirs().is_empty());
    let now = fingerprint(&rig.root);
    for (k, v) in &before {
        if !k.starts_with("import-tmp") {
            assert_eq!(now.get(k), Some(v), "{k}");
        }
    }
    rig.outside_ok();
}

#[test]
fn server_killed_while_measuring_fails_the_import_cleanly() {
    let (sep, _keep) = long_separator(1500);
    let server = start_custom(&sep, &[]);
    let rig = Rig::new(false);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server.url, false).unwrap();
    until("server measuring", 60, || server.saw("stem=vocals peak=") || !rig.state.is_running());
    let t = Instant::now();
    let _ = Command::new("kill").args(["-TERM", &server.child.id().to_string()]).status();
    let s = rig.wait("failed", |s| matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running());
    eprintln!("QA: server SIGTERM during measuring -> app {:?} after {:?}: {:?}", s.phase, t.elapsed(), s.error.as_ref().map(|e| (&e.stage, &e.message)));
    if s.phase == Phase::Failed {
        assert_nothing_left(&rig);
    }
    assert!(s.track.is_none(), "a track must not be saved from a half-measured job");
}

fn v1_ids() -> Vec<String> {
    names_in(&project().join("tests/fixtures/library-sample/tracks"))
}
