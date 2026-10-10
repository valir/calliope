//! QA acceptance tests (independent) for the increment "empty stems are judged by audible time",
//! server side: `calliope-stems --measure`, the server's `stem_levels` report and `--no-stem-levels`.
//! The oracle is an independent re-implementation in this file working on ffmpeg-decoded PCM (not
//! on calliope_lib::flac_level). Loopback server + stub separator only; no device, no real model.

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

// ------------------------------------------------------------------ independent oracle + builders

const SIX: [&str; 6] = ["vocals", "drums", "bass", "guitar", "piano", "other"];

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Writes interleaved 16-bit PCM as WAV, converts to FLAC with ffmpeg; returns the FLAC path.
fn make_flac(dir: &Path, name: &str, rate: u32, channels: u16, samples: &[i16]) -> PathBuf {
    let wav = dir.join(format!("{name}.wav"));
    let mut b = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&wav, b).unwrap();
    let flac = dir.join(format!("{name}.flac"));
    let st = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&wav)
        .args(["-c:a", "flac", "-sample_fmt", "s16"])
        .arg(&flac)
        .status()
        .unwrap();
    assert!(st.success());
    flac
}

/// Independent oracle: decode with ffmpeg to s16le (native rate/channels), then
/// window = rate/10 frames, level of a window = loudest channel's mean square, audible when
/// mean square > (32768^2) * 10^-4, evaluated in exact integers: S * 10000 > n * 2^30.
fn oracle(flac: &Path, rate: u32, channels: usize) -> (u64, u64) {
    let out = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(flac).args(["-f", "s16le", "-acodec", "pcm_s16le", "-"]).output().unwrap();
    assert!(out.status.success());
    let pcm: Vec<i16> = out.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let frames = pcm.len() / channels;
    let n = (rate / 10) as usize;
    let windows = frames / n;
    let mut audible = 0u64;
    for w in 0..windows {
        let mut best = 0u128;
        for c in 0..channels {
            let mut s = 0u128;
            for f in w * n..(w + 1) * n {
                let v = pcm[f * channels + c] as i128;
                s += (v * v) as u128;
            }
            best = best.max(s);
        }
        if best * 10_000 > (n as u128) * (1u128 << 30) {
            audible += 1;
        }
    }
    (audible * 100, windows as u64)
}

fn tone(rate: u32, secs: f64, amp: f64, gate: impl Fn(usize) -> bool) -> Vec<i16> {
    let n = (rate as f64 * secs) as usize;
    (0..n).map(|i| if gate(i / (rate as usize / 10)) { (amp * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / rate as f64).sin()) as i16 } else { 0 }).collect()
}

/// The server answers stem downloads chunked (or not); decode when the body starts with a hex size line.
fn dechunk(b: &[u8]) -> Vec<u8> {
    if b.starts_with(b"fLaC") {
        return b.to_vec();
    }
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let eol = b[i..].windows(2).position(|w| w == b"\r\n").unwrap() + i;
        let n = usize::from_str_radix(std::str::from_utf8(&b[i..eol]).unwrap().trim(), 16).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[eol + 2..eol + 2 + n]);
        i = eol + 2 + n + 2;
    }
}

fn calliope() -> Command {
    Command::new(env!("CARGO_BIN_EXE_calliope-stems"))
}

/// Runs --measure; returns (stdout lines, exit code).
fn measure(files: &[&Path]) -> (Vec<String>, i32) {
    let o = calliope().arg("--measure").args(files).output().unwrap();
    (String::from_utf8(o.stdout).unwrap().lines().map(String::from).collect(), o.status.code().unwrap())
}
fn kv(line: &str, key: &str) -> String {
    line.split_whitespace().find_map(|w| w.strip_prefix(&format!("{key}="))).unwrap_or_else(|| panic!("no {key} in {line}")).to_string()
}
fn fx(rel: &str) -> PathBuf {
    gui().join("tests/fixtures/import").join(rel)
}

// ------------------------------------------------------------------ --measure

#[test]
fn measure_prints_the_documented_line_for_the_activity_fixtures_and_exits_0() {
    let files = ["bursts", "phrases", "audible-14900ms", "audible-15000ms"].map(|n| fx(&format!("stems-activity/{n}.flac")));
    let refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
    let (lines, code) = measure(&refs);
    assert_eq!(code, 0);
    assert_eq!(lines.len(), 4);
    for (l, (f, ms)) in lines.iter().zip(files.iter().zip([9500, 16000, 14900, 15000])) {
        assert!(l.starts_with(&f.display().to_string()), "{l}");
        assert_eq!(kv(l, "audible_ms"), ms.to_string(), "{l}");
        assert_eq!(kv(l, "level_dbfs"), "-40");
        kv(l, "windows");
        kv(l, "peak_dbfs");
    }
    // bursts: a loud sample peak (about -8 dBFS) but little audible time
    let peak: f64 = kv(&lines[0], "peak_dbfs").parse().unwrap();
    assert!(peak > -9.0 && peak < -7.0, "{peak}");
    // exact line format
    let w = kv(&lines[3], "windows");
    assert_eq!(w, "200");
}

#[test]
fn measure_silent_file_shows_minus_inf_peak_and_zero_ms() {
    let (lines, code) = measure(&[&fx("stems-quiet/silent.flac")]);
    assert_eq!(code, 0);
    assert_eq!(kv(&lines[0], "audible_ms"), "0");
    assert_eq!(kv(&lines[0], "peak_dbfs"), "-inf");
}

#[test]
fn measure_quiet_stems_minus45_and_minus60_are_not_audible() {
    for f in ["minus45", "minus60"] {
        let (lines, code) = measure(&[&fx(&format!("stems-quiet/{f}.flac"))]);
        assert_eq!(code, 0);
        assert_eq!(kv(&lines[0], "audible_ms"), "0", "{f}");
    }
}

#[test]
fn measure_reports_errors_per_file_and_exits_1_but_keeps_going() {
    let d = scratch();
    let missing = d.path().join("missing.flac");
    let (lines, code) = measure(&[&fx("not-flac.flac"), &missing, &fx("stems-activity/audible-15000ms.flac")]);
    assert_eq!(code, 1);
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains(" error="), "{}", lines[0]);
    assert!(lines[1].contains(" error="), "{}", lines[1]);
    assert_eq!(kv(&lines[2], "audible_ms"), "15000");
}

#[test]
fn measure_needs_a_file_and_no_separator_and_never_listens() {
    let o = calliope().arg("--measure").output().unwrap();
    assert_ne!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--measure needs at least one file"), "{}", String::from_utf8_lossy(&o.stderr));
    // with a file but without --separator or any env: works and returns promptly
    let t = Instant::now();
    let (_, code) = measure(&[&fx("stems/vocals.flac")]);
    assert_eq!(code, 0);
    assert!(t.elapsed() < Duration::from_secs(10));
}

#[test]
fn help_mentions_measure_and_no_stem_levels_not_the_removed_flag() {
    let o = calliope().arg("--help").output().unwrap();
    let t = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
    assert!(t.contains("--measure"));
    assert!(t.contains("--no-stem-levels"));
    assert!(!t.contains("--no-stem-peaks"));
    let o = calliope().args(["--separator", "/bin/true", "--no-stem-peaks"]).output().unwrap();
    assert_ne!(o.status.code(), Some(0), "the removed flag must be rejected");
}

// ------------------------------------------------------------------ the measure against an independent oracle

#[test]
fn measure_agrees_with_an_independent_oracle_on_boundaries_and_odd_layouts() {
    let d = scratch();
    let mut cases: Vec<(String, u32, u16, Vec<i16>)> = Vec::new();
    // exact level boundary, mono 8 kHz: |s| 328 audible (-39.99), 327 not (-40.02)
    for (amp, name) in [(328i16, "c328"), (327, "c327"), (-328, "cm328"), (-327, "cm327")] {
        cases.push((name.into(), 8000, 1, vec![amp; 8000 * 15]));
    }
    // 149 vs 150 windows
    for (w, name) in [(149usize, "w149"), (150, "w150"), (151, "w151")] {
        let mut s = vec![0i16; 8000 * 20];
        for x in s.iter_mut().take(w * 800) {
            *x = 3000;
        }
        cases.push((name.into(), 8000, 1, s));
    }
    // trailing partial window ignored: 150 loud windows + 799 loud frames
    let mut s = vec![0i16; 150 * 800 + 799];
    for x in s.iter_mut() {
        *x = 5000;
    }
    cases.push(("partial150".into(), 8000, 1, s));
    cases.push(("partial_only".into(), 8000, 1, vec![9000i16; 799]));
    // stereo 44.1 kHz: loudest channel, channels not summed (each 327-ish on 44.1k scale: the limit is the same ratio)
    let mut st = Vec::new();
    for i in 0..44100 * 16 {
        let _ = i;
        st.push(300i16);
        st.push(330i16);
    }
    cases.push(("stereo_l_quiet_r_loud".into(), 44100, 2, st));
    let mut st = Vec::new();
    for _ in 0..44100 * 16 {
        st.push(327i16);
        st.push(327i16);
    }
    cases.push(("stereo_both_327".into(), 44100, 2, st));
    // a transient-free but busy signal with a gate over odd windows (48 kHz)
    cases.push(("gated48".into(), 48000, 1, tone(48000, 40.0, 4000.0, |w| w % 7 < 3)));
    cases.push(("noise_floor".into(), 44100, 1, tone(44100, 30.0, 250.0, |_| true)));
    for (name, rate, ch, samples) in &cases {
        let f = make_flac(d.path(), name, *rate, *ch, samples);
        let (want_ms, want_windows) = oracle(&f, *rate, *ch as usize);
        let (lines, code) = measure(&[&f]);
        assert_eq!(code, 0);
        assert_eq!(kv(&lines[0], "audible_ms"), want_ms.to_string(), "{name}: {}", lines[0]);
        assert_eq!(kv(&lines[0], "windows"), want_windows.to_string(), "{name}");
    }
    // spot-check the oracle itself with known answers, so a wrong oracle cannot hide a wrong product
    let ms = |n: &str| oracle(&d.path().join(format!("{n}.flac")), 8000, 1).0;
    assert_eq!((ms("c328"), ms("c327"), ms("w149"), ms("w150")), (15_000, 0, 14_900, 15_000));
    assert_eq!(ms("partial150"), 15_000);
    assert_eq!(oracle(&d.path().join("partial_only.flac"), 8000, 1), (0, 0));
}

// ------------------------------------------------------------------ the server's report

fn levels_of(v: &serde_json::Value) -> Vec<(String, u64, i64, u64, serde_json::Value)> {
    v["stem_levels"]
        .as_array()
        .expect("stem_levels array")
        .iter()
        .map(|e| (e["name"].as_str().unwrap().to_string(), e["audible_ms"].as_u64().unwrap(), e["level_dbfs"].as_i64().unwrap(), e["window_ms"].as_u64().unwrap(), e["peak_dbfs"].clone()))
        .collect()
}

#[test]
fn boundary_mode_reports_the_exact_audible_times_in_stem_order() {
    let s = start("boundary", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let l = levels_of(&v);
    assert_eq!(l.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(), SIX);
    let got: Vec<u64> = l.iter().map(|e| e.1).collect();
    assert_eq!(got, [16000, 16000, 16000, 15000, 9500, 14900]);
    assert!(l.iter().all(|e| e.2 == -40 && e.3 == 100));
    // peaks are informational: bursts (piano) is loud even though it is "empty"
    assert!(l[4].4.as_f64().unwrap() > -9.0);
    assert!(s.log().contains("measured=6/6"), "{}", s.log());
}

#[test]
fn server_report_equals_the_standalone_measure_of_the_same_stems_bit_for_bit() {
    let s = start("boundary", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let l = levels_of(&v);
    let mut served = scratch();
    for (name, ms, ..) in &l {
        let (st, body) = http(s.addr, "GET", &format!("/v1/jobs/{job}/stems/{name}"), b"");
        assert_eq!(st, 200);
        let p = served.path().join(format!("{name}.flac"));
        std::fs::write(&p, dechunk(&body)).unwrap();
        let (lines, code) = measure(&[&p]);
        assert_eq!(code, 0, "{lines:?} body len {}", std::fs::metadata(&p).unwrap().len());
        assert_eq!(kv(&lines[0], "audible_ms"), ms.to_string(), "{name}");
    }
    let _ = &mut served;
}

#[test]
fn silent_mode_reports_zero_ms_and_null_peak_for_all_six() {
    let s = start("silent", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let l = levels_of(&v);
    assert_eq!(l.len(), 6);
    assert!(l.iter().all(|e| e.1 == 0 && e.4.is_null()));
}

#[test]
fn no_stem_levels_flag_omits_the_key_in_every_state() {
    let s = start("boundary", &["--no-stem-levels"]);
    let job = submit(&s);
    let (text, _) = wait_done(&s, &job);
    assert!(!text.contains("stem_levels") && !text.contains("stem_peaks"), "{text}");
    assert!(!s.log().contains("audible_ms="), "nothing should be measured: {}", s.log());
}

#[test]
fn no_old_stem_peaks_key_is_ever_sent() {
    let s = start("boundary", &[]);
    let job = submit(&s);
    let (text, _) = wait_done(&s, &job);
    assert!(!text.contains("stem_peaks"), "{text}");
    assert!(text.contains("\"stem_levels\""));
}

#[test]
fn undecodable_stem_has_no_entry_and_the_job_still_succeeds() {
    let s = start("undecodable", &[]);
    let job = submit(&s);
    let (_, v) = wait_done(&s, &job);
    let l = levels_of(&v);
    assert!(l.iter().all(|e| e.0 != "piano"));
    assert!(s.log().contains("stem=piano audible=unknown"), "{}", s.log());
}

// ------------------------------------------------------------------ flake evidence (queue slots)

/// FLAKE ROOT CAUSE for `aborted_uploads_release_their_queue_slot_and_folder`: it waits for `jobs/` to be
/// empty, which is vacuously true before the server has even read the aborted uploads, so its single
/// submit races the handlers still holding the one slot and gets 503 "queue is full". The slots
/// ARE released (no leak): retrying the submit succeeds within milliseconds. This test retries on 503
/// for up to 15 s, which is the real property ("an aborted upload releases its slot").
#[test]
fn aborted_uploads_release_their_slot_a_retried_submit_always_gets_in() {
    let s = start("ok", &["--queue", "1"]);
    let flac = std::fs::read(fx("untagged.flac")).unwrap();
    for _ in 0..8 {
        let mut c = TcpStream::connect(s.addr).unwrap();
        let head = format!("POST /v1/jobs HTTP/1.1\r\nHost: x\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\n\r\n", flac.len());
        c.write_all(head.as_bytes()).unwrap();
        c.write_all(&flac[..flac.len() / 2]).unwrap();
        drop(c);
    }
    let end = Instant::now() + Duration::from_secs(15);
    let mut refused = 0;
    let id = loop {
        let (st, b) = http(s.addr, "POST", "/v1/jobs", &flac);
        if st == 202 {
            break serde_json::from_slice::<serde_json::Value>(&b).unwrap()["job"].as_str().unwrap().to_string();
        }
        assert_eq!(st, 503, "{}", String::from_utf8_lossy(&b));
        refused += 1;
        assert!(Instant::now() < end, "the queue slot was never released ({refused} refusals): a real leak");
        std::thread::sleep(Duration::from_millis(20));
    };
    eprintln!("503s before the slot freed up: {refused}");
    wait_done(&s, &id);
}
