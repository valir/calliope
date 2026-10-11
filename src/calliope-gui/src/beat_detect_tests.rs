//! The detection job against the REAL `calliope-stems` on 127.0.0.1 (stub separator, stub beat
//! tracker, 1 s tracker timeout), driven through `EditorManager` on the "Six Lanes" fixture track
//! cut down to two 16 s stems (the stub tracker's results depend on the length). Never a real
//! audio device or server.

use super::*;
use crate::editor::tests::{manager, spec_of, Events};
use crate::editor::{EditorManager, OpenSpec};
use crate::tempo_store::FileTempoStore;
use crate::test_server::{pgrep, TestServer};

const SIX: &str = "0199c0a0-0000-7000-8000-000000000204";
const OLD_PLAYER_JSON: &str = r#"{"schema_version":2,"position_ms":777,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":100.0,"bpm":100.0}]}}"#;

struct Rig {
    _tmp: tempfile::TempDir,
    spec: OpenSpec,
    m: EditorManager,
    events: Events,
    server: TestServer,
}

impl Rig {
    fn new(server: TestServer) -> Rig {
        let (tmp, mut spec) = spec_of(SIX);
        spec.stems.truncate(2); // vocals, drums
        for s in &spec.stems {
            let out = spec.dir.join(&s.file);
            let status = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:sample_rate=22050:duration=16"])
                .args(["-ac", "1", "-sample_fmt", "s16", "-c:a", "flac"])
                .arg(&out)
                .status()
                .expect("ffmpeg is needed");
            assert!(status.success());
        }
        let (mut m, _backend, events) = manager();
        m.open(&spec).unwrap();
        let now = Instant::now();
        for name in ["vocals", "drums"] {
            m.set_stem(SIX, name, Some(0.0), true, now).unwrap();
        }
        Rig { _tmp: tmp, spec, m, events, server }
    }

    fn params(url: &str) -> DetectParams {
        DetectParams { url: url.into(), store: Arc::new(FileTempoStore), poll: Duration::from_millis(50), stall: Duration::from_secs(10) }
    }

    fn start(&mut self) -> Result<crate::editor::EditorSnapshot, String> {
        let p = Rig::params(&self.server.url);
        self.m.start_detect(SIX, p)
    }

    fn final_event(&self) -> Option<EditorEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|e| matches!(e, EditorEvent::Detected { .. } | EditorEvent::DetectFailed { .. } | EditorEvent::DetectCancelled { .. }))
            .cloned()
    }

    /// Ticks until the one final event arrives.
    fn finish(&mut self) -> EditorEvent {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            self.m.tick(Instant::now());
            if let Some(e) = self.final_event() {
                return e;
            }
            assert!(Instant::now() < end, "no final event; got {:?}", self.events.lock().unwrap().last());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn run(&mut self) -> EditorEvent {
        self.events.lock().unwrap().clear();
        self.start().unwrap();
        self.finish()
    }

    fn phases(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                EditorEvent::Detect { state, .. } => Some(state.phase.name()),
                _ => None,
            })
            .collect();
        v.dedup();
        v
    }

    fn wait_phase(&mut self, phase: &str) {
        let end = Instant::now() + Duration::from_secs(20);
        while !self.phases().contains(&phase) {
            self.m.tick(Instant::now());
            assert!(Instant::now() < end, "never reached {phase}: {:?}", self.phases());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn player_json(&self) -> Option<serde_json::Value> {
        std::fs::read(self.spec.dir.join("player.json")).ok().map(|b| serde_json::from_slice(&b).unwrap())
    }

    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(&self.spec.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".calliope-beats-"))
            .collect()
    }

    fn segments(&self) -> Vec<(f64, f64, Option<f64>)> {
        self.player_json().unwrap()["tempo"]["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["start_ms"].as_f64().unwrap(), s["bpm"].as_f64().unwrap(), s["detected_ms"].as_f64()))
            .collect()
    }
}

fn failed(e: EditorEvent) -> String {
    match e {
        EditorEvent::DetectFailed { message, .. } => message,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn ok_detects_two_sections_and_stores_them() {
    let mut r = Rig::new(TestServer::start_beats("ok", &[]));
    let EditorEvent::Detected { id, tempo, beats, replaced } = r.run() else { panic!("not detected") };
    assert_eq!((id.as_str(), beats, replaced), (SIX, 27, false));
    assert_eq!(tempo.segments.len(), 2);
    assert!(tempo.multi);
    let want = vec![(500.0, 120.0, None), (6500.0, 90.0, Some(6500.0))];
    assert_eq!(r.segments(), want);
    assert_eq!(r.player_json().unwrap()["schema_version"], 2);
    assert_eq!(r.m.snapshot(SIX).unwrap().tempo.segments.len(), 2, "the session has the new map");
    assert_eq!(r.m.snapshot(SIX).unwrap().detect, None);
    assert!(r.leftovers().is_empty());
    let order = ["checking", "mixing", "uploading", "waiting", "detecting", "applying"];
    let got = r.phases();
    let mut it = order.iter();
    assert!(got.iter().all(|p| it.any(|o| o == p)), "phases out of order: {got:?}");
    for p in ["checking", "mixing", "uploading", "waiting", "applying"] {
        assert!(got.contains(&p), "missing {p}: {got:?}");
    }
    assert_eq!(r.events.lock().unwrap().iter().filter(|e| matches!(e, EditorEvent::Detected { .. })).count(), 1);
    // a second run replaces the map
    let EditorEvent::Detected { replaced, .. } = r.run() else { panic!("not detected") };
    assert!(replaced);
    assert!(r.server.saw("kind=beats"), "{:?}", r.server.log.lock().unwrap());
}

#[test]
fn steady_gives_one_segment_and_bar_end_a_last_segment_at_60() {
    let mut r = Rig::new(TestServer::start_beats("steady", &[]));
    assert!(matches!(r.run(), EditorEvent::Detected { .. }));
    assert_eq!(r.segments(), vec![(500.0, 120.0, None)]);
    let mut r = Rig::new(TestServer::start_beats("bar-end", &[]));
    assert!(matches!(r.run(), EditorEvent::Detected { .. }));
    let segs = r.segments();
    assert_eq!(segs, vec![(500.0, 120.0, None), (12500.0, 60.0, Some(12500.0))]);
    assert!(r.leftovers().is_empty());
}

#[test]
fn failures_have_exact_messages_and_leave_player_json_alone() {
    for (mode, want) in [
        ("few", tempo_detect::NO_BEAT_ERROR.to_string()),
        ("fail", "Beat detection failed on the server: the beat tracker failed (exit status 3): stub-beat-tracker: simulated failure".to_string()),
        ("bad-output", "Beat detection failed on the server: the beat tracker produced invalid output (not a beats document)".to_string()),
    ] {
        let mut r = Rig::new(TestServer::start_beats(mode, &[]));
        std::fs::write(r.spec.dir.join("player.json"), OLD_PLAYER_JSON).unwrap();
        let message = failed(r.run());
        assert_eq!(message, want, "{mode}");
        assert_eq!(std::fs::read_to_string(r.spec.dir.join("player.json")).unwrap(), OLD_PLAYER_JSON, "{mode}");
        assert!(r.leftovers().is_empty(), "{mode}");
        assert_eq!(r.m.snapshot(SIX).unwrap().detect, None);
    }
}

#[test]
fn a_server_without_a_tracker_and_one_that_is_not_there() {
    let mut r = Rig::new(TestServer::start("ok"));
    assert_eq!(failed(r.run()), NO_BEATS_TASK);
    assert!(r.leftovers().is_empty());
    assert!(r.player_json().is_none());

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    r.events.lock().unwrap().clear();
    r.m.start_detect(SIX, Rig::params(&url)).unwrap();
    assert_eq!(failed(r.finish()), format!("Cannot reach the edge-AI server at {url}"));
}

#[test]
fn refusals() {
    let mut r = Rig::new(TestServer::start_beats("hang", &["--beat-timeout-ms", "20000"]));
    assert_eq!(r.m.start_detect(SIX, Rig::params("  ")).unwrap_err(), "Set the edge-AI server in Settings first");
    let now = Instant::now();
    r.m.set_stem(SIX, "vocals", Some(0.0), false, now).unwrap();
    r.m.set_stem(SIX, "drums", None, true, now).unwrap();
    assert_eq!(r.start().unwrap_err(), "Check a stem first");
    r.m.set_stem(SIX, "drums", Some(0.0), true, now).unwrap();
    assert!(r.start().unwrap().detect.is_some());
    assert_eq!(r.start().unwrap_err(), "Beat detection is already running");
    r.m.cancel_detect(SIX).unwrap();
    assert!(matches!(r.finish(), EditorEvent::DetectCancelled { .. }));
    assert!(r.leftovers().is_empty());
}

#[test]
fn cancelling_a_hanging_tracker_removes_the_server_job_and_the_process() {
    let mut r = Rig::new(TestServer::start_beats("hang", &["--beat-timeout-ms", "20000"]));
    r.start().unwrap();
    r.wait_phase("detecting");
    let jobs = r.server.outer.path().join("work/jobs");
    let id = std::fs::read_dir(&jobs).unwrap().flatten().next().expect("a job folder").file_name().to_string_lossy().into_owned();
    let pattern = format!("{}/", jobs.display());
    let end = Instant::now() + Duration::from_secs(10);
    while !pgrep(&pattern) {
        assert!(Instant::now() < end, "the tracker never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    r.m.cancel_detect(SIX).unwrap();
    assert!(matches!(r.finish(), EditorEvent::DetectCancelled { .. }));
    let client = StemsClient::new(&r.server.url);
    let end = Instant::now() + Duration::from_secs(10);
    while pgrep(&pattern) || !matches!(client.status(&id), Err(ClientError::Restarted)) {
        assert!(Instant::now() < end, "the job or the tracker is still there");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(r.leftovers().is_empty());
    assert!(r.player_json().is_none());
}

#[test]
fn closing_the_session_cancels_the_detection() {
    let mut r = Rig::new(TestServer::start_beats("slow", &["--beat-timeout-ms", "20000"]));
    r.start().unwrap();
    r.wait_phase("uploading");
    r.m.close();
    assert!(matches!(r.finish(), EditorEvent::DetectCancelled { id } if id == SIX));
    assert!(r.leftovers().is_empty());
    assert!(r.player_json().is_none());
    // the manager is free again
    r.m.begin_open();
    assert!(r.m.current().is_none());
}
