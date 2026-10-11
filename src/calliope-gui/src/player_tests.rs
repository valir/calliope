//! Player engine and manager tests: copies of `tests/fixtures/library-player`, a
//! `ManualBackend` (never a real device) and injected time.

use super::*;
use crate::audio_out::ManualBackend;
use crate::repository::Repository;
use std::path::Path;
use std::sync::Mutex;

const PRACTICE: &str = "Practice Song";
const WITH_TEMPO: &str = "With Tempo";
const RATE: usize = 22050;

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn spec_of(title: &str) -> (tempfile::TempDir, OpenSpec) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-player");
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("lib");
    copy_tree(&from, &root);
    let t = Repository::new(&root).scan().tracks.into_iter().find(|t| t.title == title).unwrap();
    let spec = OpenSpec {
        id: t.id.clone(),
        title: t.title.clone(),
        dir: root.join("tracks").join(&t.id),
        sources: player_source::sources(&t),
        ffmpeg: None,
    };
    (tmp, spec)
}

type Events = Arc<Mutex<Vec<PlayerEvent>>>;

fn manager() -> (PlayerManager, ManualBackend, Events) {
    let backend = ManualBackend::new();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
    (PlayerManager::new(Arc::new(backend.clone()), sink), backend, events)
}

fn opened(title: &str) -> (tempfile::TempDir, OpenSpec, PlayerManager, ManualBackend, Events) {
    let (t, spec) = spec_of(title);
    let (mut m, b, e) = manager();
    m.open(&spec).unwrap();
    (t, spec, m, b, e)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn decoded(spec: &OpenSpec, key: &str) -> Pcm {
    let f = &spec.sources.iter().find(|s| s.key == key).unwrap().file;
    player_source::decode(&spec.dir.join(f), None, &AtomicBool::new(false), &mut |_, _| {}).unwrap()
}

/// Interleaved stereo f32 of `frames` frames from `start` (mono copied to both channels).
fn expected(pcm: &Pcm, start: usize, frames: usize) -> Vec<f32> {
    let ch = pcm.channels as usize;
    let mut v = Vec::new();
    for i in start..start + frames {
        let l = f32::from(pcm.samples[i * ch]) / 32768.0;
        let r = if ch >= 2 { f32::from(pcm.samples[i * ch + 1]) / 32768.0 } else { l };
        v.push(l);
        v.push(r);
    }
    v
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn disk(spec: &OpenSpec) -> Option<serde_json::Value> {
    let b = std::fs::read(spec.dir.join("player.json")).ok()?;
    serde_json::from_slice(&b).ok()
}

fn fade_frames() -> usize {
    RATE * FADE_MS as usize / 1000
}

#[test]
fn open_lists_sources_and_restores_position() {
    let (_t, spec, m, _b, _e) = opened(PRACTICE);
    let s = m.snapshot(&spec.id).unwrap();
    assert_eq!(s.sources.len(), 4);
    assert_eq!(s.source, "backing:backing");
    assert_eq!(s.duration_ms, 20000);
    assert_eq!(s.transport.position_ms, 0);
    assert_eq!(s.mode, "time");
    assert_eq!(s.peaks.len(), PEAK_BUCKETS);
    assert!(disk(&spec).is_none(), "opening does not write");

    let (_t, spec, m, _b, _e) = opened(WITH_TEMPO);
    let s = m.snapshot(&spec.id).unwrap();
    assert_eq!(s.transport.position_ms, 4000);
    assert_eq!(s.source, "stem:guitar");
    assert_eq!(s.mode, "bpm");
    assert_eq!(s.loops.len(), 3);
    assert_eq!(s.current_loop, Some(0));
    assert!(s.tempo.multi);
}

#[test]
fn open_without_audio_fails_and_unknown_session_is_refused() {
    let (_t, spec) = spec_of("Missing Audio");
    let (mut m, _b, _e) = manager();
    assert!(m.open(&spec).unwrap_err().contains("no audio to play"));
    assert!(!m.is_opening());
    assert!(m.play("x", Instant::now()).unwrap_err().contains("no player session for x"));
}

#[test]
fn play_equals_decoded_samples() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let pcm = decoded(&spec, "backing:backing");
    let t0 = Instant::now();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(st.playing);
    let got = b.pull(2000);
    let want = expected(&pcm, 0, 2000);
    let n = fade_frames();
    assert_eq!(got[n * 2..], want[n * 2..]);
    assert_eq!(got[0], 0.0, "fade-in starts at zero");
    assert!(got[..n * 2].iter().zip(&want).all(|(g, w)| g.abs() <= w.abs() + 1e-6));
    assert_eq!(m.snapshot(&spec.id).unwrap().transport.position_ms, 2000 * 1000 / RATE as u64);
}

#[test]
fn pause_is_silent_frozen_and_saved() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(RATE); // 1 s
    let st = m.pause(&spec.id, t0).unwrap();
    assert!(!st.playing);
    assert_eq!(st.position_ms, 1000);
    assert_eq!(peak(&b.pull(500)), 0.0);
    assert_eq!(m.snapshot(&spec.id).unwrap().transport.position_ms, 1000);
    m.tick(t0 + ms(499));
    assert!(disk(&spec).is_none());
    m.tick(t0 + ms(500));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 1000);
}

#[test]
fn seek_while_playing_is_silent_until_300ms_and_skip_clamps() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let pcm = decoded(&spec, "backing:backing");
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(100);
    let st = m.seek(&spec.id, 5000, t0).unwrap();
    assert!(st.playing && st.resume_pending && st.position_ms == 5000);
    assert_eq!(peak(&b.pull(200)), 0.0);
    m.tick(t0 + ms(299));
    assert_eq!(peak(&b.pull(200)), 0.0);
    m.tick(t0 + ms(300));
    assert!(!m.snapshot(&spec.id).unwrap().transport.resume_pending);
    let got = b.pull(1000);
    let start = 5000 * RATE / 1000;
    let want = expected(&pcm, start, 1000);
    let n = fade_frames();
    assert_eq!(got[n * 2..], want[n * 2..]);

    // skip: instant, clamped
    m.pause(&spec.id, t0).unwrap();
    m.seek(&spec.id, 2000, t0).unwrap();
    assert_eq!(m.skip(&spec.id, -5000, t0).unwrap().position_ms, 0);
    assert_eq!(m.skip(&spec.id, 5000, t0).unwrap().position_ms, 5000);
    assert_eq!(m.skip(&spec.id, 60_000, t0).unwrap().position_ms, 20000);
    assert_eq!(m.nudge(&spec.id, -100, t0).unwrap().position_ms, 19900);
    assert_eq!(m.seek(&spec.id, 99_999, t0).unwrap().position_ms, 20000);
}

#[test]
fn skip_while_playing_sounds_at_once() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(100);
    m.skip(&spec.id, 5000, t0).unwrap();
    assert!(peak(&b.pull(1000)) > 0.0);
}

#[test]
fn source_change_stops_and_keeps_position() {
    let (_t, spec, mut m, b, e) = opened(PRACTICE);
    let guitar = decoded(&spec, "stem:guitar");
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(10);
    m.seek(&spec.id, 4000, t0).unwrap();
    m.tick(t0 + ms(300));
    let snap = m.select_source(&spec.id, "stem:guitar", t0 + ms(300)).unwrap();
    assert_eq!(snap.source, "stem:guitar");
    assert!(!snap.transport.playing);
    assert_eq!(snap.transport.position_ms, 4000);
    assert!(!b.is_open());
    let last = e.lock().unwrap().iter().rev().find_map(|x| match x {
        PlayerEvent::Transport { state, .. } => Some(state.clone()),
        _ => None,
    });
    assert!(!last.unwrap().playing);
    m.play(&spec.id, t0 + ms(400)).unwrap();
    let got = b.pull(1000);
    let start = 4000 * RATE / 1000;
    let want = expected(&guitar, start, 1000);
    let n = fade_frames();
    assert_eq!(got[n * 2..], want[n * 2..]);
    // the choice is saved
    m.tick(t0 + ms(900));
    assert_eq!(disk(&spec).unwrap()["source"], "stem:guitar");
    // unknown key and same key
    assert!(m.select_source(&spec.id, "stem:nope", t0).is_err());
    assert_eq!(m.select_source(&spec.id, "stem:guitar", t0).unwrap().source, "stem:guitar");
}

#[test]
fn end_of_track_stops_at_zero_and_saves() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.seek(&spec.id, 19_900, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(RATE); // runs past the end
    m.tick(t0 + ms(20));
    let s = m.snapshot(&spec.id).unwrap();
    assert!(!s.transport.playing);
    assert_eq!(s.transport.position_ms, 0);
    assert_eq!(disk(&spec).unwrap()["position_ms"], 0);
    assert_eq!(peak(&b.pull(100)), 0.0);
    // Play again starts from the top.
    m.play(&spec.id, t0 + ms(40)).unwrap();
    assert!(peak(&b.pull(RATE / 10)) > 0.0);
}

#[test]
fn save_is_debounced_by_500ms() {
    let (_t, spec, mut m, _b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.seek(&spec.id, 3000, t0).unwrap();
    m.tick(t0 + ms(499));
    assert!(disk(&spec).is_none());
    m.tick(t0 + ms(500));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 3000);
    // a later change restarts the wait
    m.seek(&spec.id, 4000, t0 + ms(600)).unwrap();
    m.seek(&spec.id, 5000, t0 + ms(900)).unwrap();
    m.tick(t0 + ms(1399));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 3000);
    m.tick(t0 + ms(1400));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 5000);
}

#[test]
fn position_is_saved_every_10s_while_playing() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(RATE * 2);
    m.tick(t0 + ms(9999));
    assert!(disk(&spec).is_none());
    m.tick(t0 + ms(10_000));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 2000);
}

#[test]
fn close_flushes_and_reopen_restores() {
    let (_t, spec, mut m, _b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.select_source(&spec.id, "original", t0).unwrap();
    m.seek(&spec.id, 7500, t0).unwrap();
    m.close();
    let j = disk(&spec).unwrap();
    assert_eq!(j["source"], "original");
    assert_eq!(j["position_ms"], 7500);
    let (mut m2, _b2, _e2) = manager();
    let s = m2.open(&spec).unwrap();
    assert_eq!(s.source, "original");
    assert_eq!(s.transport.position_ms, 7500);
}

#[test]
fn close_without_changes_writes_nothing_and_leave_flushes() {
    let (_t, spec, mut m, b, _e) = opened(PRACTICE);
    m.close();
    assert!(disk(&spec).is_none());

    let (_t, spec, mut m, b2, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b2.pull(RATE);
    let st = m.leave(&spec.id, t0).unwrap();
    assert!(!st.playing);
    assert!(!b2.is_open());
    assert_eq!(disk(&spec).unwrap()["position_ms"], 1000);
    assert!(!b.is_open());
}

#[test]
fn past_the_end_position_restores_as_zero() {
    let (_t, spec) = spec_of(WITH_TEMPO);
    let p = spec.dir.join("player.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["position_ms"] = 999_999.into();
    std::fs::write(&p, v.to_string()).unwrap();
    let (mut m, _b, _e) = manager();
    assert_eq!(m.open(&spec).unwrap().transport.position_ms, 0);
}

#[test]
fn second_open_supersedes_a_running_one() {
    let (_t, spec) = spec_of(PRACTICE);
    let (mut m, _b, _e) = manager();
    let t1 = m.begin_open();
    let sink = m.sink();
    let loaded = load(&spec, &t1, &sink).unwrap();
    let t2 = m.begin_open();
    assert!(t1.cancel.load(Ordering::Relaxed));
    assert_eq!(m.finish_open(&t1, &spec, loaded).unwrap_err(), SUPERSEDED);
    assert!(m.is_opening());
    let loaded = load(&spec, &t2, &sink).unwrap();
    assert!(m.finish_open(&t2, &spec, loaded).is_ok());
    assert!(!m.is_opening());
}

#[test]
fn device_failure_stops_and_reports() {
    let (_t, spec, mut m, b, e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.fail("gone");
    m.tick(t0 + ms(20));
    assert!(!m.snapshot(&spec.id).unwrap().transport.playing);
    assert!(e.lock().unwrap().iter().any(|x| matches!(x, PlayerEvent::AudioError { message, .. } if message == "gone")));
}

#[test]
fn transport_events_are_throttled_while_playing() {
    let (_t, spec, mut m, b, e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    let count = |e: &Events| e.lock().unwrap().iter().filter(|x| matches!(x, PlayerEvent::Transport { .. })).count();
    let base = count(&e);
    b.pull(1000);
    for i in 1..5 {
        m.tick(t0 + ms(20 * i));
    }
    assert_eq!(count(&e), base);
    m.tick(t0 + ms(100));
    assert_eq!(count(&e), base + 1);
}

#[test]
fn engine_end_of_source_sets_flag() {
    let pcm = Arc::new(Pcm { rate: 1000, channels: 1, frames: 10, samples: vec![16384; 10] });
    let shared = Arc::new(Shared::default());
    let mut en = Engine::new(pcm, 8, shared.clone());
    en.apply(Cmd::Play);
    let mut out = [0.0f32; 8];
    en.render(&mut out);
    assert!(shared.ended.load(Ordering::Acquire));
    assert_eq!(shared.position.load(Ordering::Acquire), 10);
    assert_eq!(out[4..], [0.0; 4]);
}
