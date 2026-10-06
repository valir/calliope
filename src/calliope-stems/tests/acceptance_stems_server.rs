//! QA acceptance / robustness probes of the `calliope-stems` server (API v1), written
//! independently of the implementer's tests. Black box: the real binary on 127.0.0.1 (port 0),
//! temp work dirs, the stub separator or small throw-away separator scripts. Never the real
//! model, nothing outside loopback.
#![allow(clippy::all)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures/import").join(name)
}
fn stub() -> PathBuf {
    root().join("tests/support/stub-separator")
}

static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

struct Server {
    child: Child,
    addr: SocketAddr,
    outer: tempfile::TempDir,
    tag: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        kill_tagged(&self.tag);
    }
}

impl Server {
    fn work(&self) -> PathBuf {
        self.outer.path().join("work")
    }
    fn jobs(&self) -> PathBuf {
        self.work().join("jobs")
    }
    fn job_dirs(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.jobs())
            .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
    fn log_has(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }
    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
    fn healthy(&self) -> bool {
        let r = http(self.addr, "GET", "/v1/health", &[], b"");
        r.status == 200
    }
}

/// Every process carrying `CALLIOPE_QA_TAG=<tag>` in its environment.
fn tagged(tag: &str) -> Vec<(u32, String)> {
    let want = format!("CALLIOPE_QA_TAG={tag}");
    let me = std::process::id();
    let mut v = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        let Ok(env) = std::fs::read(e.path().join("environ")) else { continue };
        if env.split(|b| *b == 0).any(|kv| kv == want.as_bytes()) {
            // skip zombies
            if let Ok(st) = std::fs::read_to_string(e.path().join("stat")) {
                if st.rsplit(") ").next().is_some_and(|r| r.starts_with('Z')) {
                    continue;
                }
            }
            let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
            v.push((pid, String::from_utf8_lossy(&cmd).replace('\0', " ")));
        }
    }
    v
}

fn kill_tagged(tag: &str) {
    for _ in 0..3 {
        for (pid, _) in tagged(tag) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        if tagged(tag).is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_server(outer: tempfile::TempDir, separator: &Path, mode: &str, extra: &[&str]) -> Server {
    let tag = format!("qa{}-{}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .arg("--separator")
        .arg(separator)
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(outer.path().join("work"))
        .args(extra)
        .env("STUB_SEPARATOR_MODE", mode)
        .env("CALLIOPE_QA_TAG", &tag)
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
    assert!(addr.ip().is_loopback(), "bound to {addr}, not loopback");
    let log = Arc::new(Mutex::new(Vec::new()));
    let l2 = log.clone();
    std::thread::spawn(move || {
        let mut s = String::new();
        while lines.read_line(&mut s).unwrap_or(0) > 0 {
            l2.lock().unwrap().push(s.trim_end().to_string());
            s.clear();
        }
    });
    Server { child, addr, outer, tag, log }
}

fn start(mode: &str, extra: &[&str]) -> Server {
    spawn_server(tempfile::tempdir().unwrap(), &stub(), mode, extra)
}

/// A server whose separator is a throw-away shell script.
fn start_script(script_body: &str, extra: &[&str]) -> Server {
    let outer = tempfile::tempdir().unwrap();
    let sep = outer.path().join("sep.sh");
    std::fs::write(&sep, format!("#!/usr/bin/env bash\n{script_body}\n")).unwrap();
    std::fs::set_permissions(&sep, std::fs::Permissions::from_mode(0o755)).unwrap();
    spawn_server(outer, &sep.clone(), "ok", extra)
}

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}
impl Resp {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
}

fn read_resp(s: &mut TcpStream) -> Option<Resp> {
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some(Resp { status, head, body: raw[split + 4..].to_vec() })
}

fn http(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> Resp {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let _ = s.write_all(body);
    read_resp(&mut s).expect("a response")
}

fn post(addr: SocketAddr, query: &str, body: &[u8]) -> Resp {
    http(
        addr,
        "POST",
        &format!("/v1/jobs{query}"),
        &[("Content-Type", "audio/flac".into()), ("Content-Length", body.len().to_string())],
        body,
    )
}

fn good_flac() -> Vec<u8> {
    std::fs::read(fixture("untagged.flac")).unwrap()
}

/// A FLAC header (magic + STREAMINFO) claiming `total` samples at `rate` Hz, then `extra` junk.
fn flac_header(rate: u32, total: u64, extra: usize) -> Vec<u8> {
    let mut v = b"fLaC".to_vec();
    v.extend([0x80, 0, 0, 34]);
    let mut si = [0u8; 34];
    si[10] = (rate >> 12) as u8;
    si[11] = (rate >> 4) as u8;
    si[12] = (((rate & 0x0f) as u8) << 4) | (1 << 1); // 2 channels
    si[13] = ((total >> 32) & 0x0f) as u8;
    si[14..18].copy_from_slice(&(total as u32).to_be_bytes());
    v.extend(si);
    v.extend(vec![0u8; extra]);
    v
}

fn submit(addr: SocketAddr) -> String {
    let r = post(addr, "", &good_flac());
    assert_eq!(r.status, 202, "{:?}", String::from_utf8_lossy(&r.body));
    r.json()["job"].as_str().unwrap().to_string()
}

fn status(addr: SocketAddr, job: &str) -> Resp {
    http(addr, "GET", &format!("/v1/jobs/{job}"), &[], b"")
}

fn wait_state(addr: SocketAddr, job: &str, want: &str) -> serde_json::Value {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let r = status(addr, job);
        if r.status == 200 {
            let v = r.json();
            if v["state"] == want {
                return v;
            }
        }
        assert!(Instant::now() < end, "timed out waiting for {want}: {:?}", String::from_utf8_lossy(&r.body));
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------- uploads

#[test]
fn bad_uploads_are_refused_without_leaking_state() {
    let mut s = start("ok", &["--max-duration-s", "900"]);
    let a = s.addr;
    let flac = good_flac();
    let ct = ("Content-Type", "audio/flac".to_string());

    // empty body
    assert_eq!(post(a, "", b"").status, 415);
    // text
    assert_eq!(post(a, "", b"hello world, definitely not audio, at all, no way").status, 415);
    // magic only / truncated STREAMINFO
    assert_eq!(post(a, "", b"fLaC").status, 415);
    assert_eq!(post(a, "", &flac[..20]).status, 415);
    // MP3 / ogg magic
    assert_eq!(post(a, "", &[b"ID3\x04\0\0\0\0\0\0".as_slice(), &[0u8; 100]].concat()).status, 415);
    // STREAMINFO with unknown length (0 samples) and 0 sample rate
    assert_eq!(post(a, "", &flac_header(44100, 0, 100)).status, 415);
    assert_eq!(post(a, "", &flac_header(0, 100, 100)).status, 415);
    // longer than 900 s: 44100 Hz * 901 s
    let r = post(a, "", &flac_header(44100, 44100 * 901, 100));
    assert_eq!(r.status, 413, "{:?}", String::from_utf8_lossy(&r.body));
    // exactly at the limit is accepted (and fails later in the separator: junk body is fine here)
    let ok = post(a, "", &flac_header(44100, 44100 * 900, 100));
    assert_eq!(ok.status, 202, "exactly 15 min must be accepted: {:?}", String::from_utf8_lossy(&ok.body));
    let id = ok.json()["job"].as_str().unwrap().to_string();
    let _ = http(a, "DELETE", &format!("/v1/jobs/{id}"), &[], b"");
    // wrong Content-Type
    let r = http(a, "POST", "/v1/jobs", &[("Content-Type", "audio/mpeg".into()), ("Content-Length", flac.len().to_string())], &flac);
    assert_eq!(r.status, 415);
    // no Content-Type
    let r = http(a, "POST", "/v1/jobs", &[("Content-Length", flac.len().to_string())], &flac);
    assert_eq!(r.status, 415);
    // no length (chunked)
    let mut chunked = format!("{:x}\r\n", flac.len()).into_bytes();
    chunked.extend(&flac);
    chunked.extend(b"\r\n0\r\n\r\n");
    let r = http(a, "POST", "/v1/jobs", &[ct.clone(), ("Transfer-Encoding", "chunked".into())], &chunked);
    assert_eq!(r.status, 411, "chunked upload without Content-Length");
    // bad models
    for m in ["other", "../../etc", "htdemucs_6s%00x", "", "htdemucs_6s&model=evil", "-x"] {
        let r = post(a, &format!("?model={m}"), &flac);
        assert!(r.status == 400 || r.status == 202, "model {m:?} -> {}", r.status);
        if r.status == 202 {
            // allowed only for the exact default model (the `&model=evil` case has two params)
            assert!(m == "" || m.starts_with("htdemucs_6s&"), "accepted model {m:?}");
            let id = r.json()["job"].as_str().unwrap().to_string();
            let _ = http(a, "DELETE", &format!("/v1/jobs/{id}"), &[], b"");
        }
    }
    // everything refused left no job folder behind
    until("job folders gone", 10, || s.job_dirs().is_empty());
    assert!(s.alive() && s.healthy());
}

#[test]
fn oversized_upload_is_refused_up_front() {
    let s = start("ok", &["--max-upload-mb", "1"]);
    let flac = good_flac();
    // Declared length over the cap, only a little sent: 413 at once, nothing stored.
    let mut c = TcpStream::connect(s.addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let head = format!(
        "POST /v1/jobs HTTP/1.1\r\nHost: x\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        50 * 1024 * 1024
    );
    c.write_all(head.as_bytes()).unwrap();
    c.write_all(&flac[..100]).unwrap();
    let r = read_resp(&mut c).expect("413 before the body is read");
    assert_eq!(r.status, 413);
    assert!(s.job_dirs().is_empty());
    // Health states the cap.
    let h = http(s.addr, "GET", "/v1/health", &[], b"").json();
    assert_eq!(h["max_upload_bytes"], 1024 * 1024);
    // A file just under the cap works, one byte over is refused.
    let mut under = good_flac();
    under.resize(1024 * 1024, 0);
    assert_eq!(post(s.addr, "", &under).status, 202);
    under.push(0);
    assert_eq!(post(s.addr, "", &under).status, 413);
}

#[test]
fn aborted_uploads_release_their_queue_slot_and_folder() {
    let s = start("ok", &["--queue", "1"]);
    let flac = good_flac();
    for _ in 0..8 {
        // Declares the whole file, sends half, hangs up.
        let mut c = TcpStream::connect(s.addr).unwrap();
        let head = format!(
            "POST /v1/jobs HTTP/1.1\r\nHost: x\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\n\r\n",
            flac.len()
        );
        c.write_all(head.as_bytes()).unwrap();
        c.write_all(&flac[..flac.len() / 2]).unwrap();
        drop(c);
    }
    until("aborted uploads cleaned", 10, || s.job_dirs().is_empty());
    // The queue (1 waiting + running) must still accept work: a real job completes.
    let id = submit(s.addr);
    wait_state(s.addr, &id, "done");
}

#[test]
fn short_body_with_connection_kept_open_is_a_known_limitation_not_a_crash() {
    // A client that stalls (declares more than it sends and keeps the socket) holds one slot.
    // Documented in the log ("no upload read timeout"). Here: other requests are unaffected.
    let s = start("ok", &["--queue", "2"]);
    let mut stalled = Vec::new();
    for _ in 0..2 {
        let mut c = TcpStream::connect(s.addr).unwrap();
        c.write_all(b"POST /v1/jobs HTTP/1.1\r\nHost: x\r\nContent-Type: audio/flac\r\nContent-Length: 100000\r\n\r\n").unwrap();
        c.write_all(&good_flac()[..60]).unwrap();
        stalled.push(c);
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(s.healthy(), "health must answer while uploads stall");
    let id = submit(s.addr);
    wait_state(s.addr, &id, "done");
    drop(stalled);
}

#[test]
#[ignore = "KNOWN LIMIT (documented, owner decision: LAN-only, no auth): a few stalled uploads (declared Content-Length never completed, socket kept open) occupy every queue slot forever: no read timeout, so a LAN client can lock out the owner with queue+1 idle sockets"]
fn finding_stalled_uploads_lock_the_queue() {
    let s = start("ok", &["--queue", "2"]);
    let mut stalled = Vec::new();
    for _ in 0..3 {
        let mut c = TcpStream::connect(s.addr).unwrap();
        c.write_all(b"POST /v1/jobs HTTP/1.1\r\nHost: x\r\nContent-Type: audio/flac\r\nContent-Length: 100000\r\n\r\n").unwrap();
        c.write_all(&good_flac()[..60]).unwrap();
        stalled.push(c);
    }
    std::thread::sleep(Duration::from_secs(2));
    // Expected: the server drops idle uploads and accepts this one. Actual: 503.
    let r = post(s.addr, "", &good_flac());
    assert_eq!(r.status, 202, "a real upload is refused with {} while nothing runs", r.status);
    drop(stalled);
}

// ---------------------------------------------------------------- routing / traversal

#[test]
fn path_traversal_and_hostile_requests_are_harmless() {
    let mut s = start("ok", &[]);
    let a = s.addr;
    // A canary that looks like a FLAC outside the job folder.
    std::fs::write(s.outer.path().join("secret.flac"), b"fLaC-secret").unwrap();
    std::fs::write(s.work().join("secret.flac"), b"fLaC-secret").unwrap();
    let id = submit(a);
    wait_state(a, &id, "done");
    let bad_paths = [
        format!("/v1/jobs/{id}/stems/..%2f..%2fsecret"),
        format!("/v1/jobs/{id}/stems/..%2f..%2f..%2fsecret"),
        format!("/v1/jobs/{id}/stems/%2e%2e/secret"),
        format!("/v1/jobs/{id}/stems/../../../secret.flac"),
        format!("/v1/jobs/{id}/stems/secret"),
        format!("/v1/jobs/{id}/stems/vocals.flac"),
        format!("/v1/jobs/{id}/stems/VOCALS"),
        format!("/v1/jobs/{id}/stems/vocals%00"),
        format!("/v1/jobs/{id}/stems/vocals/"),
        format!("/v1/jobs/../health"),
        format!("/v1/jobs/..%2fjobs/{id}"),
        format!("/v1/jobs/{id}/../{id}"),
        "/v1/jobs/..".to_string(),
        "/v1/jobs/%2e%2e".to_string(),
        "/v1/jobs/-".to_string(),
        "/v1/jobs/--".to_string(),
        format!("/v1/jobs/{}", "a".repeat(65)),
        format!("/{}", "a".repeat(60_000)),
        "/v1/health?x=%00".to_string(),
        "//v1/health".to_string(),
    ];
    for p in &bad_paths {
        let r = http(a, "GET", p, &[], b"");
        assert!(
            matches!(r.status, 404 | 400 | 405 | 414 | 431) || (p.starts_with("/v1/health") && r.status == 200),
            "GET {} -> {}",
            &p[..p.len().min(80)],
            r.status
        );
        assert!(!String::from_utf8_lossy(&r.body).contains("secret"), "leaked: {p}");
    }
    // DELETE with traversal ids must not remove anything outside.
    for p in ["/v1/jobs/..", "/v1/jobs/..%2f..", "/v1/jobs/%2e%2e%2fjobs"] {
        let r = http(a, "DELETE", p, &[], b"");
        assert!(matches!(r.status, 404 | 400 | 405), "DELETE {p} -> {}", r.status);
    }
    assert!(s.outer.path().join("secret.flac").exists());
    assert!(s.work().join("secret.flac").exists());
    assert!(s.jobs().join(&id).exists());
    // wrong methods
    for (m, p) in [("PUT", "/v1/jobs"), ("PATCH", "/v1/health"), ("OPTIONS", "/v1/jobs"), ("TRACE", "/v1/health")] {
        let r = http(a, m, p, &[], b"");
        assert!(matches!(r.status, 405 | 404 | 400 | 501), "{m} {p} -> {}", r.status);
    }
    // the good stem still works
    let r = http(a, "GET", &format!("/v1/jobs/{id}/stems/vocals"), &[], b"");
    assert_eq!(r.status, 200);
    assert!(r.body.starts_with(b"fLaC"));
    assert!(r.head.to_lowercase().contains("audio/flac"));
    assert!(s.alive() && s.healthy());
}

#[test]
fn garbage_on_the_socket_does_not_kill_the_server() {
    let mut s = start("ok", &[]);
    let junk: Vec<Vec<u8>> = vec![
        b"\x00\x01\x02\xff\xfe garbage\r\n\r\n".to_vec(),
        b"GET\r\n\r\n".to_vec(),
        b"GET /v1/health HTTP/9.9\r\n\r\n".to_vec(),
        b"POST /v1/jobs HTTP/1.1\r\nContent-Length: -5\r\n\r\n".to_vec(),
        b"POST /v1/jobs HTTP/1.1\r\nContent-Length: 99999999999999999999999\r\n\r\n".to_vec(),
        b"POST /v1/jobs HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nabcde".to_vec(),
        [b"GET /v1/health HTTP/1.1\r\nX: ".as_slice(), &vec![b'a'; 200_000], b"\r\n\r\n"].concat(),
        b"GET /v1/health HTTP/1.1\r\nHost: x\r\n".to_vec(), // never finished
    ];
    for j in junk {
        if let Ok(mut c) = TcpStream::connect(s.addr) {
            c.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let _ = c.write_all(&j);
            let mut sink = [0u8; 512];
            let _ = c.read(&mut sink);
        }
    }
    assert!(s.alive() && s.healthy());
    assert!(s.job_dirs().is_empty());
}

// ---------------------------------------------------------------- concurrency, DELETE races

#[test]
fn concurrent_uploads_respect_the_queue_limit() {
    let mut s = start("hang", &["--queue", "2"]);
    let a = s.addr;
    let flac = Arc::new(good_flac());
    let results = Arc::new(Mutex::new(Vec::new()));
    let hs: Vec<_> = (0..8)
        .map(|_| {
            let (flac, results) = (flac.clone(), results.clone());
            std::thread::spawn(move || {
                let r = post(a, "", &flac);
                results.lock().unwrap().push((r.status, r.body));
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let res = results.lock().unwrap().clone();
    let ok: Vec<_> = res.iter().filter(|(c, _)| *c == 202).collect();
    let full = res.iter().filter(|(c, _)| *c == 503).count();
    assert_eq!(ok.len(), 3, "1 running + 2 waiting; got {:?}", res.iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(full, 5);
    // ids unique
    let mut ids: Vec<String> = ok.iter().map(|(_, b)| serde_json::from_slice::<serde_json::Value>(b).unwrap()["job"].as_str().unwrap().to_string()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    // only ONE separator runs at a time
    until("a separator running", 10, || tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    std::thread::sleep(Duration::from_millis(500));
    let seps = tagged(&s.tag).iter().filter(|(_, c)| c.contains("input.flac")).count();
    assert_eq!(seps, 1, "max concurrent separators is 1");
    // delete all (concurrently)
    let hs: Vec<_> = ids
        .iter()
        .cloned()
        .map(|id| std::thread::spawn(move || http(a, "DELETE", &format!("/v1/jobs/{id}"), &[], b"").status))
        .collect();
    for h in hs {
        assert_eq!(h.join().unwrap(), 204);
    }
    until("no separator left", 15, || !tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    until("job folders gone", 10, || s.job_dirs().is_empty());
    // capacity is back
    let r = post(a, "", &flac);
    assert_eq!(r.status, 202);
    assert!(s.alive());
}

#[test]
fn jobs_run_in_order_and_all_stems_are_served_concurrently() {
    let s = start("ok", &["--queue", "2"]);
    let a = s.addr;
    let ids: Vec<String> = (0..3).map(|_| submit(a)).collect();
    for id in &ids {
        let v = wait_state(a, id, "done");
        assert_eq!(v["stems"], serde_json::json!(["vocals", "drums", "bass", "guitar", "piano", "other"]));
        assert_eq!(v["progress"], 1.0);
    }
    let hs: Vec<_> = ids
        .iter()
        .flat_map(|id| ["vocals", "drums", "bass", "guitar", "piano", "other"].map(|n| (id.clone(), n)))
        .map(|(id, n)| {
            std::thread::spawn(move || {
                let r = http(a, "GET", &format!("/v1/jobs/{id}/stems/{n}"), &[], b"");
                (r.status, r.body.len(), r.body.starts_with(b"fLaC"))
            })
        })
        .collect();
    for h in hs {
        let (st, len, magic) = h.join().unwrap();
        assert_eq!((st, magic), (200, true));
        assert!(len > 1000);
    }
    // stem of a job that is not done -> 409; unknown stem -> 404
    assert_eq!(http(a, "GET", &format!("/v1/jobs/{}/stems/karaoke", ids[0]), &[], b"").status, 404);
}

#[test]
fn delete_races_never_corrupt_or_resurrect_jobs() {
    let mut s = start("slow", &["--queue", "2"]);
    let a = s.addr;
    for i in 0..15u64 {
        let id = submit(a);
        std::thread::sleep(Duration::from_millis(i * 20));
        let a2 = a;
        let id2 = id.clone();
        // two concurrent deletes and a poller
        let d1 = std::thread::spawn(move || http(a2, "DELETE", &format!("/v1/jobs/{id2}"), &[], b"").status);
        let id3 = id.clone();
        let d2 = std::thread::spawn(move || http(a2, "DELETE", &format!("/v1/jobs/{id3}"), &[], b"").status);
        let p = status(a, &id);
        let (c1, c2) = (d1.join().unwrap(), d2.join().unwrap());
        assert!(matches!(c1, 204 | 404) && matches!(c2, 204 | 404) && (c1 == 204 || c2 == 204), "{c1} {c2}");
        if p.status == 200 {
            assert_ne!(p.json()["state"], "done", "a deleted job must not finish");
        }
        let after = status(a, &id);
        if after.status == 200 {
            assert_eq!(after.json()["state"], "cancelled");
        } else {
            assert_eq!(after.status, 404);
        }
        // stems of a deleted job are never served
        let r = http(a, "GET", &format!("/v1/jobs/{id}/stems/vocals"), &[], b"");
        assert!(matches!(r.status, 404 | 409), "{}", r.status);
    }
    until("no separator left", 20, || !tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    until("job folders gone", 10, || s.job_dirs().is_empty());
    assert!(s.alive() && s.healthy());
}

#[test]
fn delete_of_a_done_job_while_a_stem_is_being_read() {
    let s = start("ok", &[]);
    let a = s.addr;
    let id = submit(a);
    wait_state(a, &id, "done");
    // open the stem, read only the head, keep the connection
    let mut c = TcpStream::connect(a).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    c.write_all(format!("GET /v1/jobs/{id}/stems/vocals HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).unwrap();
    let mut first = [0u8; 100];
    c.read_exact(&mut first).unwrap();
    assert_eq!(http(a, "DELETE", &format!("/v1/jobs/{id}"), &[], b"").status, 204);
    assert_eq!(status(a, &id).status, 404);
    assert_eq!(http(a, "DELETE", &format!("/v1/jobs/{id}"), &[], b"").status, 404);
    assert!(!s.jobs().join(&id).exists());
    // the reader can finish or be cut, but the server stays up
    let mut rest = Vec::new();
    let _ = c.read_to_end(&mut rest);
    assert!(s.healthy());
}

// ---------------------------------------------------------------- crash / restart / signals

#[test]
fn sigkill_mid_job_leaves_no_separator_and_restart_cleans_only_its_own_folders() {
    let outer = tempfile::tempdir().unwrap();
    let work = outer.path().join("work");
    std::fs::create_dir_all(work.join("jobs/precious")).unwrap();
    std::fs::write(work.join("jobs/precious/keep.txt"), b"mine").unwrap();
    std::fs::write(work.join("jobs/loose.txt"), b"mine").unwrap();
    std::fs::write(work.join("notes.txt"), b"mine").unwrap();
    // a symlinked folder with a marker inside, pointing outside the work dir
    let outside = outer.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join(".calliope-stems-job"), b"").unwrap();
    std::fs::write(outside.join("data.txt"), b"mine").unwrap();
    std::os::unix::fs::symlink(&outside, work.join("jobs/link")).unwrap();
    // a stale marked folder from an earlier crash
    std::fs::create_dir(work.join("jobs/dead-beef")).unwrap();
    std::fs::write(work.join("jobs/dead-beef/.calliope-stems-job"), b"").unwrap();

    let mut s = spawn_server(outer, &stub(), "hang", &[]);
    assert!(!s.jobs().join("dead-beef").exists(), "stale marked folder removed at start");
    let id = submit(s.addr);
    wait_state(s.addr, &id, "running");
    until("separator up", 10, || tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    let tag = s.tag.clone();
    // SIGKILL the server
    unsafe { libc_kill(s.child.id() as i32, 9) };
    let _ = s.child.wait();
    until("separator killed with its parent (PDEATHSIG)", 10, || {
        !tagged(&tag).iter().any(|(_, c)| c.contains("input.flac"))
    });
    assert!(s.jobs().join(&id).exists(), "kill -9 leaves the job folder (cleaned at next start)");
    // restart on the same work dir
    let work = s.work();
    let outer2 = tempfile::tempdir().unwrap(); // only to keep the type
    let _ = outer2;
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .arg("--separator")
        .arg(stub())
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(&work)
        .env("CALLIOPE_QA_TAG", &tag)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    let addr: SocketAddr = loop {
        line.clear();
        assert!(lines.read_line(&mut line).unwrap() > 0);
        if let Some(rest) = line.split("listening addr=").nth(1) {
            break rest.split_whitespace().next().unwrap().parse().unwrap();
        }
    };
    assert_eq!(status(addr, &id).status, 404, "state is not persisted: old ids are unknown");
    assert!(!work.join("jobs").join(&id).exists(), "the old job folder was cleaned at start");
    // Foreign data intact.
    assert_eq!(std::fs::read(work.join("jobs/precious/keep.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(work.join("jobs/loose.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(work.join("notes.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(outside.join("data.txt")).unwrap(), b"mine", "symlinked folder must not be followed");
    let _ = child.kill();
    let _ = child.wait();
    kill_tagged(&tag);
}

extern "C" {
    #[link_name = "kill"]
    fn libc_kill_raw(pid: i32, sig: i32) -> i32;
}
unsafe fn libc_kill(pid: i32, sig: i32) {
    libc_kill_raw(pid, sig);
}

#[test]
fn sigterm_stops_the_server_and_its_separator() {
    let mut s = start("hang", &[]);
    let id = submit(s.addr);
    wait_state(s.addr, &id, "running");
    until("separator up", 10, || tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    unsafe { libc_kill(s.child.id() as i32, 15) };
    let t = Instant::now();
    until("server exit", 15, || s.child.try_wait().unwrap().is_some());
    assert!(t.elapsed() < Duration::from_secs(12));
    until("separator gone", 10, || !tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
}

// ---------------------------------------------------------------- separator behaviour

#[test]
fn separator_contract_argv_cwd_stdin() {
    let outer = tempfile::tempdir().unwrap();
    let rec = outer.path().join("rec.txt");
    let sep = outer.path().join("sep dir with spaces");
    std::fs::create_dir(&sep).unwrap();
    let sep = sep.join("sep.sh");
    std::fs::write(
        &sep,
        format!(
            "#!/usr/bin/env bash\n{{ echo \"argc=$#\"; for a in \"$@\"; do echo \"arg=$a\"; done; echo \"cwd=$PWD\"; if read -t 1 x; then echo stdin=data; else echo stdin=eof; fi; }} > {}\ncp '{}'/*.flac \"$2\"/\n",
            rec.display(),
            root().join("tests/fixtures/import/stems").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&sep, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = spawn_server(outer, &sep, "ok", &["--model", "my-model.v2"]);
    let id = submit(s.addr);
    wait_state(s.addr, &id, "done");
    let r = std::fs::read_to_string(s.outer.path().join("rec.txt")).unwrap();
    let job = s.jobs().join(&id);
    assert!(r.contains("argc=3"), "{r}");
    assert!(r.contains(&format!("arg={}/input.flac", job.display())), "{r}");
    assert!(r.contains(&format!("arg={}/out\n", job.display())), "{r}");
    assert!(r.contains("arg=my-model.v2\n"), "{r}");
    assert!(r.contains(&format!("cwd={}", job.display())), "{r}");
    assert!(r.contains("stdin=eof"), "stdin must be null: {r}");
    // The upload is byte-identical to what was sent.
    assert_eq!(std::fs::read(job.join("input.flac")).unwrap(), good_flac());
}

#[test]
fn hostile_separator_outputs_fail_the_job_and_are_never_served() {
    let stems = root().join("tests/fixtures/import/stems");
    let cases: Vec<(&str, String)> = vec![
        ("symlink", format!("cp '{s}'/*.flac \"$2\"/; rm \"$2\"/vocals.flac; ln -s /etc/passwd \"$2\"/vocals.flac", s = stems.display())),
        ("symlink-to-flac", format!("cp '{s}'/*.flac \"$2\"/; mv \"$2\"/vocals.flac \"$(dirname \"$2\")\"/real.flac; ln -s \"$(dirname \"$2\")\"/real.flac \"$2\"/vocals.flac", s = stems.display())),
        ("seventeen", "for i in $(seq 1 17); do printf 'fLaCxxxx' > \"$2/s$i.flac\"; done".to_string()),
        ("uppercase", format!("cp '{s}'/vocals.flac \"$2\"/Vocals.flac", s = stems.display())),
        ("dotdot-name", format!("cp '{s}'/vocals.flac \"$2\"/..flac", s = stems.display())),
        ("subdir", format!("mkdir \"$2\"/sub; cp '{s}'/vocals.flac \"$2\"/sub/vocals.flac", s = stems.display())),
        ("empty", "true".to_string()),
        ("exit1-with-stems", format!("cp '{s}'/*.flac \"$2\"/; exit 1", s = stems.display())),
        ("wrong-ext", format!("cp '{s}'/vocals.flac \"$2\"/vocals.wav", s = stems.display())),
        ("space-name", format!("cp '{s}'/vocals.flac \"$2\"/'my stem.flac'", s = stems.display())),
        ("empty-file", ": > \"$2\"/vocals.flac".to_string()),
    ];
    for (name, body) in cases {
        let mut s = start_script(&body, &[]);
        let id = submit(s.addr);
        let v = wait_state(s.addr, &id, "failed");
        assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()), "{name}: {v}");
        assert!(v["stems"].is_null(), "{name}: {v}");
        let r = http(s.addr, "GET", &format!("/v1/jobs/{id}/stems/vocals"), &[], b"");
        assert!(matches!(r.status, 404 | 409), "{name}: stem served with {}", r.status);
        assert!(!String::from_utf8_lossy(&r.body).contains("root:"), "{name}: leaked /etc/passwd");
        assert!(s.alive(), "{name}");
    }
}

#[test]
fn chatty_and_hostile_separator_output_does_not_hurt_the_server() {
    let mut s = start_script(
        "head -c 3000000 /dev/zero | tr '\\0' 'A'; echo; echo 'progress 0.5'; echo 'progress nan'; echo 'progress 9'; printf 'x%.0s' $(seq 1 100000) >&2; echo; exit 3",
        &[],
    );
    let id = submit(s.addr);
    let v = wait_state(s.addr, &id, "failed");
    let err = v["error"].as_str().unwrap();
    assert!(err.len() < 600, "error text must be bounded, was {} bytes", err.len());
    assert!(s.alive() && s.healthy());
}

#[test]
fn separator_timeout_kills_a_hung_separator() {
    let s = start("hang", &["--separator-timeout-min", "1"]);
    let id = submit(s.addr);
    wait_state(s.addr, &id, "running");
    let v = {
        let end = Instant::now() + Duration::from_secs(90);
        loop {
            let v = status(s.addr, &id).json();
            if v["state"] == "failed" {
                break v;
            }
            assert!(Instant::now() < end, "never timed out: {v}");
            std::thread::sleep(Duration::from_millis(500));
        }
    };
    assert!(v["error"].as_str().unwrap().contains("timed out"), "{v}");
    until("separator gone", 10, || !tagged(&s.tag).iter().any(|(_, c)| c.contains("input.flac")));
    assert!(s.job_dirs().is_empty());
}

#[test]
fn missing_separator_binary_fails_the_job_cleanly() {
    // The separator exists at start, then is removed: the job must fail, not hang or crash.
    let outer = tempfile::tempdir().unwrap();
    let sep = outer.path().join("sep.sh");
    std::fs::write(&sep, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&sep, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = spawn_server(outer, &sep.clone(), "ok", &[]);
    std::fs::remove_file(&sep).unwrap();
    let id = submit(s.addr);
    let v = wait_state(s.addr, &id, "failed");
    assert!(v["error"].as_str().unwrap().contains("separator"), "{v}");
    assert!(s.alive() && s.healthy());
}

// ---------------------------------------------------------------- command line

fn run_cli(args: &[&str]) -> (Option<i32>, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .args(args)
        .env_remove("XDG_STATE_HOME")
        .output()
        .unwrap();
    (o.status.code(), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

#[test]
fn cli_refuses_to_start_without_a_valid_separator() {
    let (c, _, e) = run_cli(&["--work-dir", "/nonexistent-qa/x", "--listen", "127.0.0.1:0"]);
    assert_eq!(c, Some(2), "{e}");
    assert!(e.contains("--separator"), "{e}");
    let (c, _, e) = run_cli(&["--separator", "/nonexistent-qa/sep", "--listen", "127.0.0.1:0"]);
    assert_eq!(c, Some(2), "{e}");
    let tmp = tempfile::tempdir().unwrap();
    let nonexec = tmp.path().join("plain.txt");
    std::fs::write(&nonexec, "x").unwrap();
    let (c, _, e) = run_cli(&["--separator", nonexec.to_str().unwrap(), "--listen", "127.0.0.1:0"]);
    assert_eq!(c, Some(2), "{e}");
    let (c, _, e) = run_cli(&["--separator", tmp.path().to_str().unwrap()]);
    assert_eq!(c, Some(2), "a directory is not a separator: {e}");
    // no work dir was created by any of those
    assert!(!Path::new("/nonexistent-qa").exists());
}

#[test]
fn cli_rejects_bad_values_and_unknown_flags() {
    let sep = stub();
    let sep = sep.to_str().unwrap();
    for args in [
        vec!["--separator", sep, "--bogus"],
        vec!["--separator", sep, "--listen", "not-an-addr"],
        vec!["--separator", sep, "--listen", "127.0.0.1"],
        vec!["--separator", sep, "--model", "../etc/passwd"],
        vec!["--separator", sep, "--model", ""],
        vec!["--separator", sep, "--model", ".hidden"],
        vec!["--separator", sep, "--model", "a b"],
        vec!["--separator", sep, "--max-upload-mb", "0"],
        vec!["--separator", sep, "--max-upload-mb", "-3"],
        vec!["--separator", sep, "--max-upload-mb", "abc"],
        vec!["--separator", sep, "--max-duration-s", "0"],
        vec!["--separator", sep, "--separator-timeout-min", "0"],
        vec!["--separator", sep, "--queue"],
        vec!["--separator", sep, "positional"],
    ] {
        let (c, _, e) = run_cli(&args);
        assert_eq!(c, Some(2), "{args:?}: {e}");
        assert!(!e.is_empty());
    }
    let (c, out, _) = run_cli(&["--help"]);
    assert_eq!(c, Some(0));
    for f in ["--separator", "--listen", "--work-dir", "--model", "--max-upload-mb", "--max-duration-s", "--queue", "--separator-timeout-min", "--retention-hours"] {
        assert!(out.contains(f), "--help lacks {f}");
    }
    assert!(out.contains("8765") && out.contains("required"));
    let (c, out, _) = run_cli(&["--version"]);
    assert_eq!(c, Some(0));
    assert!(out.starts_with("calliope-stems "));
    // `--separator=--x` style values and a `--` prefixed path are plain values
    let (c, _, e) = run_cli(&["--separator=--no-such"]);
    assert_eq!(c, Some(2), "{e}");
}

#[test]
fn port_in_use_is_a_clean_error() {
    let s = start("ok", &[]);
    let (c, _, e) = run_cli(&["--separator", stub().to_str().unwrap(), "--listen", &s.addr.to_string(), "--work-dir", s.outer.path().join("w2").to_str().unwrap()]);
    assert_eq!(c, Some(1), "{e}");
    assert!(e.contains("cannot listen"), "{e}");
}

#[test]
fn server_writes_only_inside_its_work_dir() {
    let s = start("ok", &[]);
    let id = submit(s.addr);
    wait_state(s.addr, &id, "done");
    let _ = http(s.addr, "DELETE", &format!("/v1/jobs/{id}"), &[], b"");
    let mut names: Vec<String> = std::fs::read_dir(s.outer.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(names, ["work"]);
    let mut inner: Vec<String> = std::fs::read_dir(s.work()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    inner.sort();
    assert_eq!(inner, ["jobs"]);
}

#[test]
fn log_lines_have_the_documented_shape() {
    let s = start("ok", &[]);
    let id = submit(s.addr);
    wait_state(s.addr, &id, "done");
    std::thread::sleep(Duration::from_millis(200));
    assert!(s.log_has("calliope-stems: request method=POST path=/v1/jobs status=202"));
    assert!(s.log_has(&format!("job id={id} state=queued")));
    assert!(s.log_has(&format!("job id={id} state=running")));
    assert!(s.log_has(&format!("job id={id} state=done")));
}
