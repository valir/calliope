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
    en.apply(Cmd::Play { count_in: None });
    let mut out = [0.0f32; 8];
    en.render(&mut out);
    assert!(shared.ended.load(Ordering::Acquire));
    assert_eq!(shared.position.load(Ordering::Acquire), 10);
    assert_eq!(out[4..], [0.0; 4]);
}

// ------------------------------------------------------- metronome, count-in, loops

use crate::metronome::Click;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Counts allocations made by the thread that switched `COUNTING` on.
struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCS.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            ALLOCS.with(|a| a.set(a.get() + 1));
        }
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const GUITAR: &str = "stem:guitar";
// With Tempo: 120 BPM from 500 ms, 90 BPM from 10500 ms. loop-1 2500..6500 ms (repeat),
// loop-2 about 13167..15833 ms (no repeat, count-in), loop-3 starts at 18500 ms, no end.
const LOOP1_START: usize = 2500 * RATE / 1000;
const LOOP1_END: usize = 6500 * RATE / 1000;

fn close(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() < 1e-5, "sample {i}: got {g}, want {w}");
    }
}

fn add_click(v: &mut [f32], frame: usize, accent: bool) {
    let c = Click::new(RATE as u32);
    for (k, x) in c.samples(accent).iter().enumerate() {
        if frame + k < v.len() / 2 {
            for ch in 0..2 {
                let s = &mut v[2 * (frame + k) + ch];
                *s = (*s + x).clamp(-1.0, 1.0);
            }
        }
    }
}

/// What the engine outputs for source frames `pos..` played through a loop `start..end`
/// (fade-in at the start of the run and after every wrap, fade-out before the end).
fn looped(pcm: &Pcm, pos: usize, start: usize, end: usize, total: usize) -> Vec<f32> {
    let n = fade_frames();
    let mut out = Vec::new();
    let (mut p, mut since) = (pos, 0usize);
    while out.len() < total * 2 {
        let mut g = if since < n { since as f32 / n as f32 } else { 1.0 };
        if end - p <= n {
            g *= (end - p) as f32 / n as f32;
        }
        let e = expected(pcm, p, 1);
        out.push(e[0] * g);
        out.push(e[1] * g);
        p += 1;
        since += 1;
        if p == end {
            p = start;
            since = 0;
        }
    }
    out
}

fn fade_in(v: &mut [f32], from: usize) {
    let n = fade_frames();
    for k in 0..n {
        for ch in 0..2 {
            v[2 * (from + k) + ch] *= k as f32 / n as f32;
        }
    }
}

fn with_tempo() -> (tempfile::TempDir, OpenSpec, PlayerManager, ManualBackend, Events, Pcm) {
    let (t, spec, m, b, e) = opened(WITH_TEMPO);
    let pcm = decoded(&spec, GUITAR);
    (t, spec, m, b, e, pcm)
}

fn wraps(m: &PlayerManager) -> u64 {
    m.session.as_ref().unwrap().shared.wraps.load(Ordering::Acquire)
}

fn pos_ms(m: &PlayerManager, id: &str) -> u64 {
    m.snapshot(id).unwrap().transport.position_ms
}

#[test]
fn metronome_clicks_at_exact_frames_with_accent() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    let snap = m.set_metronome(&spec.id, true, false, t0).unwrap();
    assert!(snap.metronome.on);
    m.seek(&spec.id, 0, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let total = RATE * 2600 / 1000;
    let got = b.pull(total);
    let mut want = expected(&pcm, 0, total);
    fade_in(&mut want, 0);
    // beats every 500 ms from 0 (beat 4 of bar 0); the bar starts at 500 and 2500
    for (k, ms) in [0usize, 500, 1000, 1500, 2000, 2500].iter().enumerate() {
        add_click(&mut want, ms * RATE / 1000, k == 1 || k == 5);
    }
    close(&got, &want);
}

#[test]
fn metronome_off_leaves_the_source_untouched() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.set_metronome(&spec.id, true, false, t0).unwrap();
    m.set_metronome(&spec.id, false, false, t0).unwrap();
    m.seek(&spec.id, 0, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let got = b.pull(RATE);
    let mut want = expected(&pcm, 0, RATE);
    fade_in(&mut want, 0);
    close(&got, &want);
}

#[test]
fn count_in_before_main_play() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.set_metronome(&spec.id, false, true, t0).unwrap();
    m.seek(&spec.id, 0, t0).unwrap();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(st.counting_in && st.playing);
    let got = b.pull(33075 + 2000);
    let mut want = vec![0.0f32; (33075 + 2000) * 2];
    for (f, a) in [(0usize, true), (11025, false), (22050, false)] {
        add_click(&mut want, f, a);
    }
    let src = expected(&pcm, 0, 2000);
    for (k, x) in src.iter().enumerate() {
        want[33075 * 2 + k] = *x;
    }
    fade_in(&mut want, 33075);
    close(&got, &want);
    let st = m.snapshot(&spec.id).unwrap().transport;
    assert!(!st.counting_in);
    assert_eq!(st.position_ms, 2000 * 1000 / RATE as u64);
}

#[test]
fn count_in_holds_the_position_and_pause_cancels_it() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.set_metronome(&spec.id, false, true, t0).unwrap();
    m.seek(&spec.id, 1000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(15000);
    let t = m.snapshot(&spec.id).unwrap().transport;
    assert!(t.counting_in);
    assert_eq!(t.position_ms, 1000);
    let st = m.pause(&spec.id, t0).unwrap();
    assert!(!st.counting_in && !st.playing);
    assert_eq!(peak(&b.pull(RATE)), 0.0);
    assert_eq!(pos_ms(&m, &spec.id), 1000);
}

#[test]
fn count_in_is_not_played_after_a_scrub() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.set_metronome(&spec.id, false, true, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(33075 + 100);
    m.seek(&spec.id, 5000, t0).unwrap();
    m.tick(t0 + ms(300));
    let st = m.snapshot(&spec.id).unwrap().transport;
    assert!(!st.counting_in);
    assert!(peak(&b.pull(2000)) > 0.0);
}

#[test]
fn loop_play_repeats_without_count_in() {
    let (_t, spec, mut m, b, e, pcm) = with_tempo();
    let t0 = Instant::now();
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.loop_playing && !snap.transport.counting_in);
    assert_eq!(snap.transport.position_ms, 2500);
    let len = LOOP1_END - LOOP1_START;
    let total = len + 3000;
    let got = b.pull(total);
    close(&got, &looped(&pcm, LOOP1_START, LOOP1_START, LOOP1_END, total));
    assert_eq!(wraps(&m), 1);
    m.tick(t0 + ms(20));
    assert!(m.snapshot(&spec.id).unwrap().transport.loop_playing);
    assert!(pos_ms(&m, &spec.id) >= 2500);
    drop(e);
}

#[test]
fn loop_with_count_in_plays_it_before_the_first_pass_and_every_repeat() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Select(1), t0).unwrap();
    m.loop_action(&spec.id, LoopAction::Flags { repeat: true, count_in: true }, t0).unwrap();
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.counting_in && snap.transport.loop_playing);
    // 90 BPM: one beat is 14700 frames
    let l = snap.loops[1].clone();
    let start = (l.start_ms * RATE as f64 / 1000.0).round() as usize;
    let end = (l.end_ms.unwrap() * RATE as f64 / 1000.0).round() as usize;
    let first = 44100;
    let total = first + (end - start) + 14700 * 3 + 500;
    let got = b.pull(total);
    let mut want = vec![0.0f32; total * 2];
    for (f, a) in [(0usize, true), (14700, false), (29400, false)] {
        add_click(&mut want, f, a);
    }
    let pass = expected(&pcm, start, end - start);
    let n = fade_frames();
    for k in 0..end - start {
        let mut g = if k < n { k as f32 / n as f32 } else { 1.0 };
        if end - start - k <= n {
            g *= (end - start - k) as f32 / n as f32;
        }
        want[(first + k) * 2] = pass[2 * k] * g;
        want[(first + k) * 2 + 1] = pass[2 * k + 1] * g;
    }
    let again = first + (end - start);
    for (f, a) in [(0usize, true), (14700, false), (29400, false)] {
        add_click(&mut want, again + f, a);
    }
    let src = expected(&pcm, start, 500);
    for (k, x) in src.iter().enumerate() {
        want[(again + 44100) * 2 + k] = *x;
    }
    fade_in(&mut want, again + 44100);
    close(&got[..(again + 44100 + 500) * 2], &want[..(again + 44100 + 500) * 2]);
    assert_eq!(wraps(&m), 1);
}

#[test]
fn loop_on_off_stops_at_the_end_and_returns_to_the_start() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Flags { repeat: false, count_in: false }, t0).unwrap();
    m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    let len = LOOP1_END - LOOP1_START;
    let got = b.pull(len + 5000);
    let mut want = expected(&pcm, LOOP1_START, len);
    let n = fade_frames();
    fade_in(&mut want, 0);
    for k in 0..n {
        for ch in 0..2 {
            want[2 * (len - n + k) + ch] *= (n - k) as f32 / n as f32;
        }
    }
    want.resize((len + 5000) * 2, 0.0);
    close(&got, &want);
    assert_eq!(wraps(&m), 0);
    m.tick(t0 + ms(20));
    let s = m.snapshot(&spec.id).unwrap();
    assert!(!s.transport.loop_playing && !s.transport.playing);
    assert_eq!(s.transport.position_ms, 2500);
    // a later main Play is a plain play from the loop start, still inside a non-repeating loop
    m.tick(t0 + ms(600));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 2500);
}

#[test]
fn loop_stop_returns_to_the_start() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    b.pull(RATE);
    assert!(pos_ms(&m, &spec.id) > 2500);
    let s = m.loop_action(&spec.id, LoopAction::Stop, t0).unwrap();
    assert!(!s.transport.playing && !s.transport.loop_playing);
    assert_eq!(s.transport.position_ms, 2500);
    assert_eq!(peak(&b.pull(1000)), 0.0);
    assert_eq!(pos_ms(&m, &spec.id), 2500);
}

#[test]
fn main_play_inside_a_repeating_loop_wraps() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.seek(&spec.id, 5000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let from = 5000 * RATE / 1000;
    let total = (LOOP1_END - from) + 2000;
    let got = b.pull(total);
    close(&got, &looped(&pcm, from, LOOP1_START, LOOP1_END, total));
    assert_eq!(wraps(&m), 1);
    assert!(!m.snapshot(&spec.id).unwrap().transport.loop_playing);
}

#[test]
fn main_play_from_before_the_loop_passes_through_it() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.seek(&spec.id, 0, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(LOOP1_END + 1000);
    assert_eq!(wraps(&m), 1);
    let p = pos_ms(&m, &spec.id);
    assert!((2500..2600).contains(&p), "{p}");
}

#[test]
fn incomplete_loop_is_refused_and_never_repeats() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Select(2), t0).unwrap();
    let s = m.snapshot(&spec.id).unwrap();
    assert_eq!(s.current_loop, Some(2));
    assert_eq!(s.transport.position_ms, 18500);
    assert_eq!(m.loop_action(&spec.id, LoopAction::Play, t0).unwrap_err(), "Set the loop end first");
    assert_eq!(
        m.loop_action(&spec.id, LoopAction::Flags { repeat: false, count_in: true }, t0).unwrap_err(),
        "Set the loop end first"
    );
    assert!(!b.is_open(), "a refused loop Play opens no device");
    m.seek(&spec.id, 18500, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(RATE * 2);
    assert_eq!(wraps(&m), 0);
    m.tick(t0 + ms(20));
    let s = m.snapshot(&spec.id).unwrap();
    assert!(!s.transport.playing);
    assert_eq!(s.transport.position_ms, 0);
}

#[test]
fn mark_end_completes_the_loop_and_a_bad_end_leaves_a_message() {
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Select(2), t0).unwrap();
    m.seek(&spec.id, 18000, t0).unwrap();
    let s = m.loop_action(&spec.id, LoopAction::Mark(Mark::End), t0).unwrap();
    assert_eq!(s.loop_message.as_deref(), Some("The end must be after the start"));
    assert!(s.loops[2].end.is_none());
    m.seek(&spec.id, 19500, t0).unwrap();
    let s = m.loop_action(&spec.id, LoopAction::Mark(Mark::End), t0).unwrap();
    assert!(s.loop_message.is_none());
    assert_eq!(s.loops[2].end_ms, Some(19500.0));
    assert!(m.loop_action(&spec.id, LoopAction::Play, t0).is_ok());
}

#[test]
fn prev_next_jump_at_once_and_keep_playing() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(100);
    let s = m.loop_action(&spec.id, LoopAction::Select(1), t0).unwrap();
    assert_eq!(s.current_loop, Some(1));
    assert!(s.transport.playing && !s.transport.resume_pending);
    assert!(peak(&b.pull(1000)) > 0.0);
    let s = m.loop_action(&spec.id, LoopAction::Select(-5), t0).unwrap();
    assert_eq!(s.current_loop, Some(0));
    assert_eq!(s.transport.position_ms, 2500);
}

#[test]
fn tempo_change_keeps_loops_in_time_and_resets_speed() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    let before = m.snapshot(&spec.id).unwrap();
    m.seek(&spec.id, 3000, t0).unwrap();
    let s = m.set_bpm(&spec.id, 100.0, t0).unwrap();
    assert_eq!(s.speed, 1.0);
    for (a, b) in before.loops.iter().zip(&s.loops) {
        assert!((a.start_ms - b.start_ms).abs() <= 21.0, "{} vs {}", a.start_ms, b.start_ms);
        assert_eq!(a.end_ms.is_some(), b.end_ms.is_some());
        if let (Some(x), Some(y)) = (a.end_ms, b.end_ms) {
            assert!((x - y).abs() <= 21.0);
        }
    }
    assert!(s.loops[2].end.is_none());
    assert!(s.tempo.segments.iter().any(|g| g.bpm == 100.0));
    m.tick(t0 + ms(500));
    assert!(disk(&spec).unwrap()["tempo"]["segments"].to_string().contains("100"));

    // the new grid is live: 100 BPM beats are 13230 frames apart from 3000 ms
    m.set_metronome(&spec.id, true, false, t0).unwrap();
    m.seek(&spec.id, 3000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let got = b.pull(20000);
    let mut want = expected(&pcm, 3000 * RATE / 1000, 20000);
    fade_in(&mut want, 0);
    add_click(&mut want, 0, false);
    add_click(&mut want, 13230, false);
    // bar position decides the accents; only compare where no click sits
    let beat = 13230usize;
    let quiet = |v: &[f32]| v[2 * 2000..2 * beat].to_vec();
    close(&quiet(&got), &quiet(&want));
    assert!(peak(&got[2 * beat..2 * beat + 200]) > 0.0);
}

#[test]
fn removing_the_tempo_clears_the_grid_and_count_in() {
    let (_t, spec) = spec_of(WITH_TEMPO);
    let p = spec.dir.join("player.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["tempo"]["segments"] = serde_json::json!([{"start_ms": 500.0, "bpm": 120.0}]);
    v["metronome"] = serde_json::json!({"on": true, "count_in": true});
    std::fs::write(&p, v.to_string()).unwrap();
    let (mut m, b, _e) = manager();
    m.open(&spec).unwrap();
    let t0 = Instant::now();
    let s = m.set_bpm(&spec.id, 0.0, t0).unwrap();
    assert_eq!(s.mode, "time");
    assert!(matches!(s.loops[0].start, Boundary::Ms { .. }));
    assert!(s.loops[2].end.is_none());
    m.seek(&spec.id, 0, t0).unwrap();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(!st.counting_in);
    let got = b.pull(RATE);
    assert!(peak(&got[2 * 3000..2 * 3200]) > 0.0, "audio from the start, no count-in");
    assert_eq!(m.set_metronome(&spec.id, true, true, t0).unwrap_err(), "Enter a BPM first");
}

#[test]
fn metronome_and_speed_are_refused_without_a_tempo() {
    let (_t, spec, mut m, _b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    assert_eq!(m.set_metronome(&spec.id, true, false, t0).unwrap_err(), "Enter a BPM first");
    assert!(!m.snapshot(&spec.id).unwrap().metronome.on);
    assert_eq!(m.speed_action(&spec.id).unwrap_err(), "Enter a BPM first");
    let (_t, spec, mut m, _b, _e) = opened(WITH_TEMPO);
    assert_eq!(m.speed_action(&spec.id).unwrap_err(), "Speed change is not available yet");
}

#[test]
fn engine_callback_never_allocates() {
    let (_t, spec) = spec_of(WITH_TEMPO);
    let pcm = Arc::new(decoded(&spec, GUITAR));
    let shared = Arc::new(Shared::default());
    let (tx, _rx) = sync_channel(8);
    let mut en = Engine::new(pcm.clone(), 0, shared.clone()).with_trash(tx);
    let grid: Vec<Beat> = (0..40).map(|k| (k * 11025, k % 4 == 1)).collect();
    let plan = count_in_plan(120.0, 22050);
    let spec_loop = LoopSpec { start: 20000, end: 40000, repeat: true, count_in: Some(plan) };
    let g2 = Arc::new(grid.clone());
    let mut out = vec![0.0f32; 2 * 1024];
    let empty = Arc::new(Vec::new());
    COUNTING.with(|c| c.set(true));
    ALLOCS.with(|a| a.set(0));
    en.apply(Cmd::SetGrid(empty));
    en.apply(Cmd::SetGrid(g2));
    en.apply(Cmd::SetMetronome(true));
    en.apply(Cmd::SetLoop(Some(spec_loop)));
    en.apply(Cmd::Play { count_in: Some(plan) });
    for _ in 0..120 {
        en.render(&mut out);
    }
    en.apply(Cmd::Seek(100));
    en.apply(Cmd::Resume);
    en.apply(Cmd::PlayLoop { count_in: None });
    for _ in 0..60 {
        en.render(&mut out);
    }
    en.apply(Cmd::Pause);
    let n = ALLOCS.with(Cell::get);
    COUNTING.with(|c| c.set(false));
    assert_eq!(n, 0, "the callback allocated");
    assert!(shared.wraps.load(Ordering::Acquire) > 0);
}
