//! Protocol conformance suite for "calliope-stems API v1": the REAL `calliope-stems` binary on
//! 127.0.0.1 (port 0, temp work dir, the stub separator, never the real model), exercised with
//! raw HTTP over `TcpStream` and with `calliope_lib::stems_client`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use calliope_lib::stems_client::{ClientError, StemsClient};

fn root() -> PathBuf {
    // The shared test fixtures and the stub separator live in the GUI crate's tests/.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../calliope-gui")
}

fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures/import").join(name)
}

/// A running server; killed when dropped.
struct Server {
    child: Child,
    addr: SocketAddr,
    /// Parent of the work dir: nothing but `work` may ever appear in here.
    outer: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn work(&self) -> PathBuf {
        self.outer.path().join("work")
    }
    fn jobs(&self) -> PathBuf {
        self.work().join("jobs")
    }
    fn client(&self) -> StemsClient {
        StemsClient::new(&format!("http://{}", self.addr))
    }
}

fn start_in(outer: tempfile::TempDir, mode: &str, extra: &[&str]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .arg("--separator")
        .arg(root().join("tests/support/stub-separator"))
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(outer.path().join("work"))
        .args(extra)
        .env("STUB_SEPARATOR_MODE", mode)
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
    std::thread::spawn(move || {
        let mut sink = String::new();
        while lines.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });
    Server { child, addr, outer }
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

fn post_file(addr: SocketAddr, query: &str, file: &str) -> Resp {
    let body = std::fs::read(fixture(file)).unwrap();
    http(
        addr,
        "POST",
        &format!("/v1/jobs{query}"),
        &[("Content-Type", "audio/flac".into()), ("Content-Length", body.len().to_string())],
        &body,
    )
}

fn raw_status(addr: SocketAddr, job: &str) -> serde_json::Value {
    http(addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").json()
}

fn wait_state(addr: SocketAddr, job: &str, want: &str) -> serde_json::Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let v = raw_status(addr, job);
        if v["state"] == want {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {want}: {v}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn submit_raw(s: &Server) -> String {
    let r = post_file(s.addr, "", "untagged.flac");
    assert_eq!(r.status, 202, "{:?}", String::from_utf8_lossy(&r.body));
    r.json()["job"].as_str().unwrap().to_string()
}

/// Pids of processes whose command line mentions `needle` (the job id is part of the stub's
/// input path).
fn pids_mentioning(needle: &str) -> Vec<u32> {
    let me = std::process::id();
    let mut out = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        if let Ok(cmd) = std::fs::read(e.path().join("cmdline")) {
            if String::from_utf8_lossy(&cmd).contains(needle) {
                out.push(pid);
            }
        }
    }
    out
}

fn wait_until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Only `work` may be in the server's parent directory, and only `jobs` (and nothing outside
/// the documented layout) in the work dir.
fn assert_confined(s: &Server) {
    let names = |d: &Path| {
        let mut v: Vec<String> =
            std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(names(s.outer.path()), ["work"]);
    for n in names(&s.work()) {
        assert!(n == "jobs", "unexpected entry in the work dir: {n}");
    }
}

#[test]
fn health_has_the_documented_fields() {
    let s = start("ok", &[]);
    let h = http(s.addr, "GET", "/v1/health", &[], b"");
    assert_eq!(h.status, 200);
    let v = h.json();
    assert_eq!(v["service"], "calliope-stems");
    assert_eq!(v["api"], 1);
    assert!(v["version"].as_str().is_some_and(|x| !x.is_empty()));
    assert_eq!(v["models"], serde_json::json!(["htdemucs_6s"]));
    assert_eq!(v["default_model"], "htdemucs_6s");
    assert_eq!(v["busy"], false);
    assert_eq!(v["max_duration_s"], 900);
    assert_eq!(v["max_upload_bytes"], 314_572_800u64);

    let c = s.client().health().unwrap();
    assert_eq!((c.service.as_str(), c.api, c.default_model.as_str()), ("calliope-stems", 1, "htdemucs_6s"));
    assert_eq!(c.max_upload_bytes, 314_572_800);
}

#[test]
fn rejected_uploads_and_errors() {
    let s = start("ok", &["--max-upload-mb", "1"]);
    let flac = [("Content-Type", "audio/flac".to_string())];
    assert_eq!(http(s.addr, "POST", "/v1/jobs", &flac, b"").status, 411);
    assert_eq!(post_file(s.addr, "", "not-flac.flac").status, 415);
    assert_eq!(post_file(s.addr, "", "long.flac").status, 413);
    assert_eq!(post_file(s.addr, "?model=nope", "untagged.flac").status, 400);
    let r = http(
        s.addr,
        "POST",
        "/v1/jobs",
        &[("Content-Type", "audio/mpeg".into()), ("Content-Length", "4".into())],
        b"abcd",
    );
    assert_eq!(r.status, 415);
    assert!(r.json()["error"].is_string());
    // Over the size limit: refused from the header alone, the body is never sent.
    let big = [("Content-Type", "audio/flac".to_string()), ("Content-Length", (2 * 1024 * 1024).to_string())];
    let r = http(s.addr, "POST", "/v1/jobs", &big, b"");
    assert_eq!(r.status, 413);
    assert!(r.json()["error"].is_string());
    // Nothing was left behind.
    assert!(!s.jobs().exists() || std::fs::read_dir(s.jobs()).unwrap().next().is_none());
    // Unknown paths and wrong methods.
    assert_eq!(http(s.addr, "GET", "/v1/nothing", &[], b"").status, 404);
    assert_eq!(http(s.addr, "GET", "/v1/jobs/abcd", &[], b"").status, 404);
    assert_eq!(http(s.addr, "PUT", "/v1/jobs", &[], b"").status, 405);
    assert_eq!(http(s.addr, "POST", "/v1/health", &[], b"").status, 405);
    assert_eq!(http(s.addr, "DELETE", "/v1/jobs", &[], b"").status, 405);
    assert_confined(&s);
}

#[test]
fn ok_flow_with_the_client() {
    let s = start("slow", &[]);
    let c = s.client();
    let cancel = AtomicBool::new(false);
    let mut sent_calls = Vec::new();
    let job = c.submit(&fixture("untagged.flac"), &cancel, |sent, total| sent_calls.push((sent, total))).unwrap();
    let size = std::fs::metadata(fixture("untagged.flac")).unwrap().len();
    assert_eq!(sent_calls.last(), Some(&(size, size)));
    assert!(sent_calls.windows(2).all(|w| w[0].0 < w[1].0));

    let mut states = Vec::new();
    let mut progress: Vec<f64> = Vec::new();
    let done = c
        .wait_final(&job, &cancel, Duration::from_millis(100), Duration::from_secs(60), |st| {
            if states.last() != Some(&st.state) {
                states.push(st.state);
            }
            if let Some(p) = st.progress {
                if progress.last() != Some(&p) {
                    progress.push(p);
                }
            }
        })
        .unwrap();
    use calliope_lib::stems_api::JobState;
    assert_eq!(done.state, JobState::Done, "{done:?}");
    assert!(states.contains(&JobState::Running), "{states:?}");
    assert_eq!(*states.last().unwrap(), JobState::Done);
    // queued (if seen) comes before running
    if let Some(q) = states.iter().position(|x| *x == JobState::Queued) {
        assert!(q < states.iter().position(|x| *x == JobState::Running).unwrap());
    }
    assert!(progress.len() >= 2, "progress never moved: {progress:?}");
    assert!(progress.windows(2).all(|w| w[0] < w[1]), "{progress:?}");
    assert_eq!(done.stems.as_deref().unwrap(), ["vocals", "drums", "bass", "guitar", "piano", "other"]);

    let dest_dir = tempfile::tempdir().unwrap();
    let dest = dest_dir.path().to_path_buf();
    for name in done.stems.as_ref().unwrap() {
        let part = dest.join(format!("{name}.flac.part"));
        let n = c.fetch_stem(&job, name, &part, &AtomicBool::new(false)).unwrap();
        let want = std::fs::read(fixture(&format!("stems/{name}.flac"))).unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), want, "{name}");
        assert_eq!(n, want.len() as u64);
    }
    // Unknown stem and invalid names.
    assert!(matches!(
        c.fetch_stem(&job, "nothing", &dest.join("x.part"), &AtomicBool::new(false)),
        Err(ClientError::Restarted) | Err(ClientError::Rejected { status: 404, .. })
    ));
    assert!(!dest.join("x.part").exists());
    assert!(matches!(c.fetch_stem(&job, "../x", &dest.join("y.part"), &AtomicBool::new(false)), Err(ClientError::Invalid(_))));

    let dir = s.jobs().join(&job);
    assert!(dir.exists());
    c.delete(&job);
    assert!(!dir.exists());
    assert!(matches!(c.status(&job), Err(ClientError::Restarted)));
    assert_confined(&s);
}

#[test]
fn raw_ok_flow_serves_stems_in_order() {
    let s = start("ok", &[]);
    let job = submit_raw(&s);
    let done = wait_state(s.addr, &job, "done");
    let names: Vec<&str> = done["stems"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar", "piano", "other"]);
    for n in &names {
        let r = http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/{n}"), &[], b"");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, std::fs::read(fixture(&format!("stems/{n}.flac"))).unwrap());
    }
    let dir = s.jobs().join(&job);
    assert!(dir.exists());
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), &[], b"").status, 204);
    assert!(!dir.exists());
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status, 404);
    assert_confined(&s);
}

#[test]
fn failing_separators_fail_the_job() {
    for mode in ["fail", "bad-output", "not-flac"] {
        let s = start(mode, &[]);
        let job = submit_raw(&s);
        let v = wait_state(s.addr, &job, "failed");
        let msg = v["error"].as_str().unwrap_or("");
        assert!(!msg.is_empty(), "{mode}: {v}");
        if mode != "fail" {
            assert!(msg.contains("invalid output"), "{mode}: {msg}");
        }
        assert!(v["stems"].is_null());
        assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/vocals"), &[], b"").status, 409);
        // The same through the client.
        let st = s.client().status(&job).unwrap();
        assert_eq!(st.error.as_deref(), Some(msg));
    }
}

#[test]
fn queue_limit_gives_503() {
    let s = start("hang", &["--queue", "1"]);
    submit_raw(&s);
    submit_raw(&s);
    let r = post_file(s.addr, "", "untagged.flac");
    assert_eq!(r.status, 503);
    assert!(r.json()["error"].is_string());
    // The client sees it too.
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        s.client().submit(&fixture("untagged.flac"), &cancel, |_, _| {}),
        Err(ClientError::Rejected { status: 503, .. })
    ));
    assert_eq!(std::fs::read_dir(s.jobs()).unwrap().count(), 2);
}

#[test]
fn delete_kills_the_hanging_separator() {
    let s = start("hang", &[]);
    let job = submit_raw(&s);
    wait_state(s.addr, &job, "running");
    assert_eq!(http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/vocals"), &[], b"").status, 409);
    wait_until("stub running", 10, || !pids_mentioning(&job).is_empty());
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), &[], b"").status, 204);
    assert_eq!(raw_status(s.addr, &job)["state"], "cancelled");
    wait_until("stub gone", 10, || pids_mentioning(&job).is_empty());
    assert!(!s.jobs().join(&job).exists());
    assert_eq!(http(s.addr, "GET", "/v1/health", &[], b"").json()["busy"], false);
    // DELETE of a queued job.
    let first = submit_raw(&s);
    wait_state(s.addr, &first, "running");
    let second = submit_raw(&s);
    assert_eq!(raw_status(s.addr, &second)["state"], "queued");
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{second}"), &[], b"").status, 204);
    assert!(!s.jobs().join(&second).exists());
    assert_eq!(raw_status(s.addr, &first)["state"], "running");
}

#[test]
fn restart_forgets_jobs_and_removes_their_folders() {
    let outer = tempfile::tempdir().unwrap();
    let keep = outer.path().join("work/jobs/not-ours");
    std::fs::create_dir_all(&keep).unwrap();
    std::fs::write(keep.join("keep.txt"), b"x").unwrap();
    std::fs::write(outer.path().join("work/other.txt"), b"x").unwrap();
    let mut s = start_in(outer, "ok", &[]);
    let job = submit_raw(&s);
    wait_state(s.addr, &job, "done");
    let _ = s.child.kill();
    let _ = s.child.wait();
    let outer = std::mem::replace(&mut s.outer, tempfile::tempdir().unwrap());
    let path = outer.path().to_path_buf();
    let s2 = start_in(outer, "ok", &[]);
    assert_eq!(http(s2.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status, 404);
    assert!(matches!(s2.client().status(&job), Err(ClientError::Restarted)));
    assert!(!path.join("work/jobs").join(&job).exists());
    assert!(keep.join("keep.txt").exists() && path.join("work/other.txt").exists());
}

#[test]
fn retention_zero_removes_finished_jobs_on_the_next_janitor_run() {
    let s = start("ok", &["--retention-hours", "0", "--janitor-interval-ms", "100"]);
    let job = submit_raw(&s);
    wait_until("job removed", 20, || {
        http(s.addr, "GET", &format!("/v1/jobs/{job}"), &[], b"").status == 404
    });
    assert!(!s.jobs().join(&job).exists());
}

#[test]
fn closed_port_is_unreachable_quickly() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let started = Instant::now();
    let e = StemsClient::new(&format!("http://127.0.0.1:{port}/")).health().unwrap_err();
    assert!(matches!(e, ClientError::Unreachable(_)), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(6));
}

/// A one-shot fake server on 127.0.0.1 answering every request with `body`.
fn fake_server(body: &'static str) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut conn = conn;
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf);
            let _ = write!(
                conn,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    addr
}

#[test]
fn wrong_service_or_api_is_incompatible() {
    let other = fake_server(
        r#"{"service":"other","api":1,"version":"1","models":["m"],"default_model":"m","busy":false,"max_duration_s":900,"max_upload_bytes":1}"#,
    );
    let e = StemsClient::new(&format!("http://{other}")).health().unwrap_err();
    assert!(matches!(e, ClientError::Incompatible(_)), "{e:?}");
    let v2 = fake_server(
        r#"{"service":"calliope-stems","api":2,"version":"1","models":["m"],"default_model":"m","busy":false,"max_duration_s":900,"max_upload_bytes":1}"#,
    );
    assert!(matches!(StemsClient::new(&format!("http://{v2}")).health(), Err(ClientError::Incompatible(_))));
    let junk = fake_server("<html>hello</html>");
    assert!(matches!(StemsClient::new(&format!("http://{junk}")).health(), Err(ClientError::Incompatible(_))));
}

#[test]
fn cancel_during_the_upload_stops_it_and_leaves_no_job() {
    let s = start("hang", &[]);
    let dir = tempfile::tempdir().unwrap();
    // A valid FLAC header followed by padding: the server stores it until the end.
    let big = dir.path().join("big.flac");
    let mut data = std::fs::read(fixture("untagged.flac")).unwrap();
    data.resize(40 * 1024 * 1024, 0);
    std::fs::write(&big, data).unwrap();
    let cancel = AtomicBool::new(false);
    let mut last = 0;
    let started = Instant::now();
    let e = s
        .client()
        .submit(&big, &cancel, |sent, _| {
            last = sent;
            if sent >= 256 * 1024 {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
    assert_eq!(e, ClientError::Cancelled);
    assert!(last < 40 * 1024 * 1024 && started.elapsed() < Duration::from_secs(20));
    // The server drops the half-received upload.
    wait_until("no job folders", 10, || !s.jobs().exists() || std::fs::read_dir(s.jobs()).unwrap().next().is_none());
    assert!(!s.client().health().unwrap().busy);
}

#[test]
fn cancel_after_the_server_accepted_the_job_sends_delete() {
    let s = start("hang", &[]);
    let cancel = AtomicBool::new(false);
    // Cancel at the last byte: the upload completes, the server answers 202, the client deletes.
    let e = s
        .client()
        .submit(&fixture("untagged.flac"), &cancel, |sent, total| {
            if sent == total {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
    assert_eq!(e, ClientError::Cancelled);
    wait_until("job deleted", 10, || {
        !s.client().health().unwrap().busy && std::fs::read_dir(s.jobs()).map(|mut d| d.next().is_none()).unwrap_or(true)
    });
}

#[test]
fn wait_final_cancel_deletes_the_job() {
    let s = start("hang", &[]);
    let c = s.client();
    let cancel = AtomicBool::new(false);
    let job = c.submit(&fixture("untagged.flac"), &cancel, |_, _| {}).unwrap();
    let e = c
        .wait_final(&job, &cancel, Duration::from_millis(50), Duration::from_secs(60), |st| {
            if st.state == calliope_lib::stems_api::JobState::Running {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
    assert_eq!(e, ClientError::Cancelled);
    wait_until("stub gone", 10, || pids_mentioning(&job).is_empty());
    assert!(!s.jobs().join(&job).exists());
}

#[test]
fn stalled_job_is_reported() {
    let s = start("hang", &[]);
    let c = s.client();
    let cancel = AtomicBool::new(false);
    let job = c.submit(&fixture("untagged.flac"), &cancel, |_, _| {}).unwrap();
    let e = c
        .wait_final(&job, &cancel, Duration::from_millis(50), Duration::from_millis(500), |_| {})
        .unwrap_err();
    assert_eq!(e, ClientError::Stalled);
    assert_eq!(e.to_string(), "The edge-AI server stopped responding");
}
