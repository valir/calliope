//! QA acceptance tests for requirement 9 (server side) of specs/gui-stem-extraction.md:
//! "the server reports each stem's peak when a job is done". The REAL `calliope-stems` binary on
//! 127.0.0.1 with the stub separator or a QA separator script that copies pre-built long stems
//! (made with ffmpeg here). Never the real model, never a device.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn gui() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../calliope-gui")
}

struct Server {
    child: Child,
    addr: SocketAddr,
    _outer: tempfile::TempDir,
    log: Arc<Mutex<String>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn log(&self) -> String {
        self.log.lock().unwrap().clone()
    }
}

fn start_with(separator: &Path, mode: &str, extra: &[&str]) -> Server {
    let outer = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
        .arg("--separator")
        .arg(separator)
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
    let log = Arc::new(Mutex::new(String::new()));
    let l2 = log.clone();
    std::thread::spawn(move || {
        let mut s = String::new();
        while lines.read_line(&mut s).unwrap_or(0) > 0 {
            l2.lock().unwrap().push_str(&s);
            s.clear();
        }
    });
    Server { child, addr, _outer: outer, log }
}

fn start(mode: &str, extra: &[&str]) -> Server {
    start_with(&gui().join("tests/support/stub-separator"), mode, extra)
}

fn http(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let ct = if method == "POST" { format!("Content-Type: audio/flac\r\nContent-Length: {}\r\n", body.len()) } else { String::new() };
    s.write_all(format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{ct}\r\n").as_bytes()).unwrap();
    let _ = s.write_all(body);
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("response head");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    (head.split_whitespace().nth(1).unwrap().parse().unwrap(), raw[split + 4..].to_vec())
}
fn status_raw(addr: SocketAddr, job: &str) -> (String, serde_json::Value) {
    let (_, b) = http(addr, "GET", &format!("/v1/jobs/{job}"), b"");
    let text = String::from_utf8(b).unwrap();
    let v = serde_json::from_str(&text).unwrap();
    (text, v)
}
fn submit(s: &Server) -> String {
    let body = std::fs::read(gui().join("tests/fixtures/import/untagged.flac")).unwrap();
    let (st, b) = http(s.addr, "POST", "/v1/jobs", &body);
    assert_eq!(st, 202);
    serde_json::from_slice::<serde_json::Value>(&b).unwrap()["job"].as_str().unwrap().to_string()
}
fn wait_done(s: &Server, job: &str) -> (String, serde_json::Value) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (t, v) = status_raw(s.addr, job);
        if v["state"] == "done" {
            return (t, v);
        }
        assert!(Instant::now() < end, "timeout {v}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

const SIX: [&str; 6] = ["vocals", "drums", "bass", "guitar", "piano", "other"];

// ------------------------------------------------------------------ the JSON contract

#[test]
fn key_is_absent_while_queued_and_running_never_null() {
    let s = start("slow", &[]);
    let job = submit(&s);
    let mut seen_running = false;
    loop {
        let (text, v) = status_raw(s.addr, &job);
        if v["state"] == "done" {
            assert!(v["stem_peaks"].is_array(), "{text}");
            break;
        }
        seen_running |= v["state"] == "running";
        assert!(!text.contains("stem_peaks"), "key present before done: {text}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(seen_running, "never observed a running state");
}

#[test]
fn key_is_absent_for_failed_and_cancelled_jobs() {
    let s = start("fail", &[]);
    let job = submit(&s);
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let (text, v) = status_raw(s.addr, &job);
        if v["state"] == "failed" {
            assert!(!text.contains("stem_peaks"), "{text}");
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = start("hang", &[]);
    let job = submit(&s);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), b"").0, 204);
    let (st, b) = http(s.addr, "GET", &format!("/v1/jobs/{job}"), b"");
    if st == 200 {
        assert!(!String::from_utf8_lossy(&b).contains("stem_peaks"));
    }
}

#[test]
fn no_stem_peaks_flag_omits_the_key_in_every_state() {
    let s = start("slow", &["--no-stem-peaks"]);
    let job = submit(&s);
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (text, v) = status_raw(s.addr, &job);
        assert!(!text.contains("stem_peaks"), "{text}");
        if v["state"] == "done" {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!s.log().contains("measured="), "no measuring must happen");
}

#[test]
fn entries_have_exact_shape_and_digital_silence_is_peak_zero_null_dbfs() {
    let s = start("sparse", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let arr = v["stem_peaks"].as_array().unwrap();
    assert_eq!(arr.len(), 6);
    for (e, name) in arr.iter().zip(SIX) {
        let o = e.as_object().unwrap();
        let mut keys: Vec<&str> = o.keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["bits", "name", "peak", "peak_dbfs"], "{e}");
        assert_eq!(e["name"], name);
        assert!(e["peak"].is_u64() && e["bits"].is_u64());
        // peak_dbfs agrees with peak within rounding (human field)
        if let Some(db) = e["peak_dbfs"].as_f64() {
            let want = 20.0 * (e["peak"].as_f64().unwrap() / 2f64.powi(e["bits"].as_i64().unwrap() as i32 - 1)).log10();
            assert!((db - want).abs() < 0.01, "{e}");
        } else {
            assert_eq!(e["peak"], 0, "null dbfs only for digital silence: {e}");
        }
    }
    assert_eq!(arr[4]["peak"], 0);
    assert!(arr[4]["peak_dbfs"].is_null());
    assert!(arr[5]["peak_dbfs"].as_f64().unwrap() < -50.0);
    assert!(arr[3]["peak_dbfs"].as_f64().unwrap() > -50.0);
}

#[test]
fn all_silent_job_reports_six_zero_peaks_and_is_still_done() {
    let s = start("silent", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let arr = v["stem_peaks"].as_array().unwrap();
    assert_eq!(arr.len(), 6);
    assert!(arr.iter().all(|e| e["peak"] == 0 && e["peak_dbfs"].is_null()));
}

#[test]
fn unmeasurable_stem_has_no_entry_and_empty_list_is_an_array_not_missing() {
    let s = start("undecodable", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let names: Vec<&str> = v["stem_peaks"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"piano"));
    assert_eq!(names.len(), 5);
    assert!(s.log().contains("peak=unknown"));
}

#[test]
fn json_is_still_parseable_by_an_old_shaped_client() {
    #[derive(serde::Deserialize, Debug)]
    #[allow(dead_code)]
    struct Old {
        job: String,
        state: String,
        progress: Option<f64>,
        stems: Option<Vec<String>>,
        error: Option<String>,
    }
    let s = start("sparse", &[]);
    let job = submit(&s);
    let (text, _) = wait_done(&s, &job);
    let old: Old = serde_json::from_str(&text).unwrap();
    assert_eq!(old.stems.unwrap().len(), 6);
    assert_eq!(old.progress, Some(1.0));
}

// ------------------------------------------------------------------ long measuring: bits, cancel, shutdown

/// A QA separator that copies six pre-built long stems; returns (script, dir keeping them alive).
fn long_separator(secs: u32, bits32: bool) -> (PathBuf, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let stem = d.path().join("long.flac");
    let fmt = if bits32 { "s32" } else { "s16" };
    let st = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo", "-t", &secs.to_string(), "-sample_fmt", fmt, "-c:a", "flac"])
        .arg(&stem)
        .status()
        .unwrap();
    assert!(st.success());
    let script = d.path().join("sep.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/bash\nmkdir -p \"$2\"\nfor n in {}; do cp '{}' \"$2/$n.flac\"; done\necho 'progress 1'\n",
            SIX.join(" "),
            stem.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, d)
}

#[test]
fn bits_reflect_the_stem_depth_24_bit() {
    let (sep, _d) = long_separator(2, true);
    let s = start_with(&sep, "ok", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    for e in v["stem_peaks"].as_array().unwrap() {
        assert_eq!(e["bits"], 24, "{e}");
        assert_eq!(e["peak"], 0);
    }
}

#[test]
fn state_stays_running_while_measuring_and_cancel_during_measuring_gives_cancelled() {
    let (sep, _d) = long_separator(1200, false);
    let s = start_with(&sep, "ok", &[]);
    let job = submit(&s);
    // wait for the separator to finish (log "measure" lines start) while job state is not done
    let end = Instant::now() + Duration::from_secs(60);
    let t_measuring;
    loop {
        let (_, v) = status_raw(s.addr, &job);
        if v["state"] == "done" {
            eprintln!("QA: measuring too fast to cancel on this machine; skipping cancel part");
            return;
        }
        if s.log().contains("stem=vocals peak=") {
            assert_eq!(v["state"], "running", "{v}");
            t_measuring = Instant::now();
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    let (st, _) = http(s.addr, "DELETE", &format!("/v1/jobs/{job}"), b"");
    assert_eq!(st, 204);
    eprintln!("QA: DELETE during measuring answered {:?} after measuring started", t_measuring.elapsed());
    assert!(t_measuring.elapsed() < Duration::from_secs(10));
    // the job never turns done afterwards; folder is removed
    std::thread::sleep(Duration::from_millis(1500));
    let (st, b) = http(s.addr, "GET", &format!("/v1/jobs/{job}"), b"");
    if st == 200 {
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_ne!(v["state"], "done", "{v}");
        assert!(v.get("stem_peaks").is_none());
    } else {
        assert_eq!(st, 404);
    }
    // server healthy and the worker not wedged: a new job completes
    let job2 = submit(&s);
    let (_, v2) = wait_done(&s, &job2);
    assert!(v2["stem_peaks"].is_array());
}

#[test]
fn sigterm_during_measuring_exits_promptly() {
    let (sep, _d) = long_separator(1800, false);
    let mut s = start_with(&sep, "ok", &[]);
    let job = submit(&s);
    let end = Instant::now() + Duration::from_secs(60);
    while !s.log().contains("stem=vocals peak=") {
        let (_, v) = status_raw(s.addr, &job);
        if v["state"] == "done" {
            eprintln!("QA: too fast; skip");
            return;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    let t = Instant::now();
    let _ = Command::new("kill").args(["-TERM", &s.child.id().to_string()]).status();
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if s.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < end, "server did not exit within 20 s of SIGTERM during measuring");
        std::thread::sleep(Duration::from_millis(20));
    }
    eprintln!("QA: SIGTERM during measuring -> exit after {:?}", t.elapsed());
}

#[test]
fn measuring_time_is_logged_and_bounded_for_a_long_song() {
    // 6 stems x 10 minutes stereo 44.1k: the plan expects "a few seconds" for 4-minute songs.
    let (sep, _d) = long_separator(600, false);
    let s = start_with(&sep, "ok", &[]);
    let job = submit(&s);
    let t = Instant::now();
    let (_, v) = wait_done(&s, &job);
    assert_eq!(v["stem_peaks"].as_array().unwrap().len(), 6);
    let log = s.log();
    let ms: u64 = log.split("measured=6/6 ms=").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    eprintln!("QA: measuring 6 x 10 min silence took {ms} ms (total {:?})", t.elapsed());
    assert!(ms < 60_000);
}
