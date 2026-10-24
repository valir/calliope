//! QA acceptance tests for stems-beat-autodetect (Rust side): tempo map rules (A9, A12, A15 and
//! the stretched-beat edge cases), and the detection job against the real `calliope-stems`
//! (stub separator, stub beat tracker) or `tests/support/hostile-beats-server.py` (hostile
//! beats answers), all on 127.0.0.1. Audio only ever goes to `ManualBackend`.

#![allow(clippy::neg_cmp_op_on_partial_ord)]

use calliope_gui::audio_out::ManualBackend;
use calliope_gui::beat_detect::{DetectParams, NO_BEATS_TASK};
use calliope_gui::editor::{EditorEvent, EditorManager, EventSink, OpenSpec};
use calliope_gui::tempo_map::{SectionMeta, Segment, TempoMap};
use calliope_gui::tempo_store::{FileTempoStore, NEWER};
use calliope_gui::track_meta::StemEntry;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SIX: &str = "0199c0a0-0000-7000-8000-000000000204";

// ================================================================ tempo map (pure)

fn det(start: f64, bpm: f64) -> Segment {
    Segment {
        start_ms: start,
        bpm,
        detected_ms: Some(start),
        on_beat: false,
        meta: SectionMeta::default(),
    }
}
fn typed(start: f64, bpm: f64) -> Segment {
    Segment {
        start_ms: start,
        bpm,
        detected_ms: None,
        on_beat: false,
        meta: SectionMeta::default(),
    }
}
fn map(segs: Vec<Segment>) -> TempoMap {
    TempoMap {
        beats_per_bar: 4,
        segments: segs,
    }
}
fn fine(bpm: f64) -> f64 {
    60000.0 / (bpm * 64.0)
}

#[test]
fn a12_total_offset_is_limited_to_256_steps_over_any_number_of_moves() {
    let m0 = map(vec![typed(500.0, 120.0), det(6500.0, 90.0)]);
    // many successive absolute moves, as a drag series does; each is relative to detected_ms
    let mut m = m0.clone();
    for steps in [10, 100, 256, 0, -100, -256, 255, -255, 1, -1] {
        let (n, applied) = m.set_change_offset(1, steps).unwrap();
        assert_eq!(applied, steps);
        let off = (n.segments[1].start_ms - 6500.0) / fine(120.0);
        assert!((off - steps as f64).abs() < 1e-6, "{steps}: {off}");
        assert_eq!(
            n.segments[1].detected_ms,
            Some(6500.0),
            "detected position is kept"
        );
        m = n;
    }
    // exactly four beats of the left tempo at the limit
    let (n, _) = m.set_change_offset(1, 256).unwrap();
    assert!((n.segments[1].start_ms - (6500.0 + 4.0 * 500.0)).abs() < 1e-6);
    let (n, _) = m.set_change_offset(1, -256).unwrap();
    assert!((n.segments[1].start_ms - (6500.0 - 4.0 * 500.0)).abs() < 1e-6);
    // beyond the limit is refused (map unchanged: the method is pure) and 0 steps restores
    assert!(m.set_change_offset(1, 257).is_err());
    assert!(m.set_change_offset(1, -257).is_err());
    assert!(m.set_change_offset(1, i64::MAX).is_err());
    assert_eq!(m.set_change_offset(1, 0).unwrap().0, m0);
}

/// Defect D1: `steps.abs()` overflows (panic in debug, a wrong accept in release) for i64::MIN.
#[test]
fn a12_extreme_step_counts_are_refused_not_a_panic() {
    let m = map(vec![typed(500.0, 120.0), det(6500.0, 90.0)]);
    assert!(m.set_change_offset(1, i64::MIN).is_err());
}

#[test]
fn a12_neighbours_keep_one_beat_and_later_changes_keep_their_time() {
    let m = map(vec![
        typed(500.0, 120.0),
        det(6500.0, 90.0),
        det(9000.0, 100.0),
        det(20000.0, 60.0),
    ]);
    // change 1 pushed right is stopped one beat (of its own 90 BPM) before change 2
    let (n, applied) = m.set_change_offset(1, 256).unwrap();
    assert!(applied < 256);
    assert!(n.segments[2].start_ms - n.segments[1].start_ms >= 60000.0 / 90.0 - 1e-6);
    assert_eq!(n.segments[2], m.segments[2]);
    assert_eq!(n.segments[3], m.segments[3]);
    // change 2 pushed left is stopped one beat (120 -> left tempo 90) after change 1
    let (n, applied) = m.set_change_offset(2, -256).unwrap();
    assert!(applied > -256);
    assert!(n.segments[2].start_ms - n.segments[1].start_ms >= 60000.0 / 90.0 - 1e-6);
    // the first section's start never moves; a typed change is not movable; bad index
    assert!(m.set_change_offset(0, 1).is_err());
    assert!(m.set_change_offset(4, 1).is_err());
    let t = map(vec![typed(500.0, 120.0), typed(6500.0, 90.0)]);
    assert!(t.set_change_offset(1, 1).is_err());
    // a 1-bar section squeezed between neighbours: no panic, either moves or a clean error
    let tight = map(vec![
        typed(500.0, 120.0),
        det(4500.0, 120.0),
        det(6500.0, 120.0),
    ]);
    for i in [1, 2] {
        for s in [-256, -1, 0, 1, 256] {
            let _ = tight.set_change_offset(i, s);
        }
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
    }
}

fn assert_sane(m: &TempoMap, what: &str, strict: bool) {
    if let Err(e) = sane(m, what, strict) {
        panic!("{e}");
    }
}

fn sane(m: &TempoMap, what: &str, strict: bool) -> Result<(), String> {
    macro_rules! check {
        ($c:expr, $($a:tt)*) => { if !$c { return Err(format!($($a)*)); } };
    }
    let bases = m.bases();
    for w in bases.windows(2) {
        check!(
            !strict || w[0] <= w[1],
            "{what}: bases not ordered {bases:?} for {m:?}"
        );
    }
    // ms_at strictly increasing in tick, and tick_near inverts it
    let mut last = f64::NEG_INFINITY;
    let end = bases.last().copied().unwrap() + 80;
    for t in -20..end {
        let ms = m.ms_at(t);
        check!(
            ms > last,
            "{what}: ms_at({t})={ms} not above {last} for {m:?}"
        );
        last = ms;
        check!(
            m.tick_near(ms) == t,
            "{what}: tick_near(ms_at({t})) for {m:?}"
        );
    }
    // beats are strictly increasing and no gap exceeds 3 s (all tempos here are >= 40 BPM
    // with at most one stretched beat)
    let from = m.segments[0].start_ms - 2000.0;
    let to = m.ms_at(end);
    let beats = m.beats_in(from.max(0.0), to);
    for w in beats.windows(2) {
        check!(w[1].0 > w[0].0, "{what}: beats not increasing {:?}", w);
        check!(
            w[1].0 - w[0].0 < 3000.0,
            "{what}: beat gap {:?} for {m:?}",
            w
        );
    }
    Ok(())
}

#[test]
fn a9_a12_detected_changes_start_on_a_beat_with_a_stretched_last_beat() {
    let base = map(vec![
        typed(500.0, 120.0),
        det(6500.0, 90.0),
        det(12500.0, 150.0),
    ]);
    for steps in [
        -256, -100, -17, -1, 0, 1, 3, 31, 32, 33, 63, 64, 65, 200, 256,
    ] {
        let (m, _) = base.set_change_offset(1, steps).unwrap();
        assert_sane(&m, &format!("steps {steps}"), true);
        let bases = m.bases();
        assert_eq!(
            bases[1] % 16,
            0,
            "detected change starts on a beat (steps {steps})"
        );
        assert_eq!(bases[2] % 16, 0);
        // the beat grid has a click exactly at the section start (metronome follows the move)
        for i in [1usize, 2] {
            let at = m.segments[i].start_ms;
            let beats = m.beats_in(at - 1.0, at + 1.0);
            assert_eq!(
                beats.len(),
                1,
                "a click at change {i} (steps {steps}): {beats:?}"
            );
            assert!((beats[0].0 - at).abs() < 1e-6);
        }
    }
}

#[test]
fn typed_bpm_inside_a_stretched_beat_keeps_the_grid_sane() {
    typed_sweep(false);
}

/// Defect D2: a BPM typed in the last (stretched) beat before a detected change leaves the tick
/// bases out of order; the typed section has no effect on the metronome grid.
#[test]
fn typed_bpm_inside_a_stretched_beat_keeps_tick_bases_ordered() {
    typed_sweep(true);
}

fn typed_sweep(strict: bool) {
    let mut rng = Lcg(0x5eed);
    let mut checked = 0;
    let mut failures: Vec<(f64, f64, i64, String)> = Vec::new();
    for steps in [-200i64, -3, 0, 11, 40, 64, 200] {
        let (m0, _) = map(vec![
            typed(500.0, 120.0),
            det(6500.0, 90.0),
            det(12500.0, 100.0),
        ])
        .set_change_offset(1, steps)
        .unwrap();
        let change = m0.segments[1].start_ms;
        // typed positions: through the whole last beat (incl. a hair before the change) and a bit around it
        for k in 0..60 {
            let back = if k < 40 {
                rng.next() * 700.0
            } else {
                (k - 40) as f64 * 0.8
            };
            for bpm in [33.0, 60.0, 90.0, 100.0, 120.5, 200.0, 299.9] {
                for pos in [change - back, change + back] {
                    let mut m = m0.clone();
                    if m.set_bpm_at(pos, bpm).is_err() {
                        continue;
                    }
                    if let Err(e) = sane(&m, "typed", strict) {
                        failures.push((bpm, change - pos, steps, e));
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1000);
    if !failures.is_empty() {
        let mut bpms: Vec<f64> = failures.iter().map(|f| f.0).collect();
        bpms.dedup();
        let near = failures.iter().filter(|f| f.1 > 0.0 && f.1 < 600.0).count();
        panic!("{} of {checked} typed changes broke the grid (bpms {bpms:?}, {near} before the change); first: {:?}", failures.len(), failures[0]);
    }
}

#[test]
fn typing_bpm_zero_or_equal_neighbour_removes_or_merges_detected_changes() {
    // a removed first section drops the detected flag of the new first one (A15)
    let mut m = map(vec![
        typed(500.0, 120.0),
        det(6500.0, 90.0),
        det(12500.0, 100.0),
    ]);
    m.set_bpm_at(100.0, 0.0).unwrap();
    assert_eq!(m.segments[0].detected_ms, None);
    assert_eq!(m.segments.len(), 2);
    // a correction exactly at a change keeps its detected position
    let mut m = map(vec![typed(500.0, 120.0), det(6500.0, 90.0)]);
    m.set_bpm_at(6500.0, 95.0).unwrap();
    assert_eq!(m.segments[1].bpm, 95.0);
    assert_eq!(m.segments[1].detected_ms, Some(6500.0));
    // typing the left tempo at the change merges it away
    let mut m = map(vec![typed(500.0, 120.0), det(6500.0, 90.0)]);
    m.set_bpm_at(6500.0, 120.0).unwrap();
    assert_eq!(m.segments.len(), 1);
    // old maps (no detected_ms anywhere) serialize without the key
    let old = map(vec![typed(500.0, 120.0), typed(6500.0, 90.0)]);
    let v = serde_json::to_value(&old).unwrap();
    assert!(v["segments"][1].get("detected_ms").is_none());
    let back: TempoMap =
        serde_json::from_str(r#"{"beats_per_bar":4,"segments":[{"start_ms":500.0,"bpm":120.0}]}"#)
            .unwrap();
    assert_eq!(back.segments[0].detected_ms, None);
}

// ================================================================ servers and rig

fn stems_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let bin = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("calliope-stems");
    assert!(
        bin.is_file(),
        "{} is missing: cargo build -p calliope-stems",
        bin.display()
    );
    bin
}

fn support(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support")
        .join(name)
}

/// A child process killed on drop.
struct Server {
    child: Child,
    url: String,
    _tmp: tempfile::TempDir,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The real calliope-stems on 127.0.0.1:0; `tracker` None = no beat tracker (an old server).
fn real_server(tracker: Option<&str>) -> Server {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("stub-mode"), "ok\n").unwrap();
    let mut cmd = Command::new(stems_binary());
    if let Some(t) = tracker {
        fs::write(work.join("stub-tracker-mode"), format!("{t}\n")).unwrap();
        cmd.arg("--beat-tracker")
            .arg(support("stub-beat-tracker"))
            .args(["--beat-timeout-ms", "3000"]);
    }
    let mut child = cmd
        .arg("--separator")
        .arg(support("stub-separator"))
        .args(["--listen", "127.0.0.1:0", "--work-dir"])
        .arg(&work)
        .env_remove("STUB_SEPARATOR_MODE")
        .env_remove("STUB_TRACKER_MODE")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    let addr = loop {
        line.clear();
        assert!(
            lines.read_line(&mut line).unwrap() > 0,
            "server exited before listening"
        );
        if let Some(rest) = line.split("listening addr=").nth(1) {
            break rest.split_whitespace().next().unwrap().to_string();
        }
    };
    assert!(addr.starts_with("127.0.0.1:"), "{addr}");
    std::thread::spawn(move || {
        let mut s = String::new();
        while lines.read_line(&mut s).unwrap_or(0) > 0 {
            s.clear();
        }
    });
    Server {
        child,
        url: format!("http://{addr}"),
        _tmp: tmp,
    }
}

/// The hostile beats server answering with the raw `payload` bytes.
fn hostile_server(payload: &[u8]) -> Server {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("payload");
    fs::write(&file, payload).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let child = Command::new("python3")
        .arg("-I")
        .arg(support("hostile-beats-server.py"))
        .arg(port.to_string())
        .arg(&file)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < end, "hostile server did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    Server {
        child,
        url: format!("http://127.0.0.1:{port}"),
        _tmp: tmp,
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

type Events = Arc<Mutex<Vec<EditorEvent>>>;

struct Rig {
    _tmp: tempfile::TempDir,
    spec: OpenSpec,
    m: EditorManager,
    events: Events,
}

impl Rig {
    /// "Six Lanes" cut to two 16 s stems (the stub tracker's answers depend on the length).
    fn new() -> Rig {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/library-editor/tracks")
            .join(SIX);
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(SIX);
        copy_tree(&src, &dir);
        let v: Value = serde_json::from_slice(&fs::read(dir.join("track.json")).unwrap()).unwrap();
        let stems: Vec<StemEntry> = v["stems"]
            .as_array()
            .unwrap()
            .iter()
            .take(2)
            .map(|s| StemEntry {
                name: s["name"].as_str().unwrap().into(),
                file: s["file"].as_str().unwrap().into(),
            })
            .collect();
        for s in &stems {
            let ok = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=330:sample_rate=22050:duration=16",
                ])
                .args(["-ac", "1", "-sample_fmt", "s16", "-c:a", "flac"])
                .arg(dir.join(&s.file))
                .status()
                .expect("ffmpeg is needed")
                .success();
            assert!(ok);
        }
        let spec = OpenSpec {
            id: SIX.into(),
            title: "t".into(),
            dir,
            stems: stems.clone(),
            missing: vec![],
            variant: None,
            tempo: Default::default(),
        };
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
        let mut m = EditorManager::new(Arc::new(ManualBackend::new()), sink);
        m.open(&spec).unwrap();
        for s in &stems {
            m.set_stem(SIX, &s.name, Some(0.0), true, Instant::now())
                .unwrap();
        }
        Rig {
            _tmp: tmp,
            spec,
            m,
            events,
        }
    }

    fn params(url: &str) -> DetectParams {
        DetectParams {
            url: url.into(),
            store: Arc::new(FileTempoStore),
            poll: Duration::from_millis(50),
            stall: Duration::from_secs(5),
        }
    }

    fn finish(&mut self) -> EditorEvent {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            self.m.tick(Instant::now());
            let found = self
                .events
                .lock()
                .unwrap()
                .iter()
                .find(|e| {
                    matches!(
                        e,
                        EditorEvent::Detected { .. }
                            | EditorEvent::DetectFailed { .. }
                            | EditorEvent::DetectCancelled { .. }
                    )
                })
                .cloned();
            if let Some(e) = found {
                return e;
            }
            assert!(Instant::now() < end, "no final event of the detection");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn run(&mut self, url: &str) -> EditorEvent {
        self.events.lock().unwrap().clear();
        self.m.start_detect(SIX, Rig::params(url)).unwrap();
        self.finish()
    }

    fn player_json(&self) -> Option<Value> {
        fs::read(self.spec.dir.join("player.json"))
            .ok()
            .map(|b| serde_json::from_slice(&b).unwrap())
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(&self.spec.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".calliope-beats-") || n.ends_with(".part"))
            .collect()
    }
}

fn failure(e: EditorEvent) -> String {
    match e {
        EditorEvent::DetectFailed { message, .. } => message,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn beats_doc(beats: &[f64]) -> Vec<u8> {
    let down: Vec<f64> = beats.iter().copied().step_by(4).collect();
    serde_json::to_vec(&json!({"beats": beats, "downbeats": down, "tracker": "qa"})).unwrap()
}

fn grid(start: f64, step: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| ((start + i as f64 * step) * 1e6).round() / 1e6)
        .collect()
}

// ================================================================ detection against servers

#[test]
fn a5_a8_a9_a10_detection_stores_schema_3_and_replaces_the_old_map() {
    let server = real_server(Some("ok"));
    let mut r = Rig::new();
    fs::write(
        r.spec.dir.join("player.json"),
        r#"{"schema_version":2,"position_ms":777,"speed":0.7,"future_key":{"a":1},"tempo":{"beats_per_bar":3,"segments":[{"start_ms":100.0,"bpm":100.0}]},"loops":[{"id":"l1","start":{"tick":16},"end":{"tick":48},"repeat":true,"count_in":false}]}"#,
    )
    .unwrap();
    let t0 = Instant::now();
    let EditorEvent::Detected {
        replaced,
        beats,
        tempo,
        ..
    } = r.run(&server.url)
    else {
        panic!("not detected")
    };
    assert!(t0.elapsed() < Duration::from_secs(10));
    assert!(replaced);
    assert_eq!(beats, 27);
    assert_eq!(tempo.segments.len(), 2);
    let v = r.player_json().unwrap();
    assert_eq!(v["schema_version"], 6);
    assert_eq!(v["position_ms"], 777);
    assert_eq!(v["speed"], 0.7, "the speed is kept with a new map");
    assert_eq!(v["future_key"], json!({"a": 1}), "other settings kept");
    let segs = v["tempo"]["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 2);
    assert!(segs[0].get("detected_ms").is_none());
    assert_eq!(segs[1]["detected_ms"], segs[1]["start_ms"]);
    assert_eq!(segs[0]["bpm"], 120.0);
    assert_eq!(segs[1]["bpm"], 90.0);
    // the loop keeps its time (tick 16 of 100 BPM from 100 ms = 100 + 16*37.5 = 700 ms)
    let l = &v["loops"][0];
    let tick = l["start"]["tick"].as_i64().unwrap();
    let ms = 500.0 + tick as f64 * 60000.0 / (120.0 * 16.0);
    assert!(
        (ms - 700.0).abs() <= 60000.0 / 120.0 / 16.0 / 2.0 + 1e-6,
        "loop moved to {ms} ms"
    );
    assert!(r.leftovers().is_empty());
}

#[test]
fn a5_a6_a8_a10_half_time_ending_folds_and_steady_song() {
    let server = real_server(Some("bar-end"));
    let mut r = Rig::new();
    assert!(matches!(r.run(&server.url), EditorEvent::Detected { .. }));
    let segs = r.player_json().unwrap()["tempo"]["segments"].clone();
    // owner change 1: the half-time last bar folds into the main tempo
    assert_eq!(segs.as_array().unwrap().len(), 1, "{segs}");
    assert_eq!(segs[0]["bpm"], 120.0);
    let server = real_server(Some("steady"));
    let mut r = Rig::new();
    assert!(matches!(r.run(&server.url), EditorEvent::Detected { .. }));
    let segs = r.player_json().unwrap()["tempo"]["segments"].clone();
    assert_eq!(segs.as_array().unwrap().len(), 1);
    assert_eq!(segs[0]["start_ms"], 500.0);
}

#[test]
fn a7_server_without_beats_unreachable_and_few_beats_leave_everything_unchanged() {
    let mut r = Rig::new();
    let old = r#"{"schema_version":2,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":100.0,"bpm":100.0}]}}"#;
    fs::write(r.spec.dir.join("player.json"), old).unwrap();
    let plain = real_server(None);
    assert_eq!(failure(r.run(&plain.url)), NO_BEATS_TASK);
    let few = real_server(Some("few"));
    let m = failure(r.run(&few.url));
    assert!(m.to_lowercase().contains("beat"), "{m}");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}");
    assert_eq!(
        failure(r.run(&url)),
        format!("Cannot reach the edge-AI server at {url}")
    );
    assert_eq!(
        fs::read_to_string(r.spec.dir.join("player.json")).unwrap(),
        old
    );
    assert!(r.leftovers().is_empty());
    // the button is available again: another run is accepted
    assert!(r.m.start_detect(SIX, Rig::params(&url)).is_ok());
    r.finish();
}

#[test]
fn hostile_beat_lists_are_failures_not_crashes_and_never_touch_player_json() {
    let mut r = Rig::new();
    let old = r#"{"schema_version":2,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":100.0,"bpm":100.0}]}}"#;
    fs::write(r.spec.dir.join("player.json"), old).unwrap();
    let huge_count = grid(0.0, 0.0008, 20_001);
    let body_too_big = {
        let mut v = beats_doc(&grid(0.5, 0.5, 30));
        v.truncate(v.len() - 1);
        v.extend_from_slice(b",\"pad\":\"");
        v.extend(std::iter::repeat_n(b'x', 2 << 20));
        v.extend_from_slice(b"\"}");
        v
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "more than the allowed number of beats",
            beats_doc(&huge_count),
        ),
        ("body over 1 MiB", body_too_big),
        ("not json", b"<html>captive portal</html>".to_vec()),
        (
            "NaN",
            br#"{"beats":[0.5,NaN,1.5,2,2.5,3,3.5,4,4.5,5],"downbeats":[]}"#.to_vec(),
        ),
        (
            "negative",
            br#"{"beats":[-1,0.5,1,1.5,2,2.5,3,3.5,4,4.5],"downbeats":[]}"#.to_vec(),
        ),
        ("beyond the track", beats_doc(&grid(0.5, 0.5, 60))),
        (
            "unsorted",
            br#"{"beats":[1,0.5,1.5,2,2.5,3,3.5,4,4.5,5],"downbeats":[]}"#.to_vec(),
        ),
        (
            "duplicate",
            br#"{"beats":[0.5,0.5,1.5,2,2.5,3,3.5,4,4.5,5],"downbeats":[]}"#.to_vec(),
        ),
        (
            "downbeat not a beat",
            br#"{"beats":[0.5,1,1.5,2,2.5,3,3.5,4,4.5,5],"downbeats":[0.7]}"#.to_vec(),
        ),
        (
            "beats is a string",
            br#"{"beats":"x","downbeats":[]}"#.to_vec(),
        ),
        ("empty", b"".to_vec()),
        ("empty lists", beats_doc(&[])),
        ("seven beats", beats_doc(&grid(0.5, 0.5, 7))),
    ];
    for (name, payload) in cases {
        let s = hostile_server(&payload);
        let m = failure(r.run(&s.url));
        assert!(!m.is_empty(), "{name}");
        assert_eq!(
            fs::read_to_string(r.spec.dir.join("player.json")).unwrap(),
            old,
            "{name}: player.json changed"
        );
        assert!(r.leftovers().is_empty(), "{name}");
        assert_eq!(
            r.m.snapshot(SIX).unwrap().detect,
            None,
            "{name}: still busy"
        );
    }
}

#[test]
fn erratic_valid_beat_lists_end_quickly_with_a_sane_map_or_an_error() {
    let mut r = Rig::new();
    let mut rng = Lcg(42);
    let mut lists: Vec<(&str, Vec<f64>)> = Vec::new();
    // 1 ms spacing: 1000 BPM-equivalents all over the track (dense, valid for the checker)
    lists.push(("dense 1 ms", grid(0.0, 0.001, 17_000)));
    // random intervals 50..1000 ms
    let mut t: f64 = 0.1;
    let mut random = Vec::new();
    while t < 16.5 {
        random.push((t * 1e6).round() / 1e6);
        t += 0.05 + rng.next() * 0.95;
    }
    lists.push(("random", random));
    // very slow beats (30 BPM) and very fast ones (400 BPM)
    lists.push(("slow", grid(0.5, 2.0, 9)));
    lists.push(("fast 400 BPM", grid(0.1, 0.15, 110)));
    // tempo alternating every bar between 80 and 160 (many 1-bar sections)
    let mut alt = Vec::new();
    let mut t: f64 = 0.5;
    for bar in 0..20 {
        let step = if bar % 2 == 0 { 0.75 } else { 0.375 };
        for _ in 0..4 {
            if t < 16.5 {
                alt.push((t * 1e6).round() / 1e6);
            }
            t += step;
        }
    }
    lists.push(("alternating bars", alt));
    for (name, beats) in lists {
        let s = hostile_server(&beats_doc(&beats));
        let t0 = Instant::now();
        match r.run(&s.url) {
            EditorEvent::Detected { tempo, .. } => {
                assert!(tempo.segments.len() <= 64, "{name}");
                for w in tempo.segments.windows(2) {
                    assert!(
                        w[1].start_ms > w[0].start_ms,
                        "{name}: starts not increasing"
                    );
                }
                for sg in &tempo.segments {
                    assert!((20.0..=300.0).contains(&sg.bpm), "{name}: bpm {}", sg.bpm);
                }
                let m = TempoMap {
                    beats_per_bar: tempo.beats_per_bar,
                    segments: tempo.segments.clone(),
                };
                assert_sane(&m, name, true);
            }
            EditorEvent::DetectFailed { message, .. } => assert!(!message.is_empty(), "{name}"),
            other => panic!("{name}: {other:?}"),
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "{name} took {:?}",
            t0.elapsed()
        );
    }
}

#[test]
fn newer_schema_player_json_refuses_detection_result_and_stays_byte_identical() {
    let server = real_server(Some("ok"));
    let mut r = Rig::new();
    let newer = r#"{"schema_version":7,"tempo":{"beats_per_bar":4,"segments":[]},"x":[1,2]}"#;
    fs::write(r.spec.dir.join("player.json"), newer).unwrap();
    assert_eq!(failure(r.run(&server.url)), NEWER);
    assert_eq!(
        fs::read_to_string(r.spec.dir.join("player.json")).unwrap(),
        newer
    );
    // becoming newer in the middle of a (slow) detection
    let slow = real_server(Some("slow"));
    let mut r = Rig::new();
    r.events.lock().unwrap().clear();
    r.m.start_detect(SIX, Rig::params(&slow.url)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    fs::write(r.spec.dir.join("player.json"), newer).unwrap();
    assert_eq!(failure(r.finish()), NEWER);
    assert_eq!(
        fs::read_to_string(r.spec.dir.join("player.json")).unwrap(),
        newer
    );
    // malformed player.json is not silently replaced by a result either
    let mut r = Rig::new();
    fs::write(r.spec.dir.join("player.json"), "{ not json").unwrap();
    let res = r.run(&server.url);
    let after = fs::read_to_string(r.spec.dir.join("player.json")).unwrap();
    match res {
        EditorEvent::Detected { .. } => assert!(serde_json::from_str::<Value>(&after).is_ok()),
        _ => assert_eq!(after, "{ not json"),
    }
}

#[test]
fn a6_cancel_close_and_exit_leave_no_files_and_no_busy_state() {
    let server = real_server(Some("slow"));
    // cancel
    let mut r = Rig::new();
    r.events.lock().unwrap().clear();
    r.m.start_detect(SIX, Rig::params(&server.url)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    r.m.cancel_detect(SIX).unwrap();
    assert!(matches!(r.finish(), EditorEvent::DetectCancelled { .. }));
    assert!(r.player_json().is_none());
    assert!(r.leftovers().is_empty());
    // exit while running
    let mut r = Rig::new();
    r.m.start_detect(SIX, Rig::params(&server.url)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    r.m.shutdown_detect(Duration::from_secs(5));
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "exit waited too long"
    );
    assert!(r.player_json().is_none());
    assert!(r.leftovers().is_empty());
    // switch track = close
    let mut r = Rig::new();
    r.m.start_detect(SIX, Rig::params(&server.url)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    r.m.close();
    assert!(matches!(r.finish(), EditorEvent::DetectCancelled { .. }));
    assert!(r.player_json().is_none());
}

#[test]
fn detection_racing_with_a_save_finishes_both_cleanly() {
    use calliope_gui::import_job::RepoLock;
    use calliope_gui::repository::Repository;
    let server = real_server(Some("slow"));
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor");
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    copy_tree(&src, &root);
    let repo = Repository::new(&root);
    repo.ensure_layout().unwrap();
    let rec = repo.load_record(SIX).unwrap();
    let spec = OpenSpec {
        dir: repo.tracks_dir().join(SIX),
        id: rec.id,
        title: rec.title,
        stems: rec.stems,
        missing: rec.missing,
        variant: rec.backings.into_iter().next(),
        tempo: Default::default(),
    };
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
    let mut m = EditorManager::new(Arc::new(ManualBackend::new()), sink);
    m.open(&spec).unwrap();
    m.set_stem(SIX, "vocals", Some(0.0), true, Instant::now())
        .unwrap();
    let lock: Arc<dyn RepoLock> = Arc::new(|f: &mut dyn FnMut()| f());
    m.start_detect(SIX, Rig::params(&server.url)).unwrap();
    m.start_save(SIX, repo.clone(), lock).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        m.tick(Instant::now());
        let ev = events.lock().unwrap();
        let saved = ev.iter().any(|e| {
            matches!(
                e,
                EditorEvent::Saved { .. }
                    | EditorEvent::SaveFailed { .. }
                    | EditorEvent::SaveCancelled { .. }
            )
        });
        let det = ev.iter().any(|e| {
            matches!(
                e,
                EditorEvent::Detected { .. }
                    | EditorEvent::DetectFailed { .. }
                    | EditorEvent::DetectCancelled { .. }
            )
        });
        drop(ev);
        if saved && det {
            break;
        }
        assert!(
            Instant::now() < end,
            "race did not finish: {:?}",
            events
                .lock()
                .unwrap()
                .iter()
                .map(|e| format!("{e:?}").chars().take(40).collect::<String>())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // whichever way each ended, the files are valid JSON
    let dir = repo.tracks_dir().join(SIX);
    serde_json::from_slice::<Value>(&fs::read(dir.join("track.json")).unwrap()).unwrap();
    if let Ok(b) = fs::read(dir.join("player.json")) {
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["schema_version"], 6);
    }
    let stray: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".calliope-"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

// ================================================================ A2, A3: 0.3 s debounce, non-blocking

#[test]
fn a2_a3_waveform_recomputes_once_300ms_after_the_last_change_without_blocking() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    let mut r = Rig::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (c2, rx2) = (calls.clone(), release_rx.clone());
    r.m.set_peaks_fn(Arc::new(
        move |_stems, _gains, _frames, buckets, _cancel| {
            c2.fetch_add(1, Ordering::SeqCst);
            let _ = rx2.lock().unwrap().recv_timeout(Duration::from_secs(10));
            Some(vec![0.25; buckets])
        },
    ));
    let peaks_events = |e: &Events| {
        e.lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, EditorEvent::Peaks { .. }))
            .count()
    };
    // drain whatever the open and the checks of the rig scheduled
    for _ in 0..8 {
        r.m.tick(Instant::now() + Duration::from_secs(100));
        let _ = release_tx.send(());
        std::thread::sleep(Duration::from_millis(60));
    }
    while release_rx.lock().unwrap().try_recv().is_ok() {}
    r.events.lock().unwrap().clear();
    calls.store(0, Ordering::SeqCst);

    let t0 = Instant::now() + Duration::from_secs(1000);
    r.m.set_stem(SIX, "vocals", Some(-6.0), true, t0).unwrap();
    r.m.tick(t0 + Duration::from_millis(299));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "not before 0.3 s");
    // a second change 250 ms later restarts the wait
    r.m.set_stem(
        SIX,
        "drums",
        Some(-3.0),
        true,
        t0 + Duration::from_millis(250),
    )
    .unwrap();
    r.m.tick(t0 + Duration::from_millis(549));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "restarted from the last change"
    );
    let t = Instant::now();
    r.m.tick(t0 + Duration::from_millis(551));
    let end = Instant::now() + Duration::from_secs(5);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    // the computation is stuck (blocked on the channel): the manager still answers at once
    let t1 = Instant::now();
    r.m.seek(SIX, 1500, t0 + Duration::from_millis(552))
        .unwrap();
    r.m.tick(t0 + Duration::from_millis(553));
    r.m.snapshot(SIX).unwrap();
    assert!(
        t1.elapsed() < Duration::from_millis(250),
        "manager blocked by the waveform worker: {:?}",
        t1.elapsed()
    );
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(peaks_events(&r.events), 0);
    release_tx.send(()).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while peaks_events(&r.events) == 0 {
        r.m.tick(t0 + Duration::from_millis(560));
        assert!(Instant::now() < end, "no Peaks event");
        std::thread::sleep(Duration::from_millis(10));
    }
    r.m.tick(t0 + Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "exactly one recomputation for the two changes"
    );
    assert_eq!(peaks_events(&r.events), 1);
}

#[test]
fn a2_debounce_type_restarts_and_fires_once() {
    use calliope_gui::mix_waveform::Debounce;
    let mut d = Debounce::new();
    let t0 = Instant::now();
    assert!(!d.is_pending() && !d.take_due(t0));
    d.touch(t0);
    assert!(!d.take_due(t0 + Duration::from_millis(299)));
    d.touch(t0 + Duration::from_millis(299));
    assert!(!d.take_due(t0 + Duration::from_millis(598)));
    assert!(d.take_due(t0 + Duration::from_millis(599)));
    assert!(!d.take_due(t0 + Duration::from_secs(9)), "fires once");
}

// ================================================================ fix round 1 re-probes

#[test]
fn fix1_typed_bpm_more_than_a_beat_before_a_detected_change_is_still_a_new_change() {
    for steps in [-100i64, 0, 11, 200] {
        let (m0, _) = map(vec![
            typed(500.0, 120.0),
            det(6500.0, 90.0),
            det(12500.0, 100.0),
        ])
        .set_change_offset(1, steps)
        .unwrap();
        let change = m0.segments[1].start_ms;
        let bases = m0.bases();
        // one beat of ticks before the change is the border: strictly inside swallows, at or before it does not
        for back_ticks in [17i64, 20, 32, 80] {
            let pos = m0.ms_at(bases[1] - back_ticks);
            let mut m = m0.clone();
            m.set_bpm_at(pos, 110.0).unwrap();
            assert_eq!(
                m.segments.len(),
                4,
                "typed {back_ticks} ticks before (move {steps}) must insert: {:?}",
                m.segments
            );
            assert_eq!(
                m.segments[2], m0.segments[1],
                "the detected change keeps bpm, start and detected_ms"
            );
            assert_eq!(m.segments[1].bpm, 110.0);
            assert_eq!(m.segments[1].detected_ms, None);
            assert_sane(&m, "typed before", true);
        }
        // strictly inside the last beat: the detected change takes the BPM and keeps its position
        for back_ticks in [1i64, 5, 15] {
            let pos = m0.ms_at(bases[1] - back_ticks);
            let mut m = m0.clone();
            m.set_bpm_at(pos, 110.0).unwrap();
            assert_eq!(m.segments.len(), 3, "{back_ticks}: {:?}", m.segments);
            assert_eq!(m.segments[1].bpm, 110.0);
            assert_eq!(m.segments[1].start_ms, change);
            assert_eq!(m.segments[1].detected_ms, m0.segments[1].detected_ms);
            assert_sane(&m, "typed inside", true);
        }
        // the change itself still moves afterwards, with the total limit
        let mut m = m0.clone();
        m.set_bpm_at(m0.ms_at(bases[1] - 5), 110.0).unwrap();
        assert!(m.set_change_offset(1, 256).is_ok());
    }
}

#[test]
fn fix1_i64_min_is_refused_and_a_full_typed_sweep_keeps_bases_strict() {
    let m = map(vec![typed(500.0, 120.0), det(6500.0, 90.0)]);
    assert!(m.set_change_offset(1, i64::MIN).is_err());
    assert!(m.set_change_offset(1, i64::MAX).is_err());
    // random typed edits (including zero BPM and equal BPMs) on maps with many changes
    let mut rng = Lcg(7);
    for _ in 0..300 {
        let mut m = map(vec![
            typed(500.0, 120.0),
            det(6500.0, 90.0),
            det(9000.0, 130.0),
            det(14000.0, 70.0),
        ]);
        for i in 1..=3usize {
            let _ = m.set_change_offset(i, 0).map(|(n, _)| m = n);
        }
        for _ in 0..4 {
            let pos = 200.0 + rng.next() * 16000.0;
            let bpm = [0.0, 60.0, 90.0, 120.0, 130.0, 250.0][(rng.next() * 6.0) as usize % 6];
            let _ = m.set_bpm_at(pos, bpm);
            if !m.is_empty() {
                assert_sane(&m, "random typed", true);
            }
        }
    }
}

#[test]
fn fix1_server_with_a_bogus_beat_tracker_still_serves_stems_and_reports_no_beats() {
    for bogus in ["/nonexistent/tracker", "/etc/passwd"] {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("stub-mode"), "ok\n").unwrap();
        let mut child = Command::new(stems_binary())
            .args(["--beat-tracker", bogus, "--separator"])
            .arg(support("stub-separator"))
            .args(["--listen", "127.0.0.1:0", "--work-dir"])
            .arg(&work)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        let mut warned = false;
        let addr = loop {
            line.clear();
            if lines.read_line(&mut line).unwrap() == 0 {
                let _ = child.kill();
                panic!("server exited with a bogus tracker {bogus}");
            }
            warned |= line.contains("warning") && line.contains("beat");
            if let Some(rest) = line.split("listening addr=").nth(1) {
                break rest.split_whitespace().next().unwrap().to_string();
            }
        };
        assert!(addr.starts_with("127.0.0.1:"));
        let out = Command::new("curl")
            .args(["-s", "--max-time", "5", &format!("http://{addr}/v1/health")])
            .output()
            .unwrap();
        let health: Value = serde_json::from_slice(&out.stdout).unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(warned, "no warning for {bogus}");
        let tasks = health["tasks"].as_array().cloned().unwrap_or_default();
        assert!(tasks.iter().any(|t| t == "stems"), "{health}");
        assert!(!tasks.iter().any(|t| t == "beats"), "{health}");
    }
}
