//! QA acceptance tests for the "drop silent stems at import" increment of
//! specs/gui-stem-extraction.md (requirement 8, last acceptance criterion). Owner decision:
//! "empty" means sample peak below -50 dBFS.
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
        // Take the snapshot AFTER seeing the job stopped; reading it first returns a stale phase.
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

fn write_flac(dir: &Path, name: &str, samples: &[i32], channels: usize, bits: usize, rate: usize) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, flac_bytes(samples, channels, bits, rate)).unwrap();
    p
}

/// Mono 16-bit 8 kHz, `frames` long, zeros with `v` at `at`.
fn spike16(v: i32, frames: usize, at: usize) -> Vec<u8> {
    let mut s = vec![0; frames];
    s[at] = v;
    flac_bytes(&s, 1, 16, 8000)
}

/// Deterministic noise in -amp..=amp (LCG), interleaved.
fn noise(n: usize, amp: i32) -> Vec<i32> {
    let mut x: u64 = 0x9E3779B97F4A7C15;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let r = ((x >> 33) as i64 % (2 * amp as i64 + 1)) as i32 - amp;
            r
        })
        .collect()
}

fn level(path: &Path) -> Level {
    silent_peak(path).unwrap()
}

// ====================================================================== the measure (threshold)

#[test]
fn the_threshold_is_minus_50_dbfs() {
    assert_eq!(SILENT_STEM_DBFS, -50.0);
}

#[test]
fn boundary_16_bit_just_below_and_just_above_minus_50() {
    let d = tempfile::tempdir().unwrap();
    // 32768 * 10^(-50/20) = 103.62: 103 is -50.05 dBFS (silent), 104 is -49.97 (audible)
    for (v, audible) in [(0, false), (1, false), (103, false), (-103, false), (104, true), (-104, true), (32767, true), (-32768, true)] {
        let p = d.path().join("s.flac");
        fs::write(&p, spike16(v, 4000, 1234)).unwrap();
        assert_eq!(level(&p) == Level::Audible, audible, "peak {v}");
    }
}

#[test]
fn boundary_24_bit_at_and_around_the_exact_threshold() {
    let d = tempfile::tempdir().unwrap();
    // 2^23 * 10^-2.5 = 26527.1: 26527 is just below -50 (silent), 26528 is just above (audible)
    for (v, audible) in [(26526, false), (26527, false), (26528, true), (-26528, true)] {
        let mut s = vec![0; 4000];
        s[17] = v;
        let p = write_flac(d.path(), "s24.flac", &s, 1, 24, 44100);
        assert_eq!(level(&p) == Level::Audible, audible, "24-bit peak {v}");
    }
}

#[test]
fn boundary_8_bit_and_32_bit_depth() {
    let d = tempfile::tempdir().unwrap();
    // 8-bit: limit = ceil(128 * 0.00316) = 1, so a single LSB is already -42 dBFS: audible
    let p = write_flac(d.path(), "s8.flac", &vec![0; 4000], 1, 8, 8000);
    assert!(matches!(level(&p), Level::Silent { peak_dbfs: None }));
    let mut s = vec![0; 4000];
    s[3] = 1;
    let p = write_flac(d.path(), "s8b.flac", &s, 1, 8, 8000);
    assert_eq!(level(&p), Level::Audible);
}

#[test]
fn multichannel_any_channel_counts() {
    let d = tempfile::tempdir().unwrap();
    // 6 channels: the only loud sample is in the last channel, last frame
    let frames = 5000;
    let mut s = vec![0; frames * 6];
    let p = write_flac(d.path(), "six.flac", &s, 6, 16, 8000);
    assert!(matches!(level(&p), Level::Silent { peak_dbfs: None }));
    *s.last_mut().unwrap() = 2000;
    let p = write_flac(d.path(), "six.flac", &s, 6, 16, 8000);
    assert_eq!(level(&p), Level::Audible);
    // stereo, loud only on the left / only on the right, negative
    for idx in [0usize, 1] {
        let mut s = vec![0; 2 * 9000];
        s[2 * 8999 + idx] = -300;
        let p = write_flac(d.path(), "st.flac", &s, 2, 16, 44100);
        assert_eq!(level(&p), Level::Audible, "channel {idx}");
    }
    // stereo, both channels just below the threshold everywhere
    let mut s = vec![103; 2 * 9000];
    for (i, v) in s.iter_mut().enumerate() {
        *v = if i % 3 == 0 { -103 } else { 103 };
    }
    let p = write_flac(d.path(), "st2.flac", &s, 2, 16, 44100);
    assert!(matches!(level(&p), Level::Silent { peak_dbfs: Some(_) }));
}

#[test]
fn a_short_loud_passage_in_a_long_quiet_stem_is_kept() {
    let d = tempfile::tempdir().unwrap();
    // 60 s at 8 kHz of -60 dBFS noise with a 50 ms piano note (-20 dBFS) in the middle
    let rate = 8000;
    let mut s = noise(60 * rate, 33); // 33/32768 = -59.9 dBFS
    for i in 0..(rate / 20) {
        s[30 * rate + i] = ((i as f64 * 0.3).sin() * 3276.0) as i32;
    }
    let p = write_flac(d.path(), "long.flac", &s, 1, 16, rate);
    assert_eq!(level(&p), Level::Audible);
    // the same stem without the note is silent, and the logged peak is about -60
    let quiet = noise(60 * rate, 33);
    let p = write_flac(d.path(), "quiet.flac", &quiet, 1, 16, rate);
    match level(&p) {
        Level::Silent { peak_dbfs: Some(db) } => assert!(db < -50.0 && db > -62.0, "{db}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn demucs_style_leakage_at_minus_51_is_dropped_and_at_minus_49_kept() {
    let d = tempfile::tempdir().unwrap();
    // amp 93 = -50.9 dBFS ; amp 105 = -49.9 dBFS ; force the extreme value to occur
    let mut a = noise(2 * 20000, 93);
    a[777] = 93;
    let p = write_flac(d.path(), "m51.flac", &a, 2, 16, 44100);
    assert!(matches!(level(&p), Level::Silent { peak_dbfs: Some(_) }));
    let mut b = noise(2 * 20000, 93);
    b[19999 * 2] = -105;
    let p = write_flac(d.path(), "m49.flac", &b, 2, 16, 44100);
    assert_eq!(level(&p), Level::Audible);
}

#[test]
fn undecodable_input_is_an_error_not_silence() {
    let d = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty.flac", vec![]),
        ("magic-only.flac", b"fLaC".to_vec()),
        ("text.flac", b"fLaC-generic-body".to_vec()),
        ("riff.flac", b"RIFF....WAVEfmt ".to_vec()),
    ];
    for (n, b) in cases {
        let p = d.path().join(n);
        fs::write(&p, b).unwrap();
        assert!(silent_peak(&p).is_err(), "{n} should be an error");
    }
    // truncated real FLAC whose audible content is cut off
    let full = flac_bytes(&vec![0; 40000], 1, 16, 8000);
    let p = d.path().join("trunc.flac");
    fs::write(&p, &full[..full.len() / 2]).unwrap();
    assert!(silent_peak(&p).is_err(), "truncated stem must be Err (kept), never Silent");
    assert!(silent_peak(&d.path().join("missing.flac")).is_err());
}

#[test]
fn a_long_silent_stem_is_checked_in_reasonable_time() {
    // worst case the cancel button has to wait for: 15 min, 44.1 kHz stereo of digital silence
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("long.flac");
    let st = Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-t", "900", "-sample_fmt", "s16", "-c:a", "flac"])
        .arg(&p)
        .status()
        .unwrap();
    assert!(st.success());
    let t = Instant::now();
    assert!(matches!(level(&p), Level::Silent { peak_dbfs: None }));
    let el = t.elapsed();
    eprintln!("QA: silent_peak of 15 min stereo 44.1k silence took {el:?}");
    assert!(el < Duration::from_secs(10), "{el:?}");
}

// ====================================================================== staging safety

#[test]
fn discard_stem_part_never_follows_a_symlink_and_never_touches_other_stems() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious.flac"), "precious").unwrap();
    let repo = Repository::new(&root);
    repo.ensure_layout().unwrap();
    let st = repo.begin_staged_track("0190b1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b").unwrap();
    let stems = st.dir().join("stems");
    // vocals: a real part; drums: a symlink part to an outside file; bass: a finished stem
    fs::write(stems.join("vocals.flac.part"), "x").unwrap();
    symlink(outside.join("precious.flac"), stems.join("drums.flac.part")).unwrap();
    fs::write(stems.join("bass.flac"), "keep").unwrap();
    fs::create_dir(stems.join("guitar.flac.part")).unwrap();

    assert!(st.discard_stem_part("drums").is_err(), "a symlink part must be refused");
    assert_eq!(fs::read_to_string(outside.join("precious.flac")).unwrap(), "precious");
    assert!(stems.join("drums.flac.part").symlink_metadata().is_ok());
    assert!(st.discard_stem_part("guitar").is_err(), "a folder must be refused");
    for bad in ["", "..", "../x", "a/b", "vocals.flac", "VOCALS\0"] {
        assert!(st.discard_stem_part(bad).is_err(), "{bad:?}");
    }
    assert!(st.discard_stem_part("piano").is_err(), "missing part");
    st.discard_stem_part("vocals").unwrap();
    assert!(!stems.join("vocals.flac.part").exists());
    assert_eq!(fs::read_to_string(stems.join("bass.flac")).unwrap(), "keep");
    // abandoning must not follow the symlink either
    st.abandon().unwrap();
    assert_eq!(fs::read_to_string(outside.join("precious.flac")).unwrap(), "precious");
}

#[test]
fn a_crash_between_stems_leaves_no_visible_track_and_the_next_start_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let repo = Repository::new(&root);
    repo.ensure_layout().unwrap();
    let id = "0190b1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b";
    {
        let st = repo.begin_staged_track(id).unwrap();
        let stems = st.dir().join("stems");
        fs::write(stems.join("vocals.flac"), "kept").unwrap();
        fs::write(stems.join("drums.flac.part"), "half").unwrap();
        std::mem::forget(st); // the process dies: no commit, no abandon
    }
    let lib = repo.scan();
    assert!(lib.tracks.is_empty(), "a staged track is visible");
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(repo.clean_stale_staging(None), 1);
    assert!(names_in(&root.join("tracks")).is_empty());
}

// ====================================================================== import-level rig

fn kept_names(t: &repository::TrackRecord) -> Vec<String> {
    t.stems.iter().map(|s| s.name.clone()).collect()
}

fn stem_files(rig: &Rig, id: &str) -> Vec<String> {
    names_in(&rig.root.join("tracks").join(id).join("stems"))
}

fn track_json(rig: &Rig, id: &str) -> serde_json::Value {
    serde_json::from_slice(&fs::read(rig.root.join("tracks").join(id).join("track.json")).unwrap()).unwrap()
}

fn import_with(rig: &Rig, url: &str, keep: bool) -> JobSnapshot {
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, url, keep).unwrap();
    rig.wait("saved or failed", |s| matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running())
}

fn assert_clean_after_save(rig: &Rig) {
    assert!(rig.staging().is_empty(), "{:?}", rig.staging());
    assert!(rig.import_tmp().is_empty(), "import-tmp not cleaned: {:?}", rig.import_tmp());
    rig.outside_ok();
}

#[test]
fn sparse_import_keeps_only_audible_stems_in_server_order() {
    let server = RealServer::start("sparse");
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url, false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let t = done.track.clone().unwrap();
    // server order is vocals, drums, bass, guitar, piano, other
    assert_eq!(kept_names(&t), ["vocals", "drums", "bass", "guitar"]);
    assert_eq!(
        done.dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        ["piano", "other"]
    );
    assert_eq!(done.dropped[0].peak_dbfs, None, "digital silence is -inf / None");
    let p = done.dropped[1].peak_dbfs.expect("other has a finite peak");
    assert!(p > -61.0 && p < -59.0, "{p}");
    // the snapshot a re-attaching UI gets carries the same list
    assert_eq!(rig.state.snapshot().unwrap().dropped, done.dropped);
    // the event too
    let ev = rig.events.lock().unwrap().iter().rev().find_map(|e| match e {
        ImportEvent::Saved { track, dropped } => Some((track.clone(), dropped.clone())),
        _ => None,
    });
    let (et, ed) = ev.expect("a Saved event");
    assert_eq!(ed, done.dropped);
    assert_eq!(et.id, t.id);
    // disk: track.json lists the 4, the folder holds exactly those 4 files, no .part
    let j = track_json(&rig, &t.id);
    let listed: Vec<String> = j["stems"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(listed, ["vocals", "drums", "bass", "guitar"]);
    for s in j["stems"].as_array().unwrap() {
        assert!(rig.root.join("tracks").join(&t.id).join(s["file"].as_str().unwrap()).is_file());
    }
    assert_eq!(stem_files(&rig, &t.id), ["bass.flac", "drums.flac", "guitar.flac", "vocals.flac"]);
    assert_eq!(j["type"], "stem");
    assert!(j["original"].is_null() || j.get("original").is_none());
    assert!(!rig.root.join("tracks").join(&t.id).join("original.flac").exists());
    // the quiet-but-above-threshold guitar (-45 dBFS) is byte-identical to the served stem
    assert_eq!(
        fs::read(rig.root.join("tracks").join(&t.id).join("stems/guitar.flac")).unwrap(),
        fs::read(fixture("stems-quiet/minus45.flac")).unwrap()
    );
    assert_clean_after_save(&rig);
    // the library lists the track and no problems
    let lib = Repository::new(&rig.root).scan();
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(lib.tracks.len(), 1);
    assert_eq!(lib.tracks[0].stems.len(), 4);
}

#[test]
fn sparse_import_with_keep_original_still_adopts_the_full_mix() {
    let server = RealServer::start("sparse");
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url, true);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let t = done.track.unwrap();
    assert_eq!(t.stems.len(), 4);
    assert_eq!(t.original.as_deref(), Some("original.flac"));
    let dir = rig.root.join("tracks").join(&t.id);
    assert!(dir.join("original.flac").is_file());
    assert!(fs::metadata(dir.join("original.flac")).unwrap().len() > 1000);
    assert_eq!(track_json(&rig, &t.id)["original"], "original.flac");
    assert_eq!(stem_files(&rig, &t.id).len(), 4);
    assert_clean_after_save(&rig);
}

#[test]
fn ok_import_keeps_all_six_and_reports_nothing_dropped() {
    let server = RealServer::start("ok");
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url, false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert!(done.dropped.is_empty());
    assert_eq!(done.track.clone().unwrap().stems.len(), 6);
    let j = serde_json::to_value(&done).unwrap();
    assert_eq!(j["dropped"], serde_json::json!([]), "the field is always serialised");
    assert_clean_after_save(&rig);
}

#[test]
fn saved_event_and_snapshot_serialise_dropped_for_the_ui() {
    let server = RealServer::start("sparse");
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url, false);
    let j = serde_json::to_value(&done).unwrap();
    assert_eq!(j["dropped"][0], serde_json::json!({"name": "piano", "peak_dbfs": null}));
    assert_eq!(j["dropped"][1]["name"], "other");
    assert!(j["dropped"][1]["peak_dbfs"].as_f64().unwrap() < -50.0);
    let ev = rig.events.lock().unwrap().iter().rev().find(|e| matches!(e, ImportEvent::Saved { .. })).cloned().unwrap();
    let ej = serde_json::to_value(&ev).unwrap();
    assert_eq!(ej["phase"], "saved");
    assert_eq!(ej["dropped"].as_array().unwrap().len(), 2);
}

#[test]
fn all_silent_import_fails_cleanly_with_the_exact_message() {
    let server = RealServer::start("silent");
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let src_before = fingerprint(&rig.sources);
    for keep in [false, true] {
        rig.file("untagged.flac").unwrap();
        let ready = rig.ready();
        rig.extract(&ready, &server.url, keep).unwrap();
        let s = rig.wait("failed", |s| s.phase == Phase::Failed && !rig.state.is_running());
        let e = s.error.clone().expect("an error");
        assert_eq!(e.stage, Stage::Server);
        assert_eq!(e.message, "Every stem is silent (below -50 dBFS), so no track was saved");
        assert_eq!(e.http_status, None);
        assert!(s.track.is_none());
        assert!(s.dropped.is_empty() || s.dropped.len() == 6, "{:?}", s.dropped);
        // nothing new in tracks, no staging, no original adopted; the prepared audio stays
        assert_eq!(rig.tracks().len(), v1_ids().len(), "{:?}", rig.tracks());
        assert!(rig.staging().is_empty());
        let tmp = rig.import_tmp();
        assert_eq!(tmp.len(), 1, "{tmp:?}");
        assert!(names_in(&rig.root.join("import-tmp").join(&tmp[0])).contains(&"audio.flac".to_string()));
        // existing tracks and the user's source are untouched
        let now = fingerprint(&rig.root);
        for (k, v) in &before {
            assert_eq!(now.get(k), Some(v), "{k}");
        }
        assert_eq!(fingerprint(&rig.sources), src_before);
        // the server job was deleted
        until("server job removed", 10, || server.job_dirs().is_empty());
        rig.state.discard(&s.job).unwrap();
        assert!(rig.import_tmp().is_empty(), "{:?}", rig.import_tmp());
    }
    rig.outside_ok();
}

#[test]
fn after_an_all_silent_failure_the_user_can_extract_again_and_succeed() {
    let silent = RealServer::start("silent");
    let ok = RealServer::start("sparse");
    let rig = Rig::new(false);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &silent.url, true).unwrap();
    let f = rig.wait("failed", |s| s.phase == Phase::Failed && !rig.state.is_running());
    assert_eq!(f.error.clone().unwrap().stage, Stage::Server);
    rig.extract(&f, &ok.url, true).unwrap();
    let s = rig.wait("saved", |s| s.phase == Phase::Saved);
    assert_eq!(s.track.unwrap().stems.len(), 4);
    assert_eq!(s.dropped.len(), 2, "dropped is reset for the new run");
    assert!(rig.staging().is_empty());
}

#[test]
fn boundary_and_undecodable_stems_through_a_whole_import() {
    let mut cfg = AiConfig::good();
    cfg.stems = ["vocals", "drums", "bass", "guitar", "piano", "other"].iter().map(|s| s.to_string()).collect();
    // 10 s with a single 50 ms note somewhere in the middle (guitar)
    let mut long = vec![0i32; 80_000];
    for i in 0..400 {
        long[41_000 + i] = ((i as f64 * 0.4).sin() * 3000.0) as i32;
    }
    let guitar = flac_bytes(&long, 1, 16, 8000);
    let vocals = fs::read(fixture("stems/vocals.flac")).unwrap();
    cfg.body = Arc::new(move |n| match n {
        "vocals" => vocals.clone(),
        "drums" => spike16(103, 8000, 100),  // -50.05: dropped
        "bass" => spike16(-104, 8000, 100),  // -49.97: kept
        "guitar" => guitar.clone(),
        "piano" => spike16(0, 8000, 0),      // zeros: dropped
        _ => b"fLaC-generic-body".to_vec(),   // undecodable: kept
    });
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url(), false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let t = done.track.unwrap();
    assert_eq!(kept_names(&t), ["vocals", "bass", "guitar", "other"]);
    assert_eq!(done.dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["drums", "piano"]);
    let db = done.dropped[0].peak_dbfs.unwrap();
    assert!((db + 50.05).abs() < 0.02, "{db}");
    assert_eq!(stem_files(&rig, &t.id), ["bass.flac", "guitar.flac", "other.flac", "vocals.flac"]);
    // the undecodable body was stored untouched
    assert_eq!(fs::read(rig.root.join("tracks").join(&t.id).join("stems/other.flac")).unwrap(), b"fLaC-generic-body");
    assert_clean_after_save(&rig);
}

#[test]
fn a_single_audible_stem_among_silent_ones_makes_a_one_stem_track() {
    let mut cfg = AiConfig::good();
    let vocals = fs::read(fixture("stems/vocals.flac")).unwrap();
    cfg.body = Arc::new(move |n| if n == "other" { vocals.clone() } else { spike16(0, 8000, 0) });
    let ai = fake_ai(cfg);
    let rig = Rig::new(false);
    let done = import_with(&rig, &ai.url(), false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    assert_eq!(kept_names(&done.track.unwrap()), ["other"]);
    assert_eq!(done.dropped.len(), 5);
    assert!(Repository::new(&rig.root).scan().problems.is_empty());
}

#[test]
fn existing_tracks_are_not_rewritten_or_remeasured() {
    let server = RealServer::start("sparse");
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    let done = import_with(&rig, &server.url, true);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let new_id = done.track.unwrap().id;
    let after = fingerprint(&rig.root);
    for (k, v) in &before {
        assert_eq!(after.get(k), Some(v), "pre-existing entry changed: {k}");
    }
    for k in after.keys() {
        assert!(before.contains_key(k) || k.starts_with(&format!("tracks/{new_id}")) || k == "import-tmp", "unexpected {k}");
    }
}

#[test]
fn cancel_while_the_check_runs_leaves_nothing() {
    // a slow trickle of a big undecodable stem gives a long Receiving phase; cancel in it,
    // then again after a silent import has just been refused
    let mut cfg = AiConfig::good();
    cfg.slow_stem_ms = 30;
    cfg.body = Arc::new(|_| spike16(0, 8000 * 30, 0));
    let ai = fake_ai(cfg);
    let rig = Rig::new(true);
    let before = fingerprint(&rig.root);
    rig.file("untagged.flac").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &ai.url(), true).unwrap();
    rig.wait("receiving", |s| s.phase == Phase::Receiving);
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    rig.state.cancel(&ready.job).unwrap();
    rig.wait("stopped", |_| !rig.state.is_running());
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert!(rig.staging().is_empty());
    assert_eq!(rig.tracks().len(), v1_ids().len());
    let s = rig.state.snapshot().unwrap();
    assert!(s.dropped.is_empty(), "dropped is cleared by a cancel: {:?}", s.dropped);
    let now = fingerprint(&rig.root);
    for (k, v) in &before {
        if !k.starts_with("import-tmp") {
            assert_eq!(now.get(k), Some(v), "{k}");
        }
    }
    rig.outside_ok();
}

#[test]
fn the_reduced_track_opens_in_the_editor_with_four_lanes() {
    let server = RealServer::start("sparse");
    let rig = Rig::new(false);
    let done = import_with(&rig, &server.url, false);
    let t = done.track.unwrap();
    let rec = Repository::new(&rig.root).load_record(&t.id).unwrap();
    assert!(rec.missing.is_empty(), "{:?}", rec.missing);
    let spec = editor::OpenSpec {
        dir: rig.root.join("tracks").join(&t.id),
        id: rec.id,
        title: rec.title,
        stems: rec.stems,
        missing: rec.missing,
        variant: rec.backings.into_iter().next(),
    };
    let backend = ManualBackend::new();
    let mut m = editor::EditorManager::new(Box::new(backend), Arc::new(|_| {}));
    let snap = m.open(&spec).expect("the 4-stem track opens");
    let lanes: Vec<&str> = snap.stems.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(lanes, ["vocals", "drums", "bass", "guitar"]);
}

fn v1_ids() -> Vec<String> {
    names_in(&project().join("tests/fixtures/library-sample/tracks"))
}
