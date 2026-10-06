//! Quick end-to-end checks of the real binary with the stub separator, on 127.0.0.1 only.
//! (The full protocol conformance suite is `conformance.rs`.)

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Server {
    child: Child,
    addr: SocketAddr,
    work: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_in(work: tempfile::TempDir, mode: &str, extra: &[&str]) -> Server {
    let sep = root().join("tests/support/stub-separator");
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .arg("--separator")
        .arg(sep)
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(work.path())
        .args(extra)
        .env("STUB_SEPARATOR_MODE", mode)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    let addr = loop {
        line.clear();
        assert!(lines.read_line(&mut line).unwrap() > 0, "server exited before listening");
        if let Some(rest) = line.split("listening addr=").nth(1) {
            break rest.split_whitespace().next().unwrap().parse().unwrap();
        }
    };
    // Keep draining stderr so the server never blocks on a full pipe.
    std::thread::spawn(move || {
        let mut sink = String::new();
        while lines.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });
    Server { child, addr, work }
}

fn start(mode: &str, extra: &[&str]) -> Server {
    start_in(tempfile::tempdir().unwrap(), mode, extra)
}

struct Resp {
    status: u16,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&self.body)))
    }
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
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("response head");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Resp { status, body: raw[split + 4..].to_vec() }
}

fn post(addr: SocketAddr, query: &str, file: &str) -> Resp {
    let body = std::fs::read(root().join("tests/fixtures/import").join(file)).unwrap();
    http(
        addr,
        "POST",
        &format!("/v1/jobs{query}"),
        &[("Content-Type", "audio/flac".into()), ("Content-Length", body.len().to_string())],
        &body,
    )
}

fn wait_state(addr: SocketAddr, job: &str, want: &str) -> serde_json::Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let v = http(addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").json();
        if v["state"] == want {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {want}: {v}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn health_and_error_statuses() {
    let s = start("ok", &[]);
    let h = http(s.addr, "GET", "/v1/health", &[], b"");
    assert_eq!(h.status, 200);
    let v = h.json();
    assert_eq!((v["service"].as_str(), v["api"].as_u64(), v["default_model"].as_str()), (Some("calliope-stems"), Some(1), Some("htdemucs_6s")));
    assert_eq!(v["max_upload_bytes"], 314_572_800u64);
    assert_eq!(http(s.addr, "POST", "/v1/jobs", &[("Content-Type", "audio/flac".into())], b"").status, 411);
    assert_eq!(post(s.addr, "", "not-flac.flac").status, 415);
    assert_eq!(post(s.addr, "", "long.flac").status, 413);
    assert_eq!(post(s.addr, "?model=nope", "untagged.flac").status, 400);
    assert_eq!(http(s.addr, "PUT", "/v1/jobs", &[], b"").status, 405);
    assert_eq!(http(s.addr, "GET", "/v1/nothing", &[], b"").status, 404);
    assert_eq!(http(s.addr, "GET", "/v1/jobs/abcd", &[], b"").status, 404);
    let r = http(s.addr, "POST", "/v1/jobs", &[("Content-Type", "audio/mpeg".into()), ("Content-Length", "4".into())], b"abcd");
    assert_eq!(r.status, 415);
    assert!(r.json()["error"].is_string());
}

#[test]
fn ok_flow_serves_the_stems_and_delete_removes_the_folder() {
    let s = start("ok", &[]);
    let r = post(s.addr, "?model=htdemucs_6s", "untagged.flac");
    assert_eq!(r.status, 202, "{:?}", String::from_utf8_lossy(&r.body));
    let job = r.json()["job"].as_str().unwrap().to_string();
    let done = wait_state(s.addr, &job, "done");
    let names: Vec<&str> = done["stems"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar", "piano", "other"]);
    let stem = http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/vocals"), &[], b"");
    assert_eq!(stem.status, 200);
    let want = std::fs::read(root().join("tests/fixtures/import/stems/vocals.flac")).unwrap();
    assert_eq!(stem.body, want);
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/nothing"), &[], b"").status, 404);
    let dir = s.work.path().join("jobs").join(&job);
    assert!(dir.exists());
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), &[], b"").status, 204);
    assert!(!dir.exists());
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status, 404);
}

#[test]
fn failing_separators_fail_the_job() {
    for mode in ["fail", "bad-output", "not-flac"] {
        let s = start(mode, &[]);
        let job = post(s.addr, "", "untagged.flac").json()["job"].as_str().unwrap().to_string();
        let v = wait_state(s.addr, &job, "failed");
        assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()), "{mode}: {v}");
        assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/vocals"), &[], b"").status, 409);
    }
}

#[test]
fn delete_kills_a_hanging_separator() {
    let s = start("hang", &[]);
    let job = post(s.addr, "", "untagged.flac").json()["job"].as_str().unwrap().to_string();
    wait_state(s.addr, &job, "running");
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/vocals"), &[], b"").status, 409);
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), &[], b"").status, 204);
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").json()["state"], "cancelled");
    assert!(!s.work.path().join("jobs").join(&job).exists());
    // The worker is free again.
    assert_eq!(http(s.addr, "GET", "/v1/health", &[], b"").json()["busy"], false);
}

#[test]
fn queue_limit_gives_503() {
    let s = start("hang", &["--queue", "1"]);
    assert_eq!(post(s.addr, "", "untagged.flac").status, 202);
    assert_eq!(post(s.addr, "", "untagged.flac").status, 202);
    assert_eq!(post(s.addr, "", "untagged.flac").status, 503);
}

#[test]
fn restart_forgets_jobs_and_removes_only_marked_folders() {
    let work = tempfile::tempdir().unwrap();
    let keep = work.path().join("jobs/not-ours");
    std::fs::create_dir_all(&keep).unwrap();
    std::fs::write(keep.join("keep.txt"), b"x").unwrap();
    std::fs::write(work.path().join("other.txt"), b"x").unwrap();
    let mut s = start_in(work, "ok", &[]);
    let job = post(s.addr, "", "untagged.flac").json()["job"].as_str().unwrap().to_string();
    wait_state(s.addr, &job, "done");
    // Restart on the same work dir.
    let _ = s.child.kill();
    let _ = s.child.wait();
    let work = std::mem::replace(&mut s.work, tempfile::tempdir().unwrap());
    let path = work.path().to_path_buf();
    let s2 = start_in(work, "ok", &[]);
    assert_eq!(http(s2.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status, 404);
    assert!(!path.join("jobs").join(&job).exists());
    assert!(keep.join("keep.txt").exists() && path.join("other.txt").exists());
}

#[test]
fn retention_zero_removes_finished_jobs() {
    let s = start("ok", &["--retention-hours", "0", "--janitor-interval-ms", "100"]);
    let job = post(s.addr, "", "untagged.flac").json()["job"].as_str().unwrap().to_string();
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if http(s.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status == 404 {
            break;
        }
        assert!(Instant::now() < end, "job was not removed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!s.work.path().join("jobs").join(&job).exists());
}

#[test]
fn separator_flag_is_required() {
    let out = Command::new(env!("CARGO_BIN_EXE_calliope-stems")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--separator"));
    let out = Command::new(env!("CARGO_BIN_EXE_calliope-stems")).arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("calliope-stems 0."));
}

#[test]
fn sigterm_stops_the_server_and_its_separator() {
    let mut s = start("hang", &[]);
    let job = post(s.addr, "", "untagged.flac").json()["job"].as_str().unwrap().to_string();
    wait_state(s.addr, &job, "running");
    let st = Command::new("kill").args(["-TERM", &s.child.id().to_string()]).status().unwrap();
    assert!(st.success());
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            assert!(status.success(), "{status:?}");
            break;
        }
        assert!(Instant::now() < end, "server did not exit after SIGTERM");
        std::thread::sleep(Duration::from_millis(50));
    }
}
