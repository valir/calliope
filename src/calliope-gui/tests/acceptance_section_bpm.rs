//! Acceptance tests for gui-section-bpm-detect (Rust side, one per criterion except the layout
//! one, which the GUI test checks). A `PlayerManager` with the `ManualBackend` (no sound device)
//! on a temporary copy of the library-player fixture, against the real `calliope-stems` on
//! 127.0.0.1:0 with the stub beat tracker. Track 304 (backing 330 Hz, vocals 440 Hz, guitar
//! 660 Hz, 22 050 Hz) gets a 100 BPM section at 500 ms and a 120 BPM one at 17 800 ms.

use calliope_gui::audio_out::ManualBackend;
use calliope_gui::beat_detect::DetectPhase;
use calliope_gui::player::{
    EventSink, OpenSpec, PlayerEvent, PlayerManager, PlayerSnapshot, SectionEdit,
};
use calliope_gui::player_source::{self, Pcm};
use calliope_gui::repository::Repository;
use calliope_gui::section_bpm::{BpmParams, BpmResult, Hint, ResultKind, TOO_SHORT};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WITH_TEMPO: &str = "0199c0a0-0000-7000-8000-000000000304";
const OTHER: &str = "0199c0a0-0000-7000-8000-000000000301";
const RATE: usize = 22050;

// ================================================================ the server

fn support(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/support")
        .join(name)
}

/// The real calliope-stems with the stub beat tracker; killed on drop. Stderr lines are collected.
struct Server {
    child: Child,
    url: String,
    log: Arc<Mutex<Vec<String>>>,
    tmp: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start(tracker_mode: &str) -> Server {
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
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("stub-mode"), "ok\n").unwrap();
        fs::write(work.join("stub-tracker-mode"), format!("{tracker_mode}\n")).unwrap();
        let mut child = Command::new(bin)
            .arg("--beat-tracker")
            .arg(support("stub-beat-tracker"))
            .args(["--beat-timeout-ms", "5000"])
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
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            while lines.read_line(&mut s).unwrap_or(0) > 0 {
                l2.lock().unwrap().push(s.trim_end().to_string());
                s.clear();
            }
        });
        Server {
            child,
            url: format!("http://{addr}"),
            log,
            tmp,
        }
    }

    fn set_mode(&self, mode: &str) {
        fs::write(
            self.tmp.path().join("work/stub-tracker-mode"),
            format!("{mode}\n"),
        )
        .unwrap();
    }

    fn posts(&self) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("method=POST") && l.contains("path=/v1/beats"))
            .count()
    }

    /// The piece the stub tracker saw last: interleaved samples and the frame count.
    fn last_piece(&self) -> Vec<i32> {
        let bytes = fs::read(self.tmp.path().join("work/stub-tracker-last.flac")).unwrap();
        let mut r = claxon::FlacReader::new(&bytes[..]).unwrap();
        r.samples().map(|s| s.unwrap()).collect()
    }
}

// ================================================================ the rig

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

type Events = Arc<Mutex<Vec<PlayerEvent>>>;

struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    m: PlayerManager,
    b: ManualBackend,
    ev: Events,
    server: Server,
}

fn spec_of(root: &Path, id: &str) -> OpenSpec {
    let t = Repository::new(root)
        .scan()
        .tracks
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    OpenSpec {
        id: t.id.clone(),
        title: t.title.clone(),
        dir: root.join("tracks").join(&t.id),
        sources: player_source::sources(&t),
        ffmpeg: None,
    }
}

fn rig(tracker_mode: &str) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("lib");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-player"),
        &root,
    );
    let p = root.join("tracks").join(WITH_TEMPO).join("player.json");
    let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    v["tempo"]["segments"] = serde_json::json!([
        {"start_ms": 500, "bpm": 100, "found_ms": 500},
        {"start_ms": 17800, "bpm": 120, "detected_ms": 17800}
    ]);
    fs::write(&p, v.to_string()).unwrap();
    let b = ManualBackend::new();
    let ev: Events = Arc::new(Mutex::new(Vec::new()));
    let e2 = ev.clone();
    let sink: EventSink = Arc::new(move |e| e2.lock().unwrap().push(e));
    let mut m = PlayerManager::new(Arc::new(b.clone()), sink);
    m.open(&spec_of(&root, WITH_TEMPO)).unwrap();
    Rig {
        _tmp: tmp,
        root,
        m,
        b,
        ev,
        server: Server::start(tracker_mode),
    }
}

impl Rig {
    fn snap(&self) -> PlayerSnapshot {
        self.m.snapshot(WITH_TEMPO).unwrap()
    }

    fn bpms(&self) -> Vec<f64> {
        self.snap().tempo.segments.iter().map(|s| s.bpm).collect()
    }

    fn starts(&self) -> Vec<f64> {
        self.snap()
            .tempo
            .segments
            .iter()
            .map(|s| s.start_ms)
            .collect()
    }

    fn select(&mut self, key: &str) {
        self.m
            .select_source(WITH_TEMPO, key, Instant::now())
            .unwrap();
    }

    fn seek(&mut self, ms: u64) {
        self.m.seek(WITH_TEMPO, ms, Instant::now()).unwrap();
    }

    fn detect_with(&mut self, url: &str, index: usize) -> Result<PlayerSnapshot, String> {
        let at = self.snap().tempo.segments[index].start_ms;
        self.m.start_bpm_detect(
            WITH_TEMPO,
            index,
            at,
            BpmParams {
                url: url.to_string(),
                poll: Duration::from_millis(50),
                stall: Duration::from_secs(30),
            },
        )
    }

    fn detect_at(&mut self, ms: u64) -> Result<PlayerSnapshot, String> {
        self.seek(ms);
        let url = self.server.url.clone();
        self.detect_with(&url, 0)
    }

    fn results(&self) -> Vec<BpmResult> {
        self.ev
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PlayerEvent::BpmDetected { result, .. } => Some(result.clone()),
                _ => None,
            })
            .collect()
    }

    /// Ticks every 20 ms until the `n`th `bpm-detected` event (at most 10 s).
    fn wait_result(&mut self, n: usize) -> BpmResult {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            self.m.tick(Instant::now());
            if let Some(r) = self.results().get(n - 1) {
                return r.clone();
            }
            assert!(Instant::now() < end, "no bpm-detected event");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ticks for `d`, to show that nothing is reported.
    fn idle(&mut self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.m.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_phase(&mut self, phase: DetectPhase) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            self.m.tick(Instant::now());
            if let Some(d) = self.snap().bpm_detect {
                if d.phase as u8 >= phase as u8 {
                    return;
                }
            }
            assert!(Instant::now() < end, "the run never got to {phase:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn player_json(&mut self) -> Value {
        self.m.tick(Instant::now() + Duration::from_secs(2));
        let p = self
            .root
            .join("tracks")
            .join(WITH_TEMPO)
            .join("player.json");
        serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
    }

    fn pcm(&self, key: &str) -> Pcm {
        let spec = spec_of(&self.root, WITH_TEMPO);
        let f = &spec.sources.iter().find(|s| s.key == key).unwrap().file;
        player_source::decode(
            &spec.dir.join(f),
            None,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap()
    }
}

/// The samples of `pcm` from `from_ms`, `n` frames, interleaved like the stub's copy.
fn slice(pcm: &Pcm, from_ms: f64, n: usize) -> Vec<i32> {
    let ch = pcm.channels as usize;
    let from = (from_ms * f64::from(pcm.rate) / 1000.0).round() as usize;
    pcm.samples[from * ch..(from + n) * ch]
        .iter()
        .map(|&s| i32::from(s))
        .collect()
}

fn frames(piece: &[i32], pcm: &Pcm) -> usize {
    piece.len() / pcm.channels as usize
}

// ================================================================ the criteria

#[test]
fn ac01_the_audio_sent_is_the_7_2_s_from_the_playhead() {
    let mut r = rig("bpm:93.4");
    r.select("backing:backing");
    r.detect_at(10500).unwrap();
    let res = r.wait_result(1);
    assert_eq!((res.from_ms, res.to_ms), (10500.0, 17700.0));
    let pcm = r.pcm("backing:backing");
    let piece = r.server.last_piece();
    let n = frames(&piece, &pcm);
    assert!(n.abs_diff(158_760) <= 1, "{n} frames, want 7.2 s");
    assert_eq!(
        piece,
        slice(&pcm, 10500.0, n),
        "the sent audio is the slice"
    );
}

#[test]
fn ac02_three_seconds_before_the_end_only_those_are_sent() {
    let mut r = rig("bpm:93.4");
    r.select("backing:backing");
    r.detect_at(14800).unwrap();
    let res = r.wait_result(1);
    assert_eq!((res.from_ms, res.to_ms), (14800.0, 17800.0));
    let pcm = r.pcm("backing:backing");
    let piece = r.server.last_piece();
    let n = frames(&piece, &pcm);
    assert!(n.abs_diff(66_150) <= 1, "{n} frames, want 3.0 s");
    assert_eq!(piece, slice(&pcm, 14800.0, n));
}

#[test]
fn ac03_less_than_a_bar_is_refused_and_the_server_is_not_called() {
    let mut r = rig("bpm:93.4");
    let err = r.detect_at(17000).unwrap_err();
    assert_eq!(err, TOO_SHORT);
    assert!(r.snap().bpm_detect.is_none());
    r.idle(Duration::from_millis(300));
    assert_eq!(r.server.posts(), 0, "no POST /v1/beats");
    assert!(r.results().is_empty());
    assert_eq!(r.bpms(), vec![100.0, 120.0]);
}

#[test]
fn ac04_the_piece_comes_from_the_selected_source_not_the_backing() {
    let mut r = rig("bpm:93.4");
    r.select("stem:guitar");
    r.detect_at(10500).unwrap();
    r.wait_result(1);
    let guitar = r.pcm("stem:guitar");
    let backing = r.pcm("backing:backing");
    let piece = r.server.last_piece();
    let n = frames(&piece, &guitar);
    assert_eq!(piece, slice(&guitar, 10500.0, n), "the guitar slice");
    assert_ne!(piece, slice(&backing, 10500.0, n), "not the backing");
}

#[test]
fn ac05_the_detected_bpm_replaces_the_section_and_is_saved() {
    let mut r = rig("bpm:93.4");
    r.detect_at(10500).unwrap();
    let res = r.wait_result(1);
    assert_eq!(
        (res.outcome, res.new_bpm),
        (ResultKind::Applied, Some(93.4))
    );
    assert_eq!(r.bpms(), vec![93.4, 120.0]);
    assert_eq!(r.starts(), vec![500.0, 17800.0]);
    let json = r.player_json();
    let segs = json["tempo"]["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 2);
    assert_eq!(
        (segs[0]["start_ms"].as_f64(), segs[0]["bpm"].as_f64()),
        (Some(500.0), Some(93.4))
    );
    assert_eq!(
        (segs[1]["start_ms"].as_f64(), segs[1]["bpm"].as_f64()),
        (Some(17800.0), Some(120.0))
    );
}

#[test]
fn ac06_the_result_carries_old_new_range_and_the_twice_half_hint() {
    let mut r = rig("bpm:93.4");
    r.detect_at(10500).unwrap();
    let a = r.wait_result(1);
    assert_eq!(
        (a.old_bpm, a.new_bpm, a.from_ms, a.to_ms),
        (100.0, Some(93.4), 10500.0, 17700.0)
    );
    assert_eq!(a.hint, None);
    // about twice the old one
    r.server.set_mode("bpm:186.8");
    r.detect_at(10500).unwrap();
    let b = r.wait_result(2);
    assert_eq!(
        (b.old_bpm, b.new_bpm, b.hint),
        (93.4, Some(186.8), Some(Hint::Double))
    );
    // and about half
    r.server.set_mode("bpm:93.4");
    r.detect_at(10500).unwrap();
    let c = r.wait_result(3);
    assert_eq!(
        (c.old_bpm, c.new_bpm, c.hint),
        (186.8, Some(93.4), Some(Hint::Half))
    );
}

#[test]
fn ac07_server_down_or_no_steady_beat_says_so_and_keeps_the_bpm() {
    let mut r = rig("irregular");
    r.detect_at(10500).unwrap();
    let a = r.wait_result(1);
    assert_eq!(a.outcome, ResultKind::Failed);
    assert!(a.message.unwrap().contains("No steady beat"));
    assert_eq!(r.bpms(), vec![100.0, 120.0]);

    // a closed port on this machine
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    r.seek(10500);
    r.detect_with(&format!("http://127.0.0.1:{port}"), 0)
        .unwrap();
    let b = r.wait_result(2);
    assert_eq!(b.outcome, ResultKind::Failed);
    assert!(b
        .message
        .unwrap()
        .contains("Cannot reach the edge-AI server"));
    assert_eq!(r.bpms(), vec![100.0, 120.0]);
    assert!(r.snap().bpm_detect.is_none());
}

#[test]
fn ac08_cancel_keeps_the_bpm_and_frees_the_button() {
    let mut r = rig("slow-bpm:150");
    r.detect_at(10500).unwrap();
    r.wait_phase(DetectPhase::Uploading);
    assert!(r.detect_at(10500).unwrap_err().contains("already running"));
    let snap = r.m.cancel_bpm_detect(WITH_TEMPO).unwrap();
    assert!(snap.bpm_detect.is_none());
    r.idle(Duration::from_secs(4));
    assert!(r.results().is_empty(), "a cancelled run reports nothing");
    assert_eq!(r.bpms(), vec![100.0, 120.0]);
    // enabled again: a new run starts
    r.server.set_mode("bpm:93.4");
    r.detect_at(10500).unwrap();
    assert_eq!(r.wait_result(1).new_bpm, Some(93.4));
}

#[test]
fn ac09_a_removed_section_or_another_track_drops_the_result() {
    let mut r = rig("slow-bpm:150");
    // the second section (2.1 s from 17 900 ms, a bar at 120 BPM is 2.0 s)
    r.seek(17900);
    let url = r.server.url.clone();
    r.detect_with(&url, 1).unwrap();
    r.wait_phase(DetectPhase::Uploading);
    let remove = SectionEdit::Remove {
        index: 1,
        at: 17800.0,
    };
    r.m.edit_sections(WITH_TEMPO, remove, Instant::now())
        .unwrap();
    let res = r.wait_result(1);
    assert_eq!(res.outcome, ResultKind::Dropped);
    assert_eq!(r.bpms(), vec![100.0], "the map is as the removal left it");

    // another track opened during a run
    r.detect_at(10500).unwrap();
    r.wait_phase(DetectPhase::Uploading);
    let other = spec_of(&r.root, OTHER);
    r.m.open(&other).unwrap();
    r.idle(Duration::from_secs(4));
    assert_eq!(r.results().len(), 1, "no second result");
    let json = r.player_json();
    assert_eq!(json["tempo"]["segments"][0]["bpm"].as_f64(), Some(100.0));
    r.m.open(&spec_of(&r.root, WITH_TEMPO)).unwrap();
    assert_eq!(r.bpms(), vec![100.0], "the dropped run changed nothing");
}

/// Frames at which a click starts: where `got` leaves `want` by more than 0.05, merged within 300.
fn clicks(got: &[f32], want: &[f32]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut last: Option<usize> = None;
    for f in 200..got.len() / 2 {
        if (got[2 * f] - want[2 * f]).abs() > 0.05 {
            if last.is_none_or(|l| f > l + 300) {
                out.push(f);
            }
            last = Some(f);
        }
    }
    out
}

#[test]
fn ac10_playing_with_the_click_on_the_clicks_follow_the_new_bpm() {
    let mut r = rig("bpm:120");
    r.select("stem:vocals");
    for (i, at) in [(0, 500.0), (1, 17800.0)] {
        let e = SectionEdit::Flags {
            index: i,
            at,
            metronome: true,
            count_in: false,
        };
        r.m.edit_sections(WITH_TEMPO, e, Instant::now()).unwrap();
    }
    r.seek(2000);
    r.m.play(WITH_TEMPO, Instant::now()).unwrap();
    let mut got = r.b.pull(2 * RATE);
    r.detect_at_playing();
    // pull 20 ms at a time until the result is applied
    let end = Instant::now() + Duration::from_secs(10);
    let edit_frame = loop {
        r.m.tick(Instant::now());
        if !r.results().is_empty() {
            break got.len() / 2;
        }
        assert!(Instant::now() < end, "no result");
        got.extend(r.b.pull(RATE / 50));
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(r.results()[0].outcome, ResultKind::Applied);
    got.extend(r.b.pull(2 * RATE));
    assert!(r.snap().transport.playing, "playback went on");
    r.m.pause(WITH_TEMPO, Instant::now()).unwrap();

    let pcm = r.pcm("stem:vocals");
    let ch = pcm.channels as usize;
    let n = got.len() / 2;
    let start = 2000 * RATE / 1000;
    let mut want = Vec::with_capacity(got.len());
    for i in start..start + n {
        let l = f32::from(pcm.samples[i * ch]) / 32768.0;
        want.push(l);
        want.push(if ch >= 2 {
            f32::from(pcm.samples[i * ch + 1]) / 32768.0
        } else {
            l
        });
    }
    let on = clicks(&got, &want);
    let ms = |f: usize| f as f64 * 1000.0 / RATE as f64;
    let gaps = |v: &[usize]| {
        v.windows(2)
            .map(|w| ms(w[1]) - ms(w[0]))
            .collect::<Vec<_>>()
    };
    let before: Vec<usize> = on.iter().copied().filter(|&f| f < edit_frame).collect();
    let after: Vec<usize> = on
        .iter()
        .copied()
        .filter(|&f| ms(f) > ms(edit_frame) + 600.0)
        .collect();
    assert!(
        before.len() >= 3 && after.len() >= 3,
        "{on:?} edit at {edit_frame}"
    );
    for g in gaps(&before) {
        assert!((g - 600.0).abs() < 10.0, "before the edit: {g}");
    }
    for g in gaps(&after) {
        assert!((g - 500.0).abs() < 10.0, "after the edit: {g}");
    }
}

impl Rig {
    /// Starts the run for section 0 without moving the playhead.
    fn detect_at_playing(&mut self) {
        let url = self.server.url.clone();
        self.detect_with(&url, 0).unwrap();
    }
}
