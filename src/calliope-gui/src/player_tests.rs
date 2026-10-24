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
    if title == WITH_TEMPO {
        // the fixture is a schema-1 file with the master Off; the tests hear the clicks, so the
        // master (migrated into the section flags on reading) is On
        let p = root.join("tracks/0199c0a0-0000-7000-8000-000000000304/player.json");
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        v["metronome"] = serde_json::json!({"on": true, "count_in": false});
        std::fs::write(&p, v.to_string()).unwrap();
    }
    let t = Repository::new(&root)
        .scan()
        .tracks
        .into_iter()
        .find(|t| t.title == title)
        .unwrap();
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
    (
        PlayerManager::new(Arc::new(backend.clone()), sink),
        backend,
        events,
    )
}

fn opened(
    title: &str,
) -> (
    tempfile::TempDir,
    OpenSpec,
    PlayerManager,
    ManualBackend,
    Events,
) {
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
    player_source::decode(
        &spec.dir.join(f),
        None,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap()
}

/// Interleaved stereo f32 of `frames` frames from `start` (mono copied to both channels).
fn expected(pcm: &Pcm, start: usize, frames: usize) -> Vec<f32> {
    let ch = pcm.channels as usize;
    let mut v = Vec::new();
    for i in start..start + frames {
        let l = f32::from(pcm.samples[i * ch]) / 32768.0;
        let r = if ch >= 2 {
            f32::from(pcm.samples[i * ch + 1]) / 32768.0
        } else {
            l
        };
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
    assert!(m
        .play("x", Instant::now())
        .unwrap_err()
        .contains("no player session for x"));
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
    assert!(got[..n * 2]
        .iter()
        .zip(&want)
        .all(|(g, w)| g.abs() <= w.abs() + 1e-6));
    assert_eq!(
        m.snapshot(&spec.id).unwrap().transport.position_ms,
        2000 * 1000 / RATE as u64
    );
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
    let snap = m
        .select_source(&spec.id, "stem:guitar", t0 + ms(300))
        .unwrap();
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
    assert_eq!(
        m.select_source(&spec.id, "stem:guitar", t0).unwrap().source,
        "stem:guitar"
    );
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
    assert!(e
        .lock()
        .unwrap()
        .iter()
        .any(|x| matches!(x, PlayerEvent::AudioError { message, .. } if message == "gone")));
}

#[test]
fn transport_events_are_throttled_while_playing() {
    let (_t, spec, mut m, b, e) = opened(PRACTICE);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    let count = |e: &Events| {
        e.lock()
            .unwrap()
            .iter()
            .filter(|x| matches!(x, PlayerEvent::Transport { .. }))
            .count()
    };
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
    let pcm = Arc::new(Pcm {
        rate: 1000,
        channels: 1,
        frames: 10,
        samples: vec![16384; 10],
    });
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

fn add_click(v: &mut [f32], frame: usize) {
    let c = Click::new(RATE as u32);
    for (k, x) in c.samples().iter().enumerate() {
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
        let mut g = if since < n {
            since as f32 / n as f32
        } else {
            1.0
        };
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

fn with_tempo() -> (
    tempfile::TempDir,
    OpenSpec,
    PlayerManager,
    ManualBackend,
    Events,
    Pcm,
) {
    let (t, spec, m, b, e) = opened(WITH_TEMPO);
    let pcm = decoded(&spec, GUITAR);
    (t, spec, m, b, e, pcm)
}

fn wraps(m: &PlayerManager) -> u64 {
    m.session
        .as_ref()
        .unwrap()
        .shared
        .wraps
        .load(Ordering::Acquire)
}

fn pos_ms(m: &PlayerManager, id: &str) -> u64 {
    m.snapshot(id).unwrap().transport.position_ms
}

/// The Click and Count In of the section `index` (starting at `at` ms).
fn flags(
    m: &mut PlayerManager,
    id: &str,
    index: usize,
    at: f64,
    metronome: bool,
    count_in: bool,
    t: Instant,
) {
    m.edit_sections(
        id,
        SectionEdit::Flags {
            index,
            at,
            metronome,
            count_in,
        },
        t,
    )
    .unwrap();
}

/// Click off in both sections of the fixture (the loop tests compare the plain audio).
fn quiet(m: &mut PlayerManager, id: &str, t: Instant) {
    flags(m, id, 0, 500.0, false, false, t);
    flags(m, id, 1, 10500.0, false, false, t);
}

#[test]
fn metronome_clicks_at_exact_frames_in_one_tone() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    m.seek(&spec.id, 0, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let total = RATE * 2600 / 1000;
    let got = b.pull(total);
    let mut want = expected(&pcm, 0, total);
    fade_in(&mut want, 0);
    // beats every 500 ms from 0 (beat 4 of bar 0); the bar starts at 500 and 2500
    for ms in [0usize, 500, 1000, 1500, 2000, 2500] {
        add_click(&mut want, ms * RATE / 1000);
    }
    close(&got, &want);
}

#[test]
fn click_off_in_every_section_leaves_the_source_untouched() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    flags(&mut m, &spec.id, 0, 500.0, false, false, t0);
    flags(&mut m, &spec.id, 1, 10500.0, false, false, t0);
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
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
    m.seek(&spec.id, 0, t0).unwrap();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(st.counting_in && st.playing);
    let got = b.pull(33075 + 2000);
    let mut want = vec![0.0f32; (33075 + 2000) * 2];
    for f in [0usize, 11025, 22050] {
        add_click(&mut want, f);
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
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
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
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
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
    quiet(&mut m, &spec.id, t0);
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.loop_playing && !snap.transport.counting_in);
    assert_eq!(snap.transport.position_ms, 2500);
    let len = LOOP1_END - LOOP1_START;
    let total = len + 3000;
    let got = b.pull(total);
    close(
        &got,
        &looped(&pcm, LOOP1_START, LOOP1_START, LOOP1_END, total),
    );
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
    quiet(&mut m, &spec.id, t0);
    m.loop_action(&spec.id, LoopAction::Select(1), t0).unwrap();
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: true,
            count_in: true,
        },
        t0,
    )
    .unwrap();
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
    for f in [0usize, 14700, 29400] {
        add_click(&mut want, f);
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
    for f in [0usize, 14700, 29400] {
        add_click(&mut want, again + f);
    }
    let src = expected(&pcm, start, 500);
    for (k, x) in src.iter().enumerate() {
        want[(again + 44100) * 2 + k] = *x;
    }
    fade_in(&mut want, again + 44100);
    close(
        &got[..(again + 44100 + 500) * 2],
        &want[..(again + 44100 + 500) * 2],
    );
    assert_eq!(wraps(&m), 1);
}

#[test]
fn loop_on_off_stops_at_the_end_and_returns_to_the_start() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    quiet(&mut m, &spec.id, t0);
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: false,
            count_in: false,
        },
        t0,
    )
    .unwrap();
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
    quiet(&mut m, &spec.id, t0);
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
    assert_eq!(
        m.loop_action(&spec.id, LoopAction::Play, t0).unwrap_err(),
        "Set the loop end first"
    );
    assert_eq!(
        m.loop_action(
            &spec.id,
            LoopAction::Flags {
                repeat: false,
                count_in: true
            },
            t0
        )
        .unwrap_err(),
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
    let s = m
        .loop_action(&spec.id, LoopAction::Mark(Mark::End), t0)
        .unwrap();
    assert_eq!(
        s.loop_message.as_deref(),
        Some("The end must be after the start")
    );
    assert!(s.loops[2].end.is_none());
    m.seek(&spec.id, 19500, t0).unwrap();
    let s = m
        .loop_action(&spec.id, LoopAction::Mark(Mark::End), t0)
        .unwrap();
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
fn tempo_change_keeps_loops_in_time_and_keeps_speed() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    let before = m.snapshot(&spec.id).unwrap();
    m.seek(&spec.id, 3000, t0).unwrap();
    m.speed(&spec.id, SpeedAction::Set(80), t0).unwrap();
    let s = m.set_bpm(&spec.id, 100.0, t0).unwrap();
    assert_eq!(s.speed_pct, 80);
    for (a, b) in before.loops.iter().zip(&s.loops) {
        assert!(
            (a.start_ms - b.start_ms).abs() <= 21.0,
            "{} vs {}",
            a.start_ms,
            b.start_ms
        );
        assert_eq!(a.end_ms.is_some(), b.end_ms.is_some());
        if let (Some(x), Some(y)) = (a.end_ms, b.end_ms) {
            assert!((x - y).abs() <= 21.0);
        }
    }
    assert!(s.loops[2].end.is_none());
    assert!(s.tempo.segments.iter().any(|g| g.bpm == 100.0));
    m.tick(t0 + ms(500));
    assert!(disk(&spec).unwrap()["tempo"]["segments"]
        .to_string()
        .contains("100"));

    m.speed(&spec.id, SpeedAction::Reset, t0).unwrap(); // the audio below is compared at 100 %
                                                        // the new grid is live: 100 BPM beats are 13230 frames apart from 3000 ms
    m.seek(&spec.id, 3000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let got = b.pull(20000);
    let mut want = expected(&pcm, 3000 * RATE / 1000, 20000);
    fade_in(&mut want, 0);
    add_click(&mut want, 0);
    add_click(&mut want, 13230);
    close(&got, &want);
}

fn starts(s: &PlayerSnapshot) -> Vec<f64> {
    s.tempo.segments.iter().map(|g| g.start_ms).collect()
}

#[test]
fn section_edits_reach_the_engine_loops_and_disk() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    let before = m.snapshot(&spec.id).unwrap();
    let s = m
        .edit_sections(
            &spec.id,
            SectionEdit::Add {
                position: 3000.0,
                bpm: 100.0,
            },
            t0,
        )
        .unwrap();
    assert_eq!(starts(&s), vec![500.0, 3000.0, 10500.0]);
    assert!(s.tempo.segments[1].on_beat);
    assert_eq!(s.speed_pct, 100);
    for (a, b) in before.loops.iter().zip(&s.loops) {
        assert!(
            (a.start_ms - b.start_ms).abs() <= 21.0,
            "{} vs {}",
            a.start_ms,
            b.start_ms
        );
    }
    let fixture = disk(&spec).unwrap();
    m.tick(t0 + ms(499));
    assert_eq!(disk(&spec).unwrap(), fixture, "not before the debounce");
    m.tick(t0 + ms(500));
    assert!(disk(&spec).unwrap()["tempo"]["segments"]
        .to_string()
        .contains("on_beat"));

    // the new grid is live: 100 BPM beats from the placed change
    m.seek(&spec.id, 3000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let got = b.pull(20000);
    let mut want = expected(&pcm, 3000 * RATE / 1000, 20000);
    fade_in(&mut want, 0);
    add_click(&mut want, 0);
    add_click(&mut want, 13230);
    close(&got, &want);

    // the other edits: each marks dirty (written 500 ms later, not before) and keeps the loops in time
    let edits = [
        (
            SectionEdit::Bpm {
                index: 1,
                at: 3000.0,
                bpm: 110.0,
            },
            1,
            110.0,
        ),
        (
            SectionEdit::Scale {
                index: 1,
                at: 3000.0,
                factor: Scale::Halve,
            },
            1,
            55.0,
        ),
        (
            SectionEdit::Move {
                index: 1,
                at: 3000.0,
                steps: 384,
            },
            1,
            55.0,
        ),
        (
            SectionEdit::Remove {
                index: 1,
                at: 3500.0,
            },
            0,
            120.0,
        ),
    ];
    for (k, (e, i, want_bpm)) in edits.into_iter().enumerate() {
        let t = t0 + ms(1000 * (k as u64 + 1));
        let on_disk = disk(&spec).unwrap()["tempo"]["segments"].clone();
        let s = m.edit_sections(&spec.id, e.clone(), t).unwrap();
        assert_eq!(s.tempo.segments[i].bpm, want_bpm, "{e:?}");
        for (a, b) in before.loops.iter().zip(&s.loops) {
            assert!((a.start_ms - b.start_ms).abs() <= 21.0);
        }
        m.tick(t + ms(499));
        assert_eq!(disk(&spec).unwrap()["tempo"]["segments"], on_disk, "{e:?}");
        m.tick(t + ms(500));
        assert_ne!(disk(&spec).unwrap()["tempo"]["segments"], on_disk, "{e:?}");
    }
    assert_eq!(starts(&m.snapshot(&spec.id).unwrap()), vec![500.0, 10500.0]);
}

#[test]
fn section_guard_accepts_starts_that_went_through_json() {
    use crate::player_ipc::TempoAction;
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.edit_sections(
        &spec.id,
        SectionEdit::Bpm {
            index: 0,
            at: 500.0,
            bpm: 93.0,
        },
        t0,
    )
    .unwrap();
    for n in 0..60 {
        let pos = 1500.0 + n as f64 * 97.3;
        let s = m
            .edit_sections(
                &spec.id,
                SectionEdit::Add {
                    position: pos,
                    bpm: 100.0,
                },
                t0,
            )
            .unwrap();
        let seg = s
            .tempo
            .segments
            .iter()
            .find(|g| (g.start_ms - pos).abs() < 40.0)
            .unwrap();
        let text = serde_json::to_string(seg).unwrap();
        let back: crate::tempo_map::Segment = serde_json::from_str(&text).unwrap();
        let index = s
            .tempo
            .segments
            .iter()
            .position(|g| g.start_ms == seg.start_ms)
            .unwrap();
        let json = format!(
            r#"{{"kind":"section-bpm","index":{index},"at":{},"bpm":110}}"#,
            back.start_ms
        );
        let TempoAction::SectionBpm { index, at, bpm } = serde_json::from_str(&json).unwrap()
        else {
            panic!()
        };
        assert_eq!(at, seg.start_ms, "{json}");
        let s = m
            .edit_sections(&spec.id, SectionEdit::Bpm { index, at, bpm }, t0)
            .unwrap();
        // a one-ulp difference is still the same section, a real difference is not
        let at1 = s.tempo.segments[index].start_ms;
        let e = SectionEdit::Bpm {
            index,
            at: f64::from_bits(at1.to_bits() + 1),
            bpm: 100.0,
        };
        assert!(
            m.edit_sections(&spec.id, e, t0).is_ok(),
            "one ulp off at index {index}"
        );
        let e = SectionEdit::Bpm {
            index,
            at: at1 + 1e-3,
            bpm: 100.0,
        };
        assert_eq!(
            m.edit_sections(&spec.id, e, t0).unwrap_err(),
            "The tempo sections changed; try again"
        );
        m.edit_sections(&spec.id, SectionEdit::Remove { index, at }, t0)
            .unwrap();
    }
}

#[test]
fn stale_or_refused_section_edits_change_nothing() {
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    let before = m.snapshot(&spec.id).unwrap();
    let fixture = disk(&spec).unwrap();
    let stale = "The tempo sections changed; try again";
    let e = SectionEdit::Remove {
        index: 1,
        at: 10499.0,
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), stale);
    let e = SectionEdit::Move {
        index: 7,
        at: 0.0,
        steps: 1,
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), stale);
    let e = SectionEdit::Bpm {
        index: 1,
        at: 10500.0,
        bpm: 0.0,
    };
    assert_eq!(
        m.edit_sections(&spec.id, e, t0).unwrap_err(),
        "Enter a BPM between 20 and 300"
    );
    let e = SectionEdit::Remove {
        index: 0,
        at: 500.0,
    };
    assert_eq!(
        m.edit_sections(&spec.id, e, t0).unwrap_err(),
        "The first section has no tempo change to remove"
    );
    let e = SectionEdit::Add {
        position: 100.0,
        bpm: 100.0,
    };
    assert!(m.edit_sections(&spec.id, e, t0).is_err());
    let after = m.snapshot(&spec.id).unwrap();
    assert_eq!(starts(&after), starts(&before));
    assert_eq!(after.loops.len(), before.loops.len());
    for (a, b) in before.loops.iter().zip(&after.loops) {
        assert_eq!((a.start_ms, a.end_ms), (b.start_ms, b.end_ms));
    }
    m.tick(t0 + ms(5000));
    assert_eq!(disk(&spec).unwrap(), fixture, "refusals write nothing");
}

const STALE: &str = "The tempo sections changed; try again";

#[test]
fn section_metronome_toggle_is_heard_from_the_next_beat_and_saved() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    let e = SectionEdit::Flags {
        index: 1,
        at: 10500.0,
        metronome: false,
        count_in: false,
    };
    let s = m.edit_sections(&spec.id, e, t0).unwrap();
    assert!(!s.tempo.segments[1].meta.metronome);
    let on_disk = disk(&spec).unwrap()["tempo"]["segments"].clone();
    m.tick(t0 + ms(499));
    assert_eq!(
        disk(&spec).unwrap()["tempo"]["segments"],
        on_disk,
        "not before the debounce"
    );
    m.tick(t0 + ms(500));
    assert_eq!(
        disk(&spec).unwrap()["tempo"]["segments"][1]["metronome"],
        false
    );

    // from 10000: 120 BPM clicks at 10000 only; the 90 BPM section is silent
    let start = 10000 * RATE / 1000;
    m.seek(&spec.id, 10000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let first = RATE * 3 / 2;
    let got = b.pull(first);
    let mut want = expected(&pcm, start, first);
    fade_in(&mut want, 0);
    add_click(&mut want, 0);
    close(&got, &want);

    // on again at 11500 ms: the next 90 BPM beat is at 11833.33 ms
    let e = SectionEdit::Flags {
        index: 1,
        at: 10500.0,
        metronome: true,
        count_in: false,
    };
    m.edit_sections(&spec.id, e, t0 + ms(1000)).unwrap();
    let next = ((35500.0f64 / 3.0) * RATE as f64 / 1000.0).round() as usize - (start + first);
    let got = b.pull(RATE);
    let mut want = expected(&pcm, start + first, RATE);
    add_click(&mut want, next);
    close(&got, &want);

    let e = SectionEdit::Name {
        index: 1,
        at: 10500.0,
        name: " Verse ".into(),
    };
    let t1 = t0 + ms(2000);
    let s = m.edit_sections(&spec.id, e, t1).unwrap();
    assert_eq!(s.tempo.segments[1].meta.name, "Verse");
    m.tick(t1 + ms(499));
    assert_ne!(
        disk(&spec).unwrap()["tempo"]["segments"][1]["name"],
        "Verse"
    );
    m.tick(t1 + ms(500));
    assert_eq!(
        disk(&spec).unwrap()["tempo"]["segments"][1]["name"],
        "Verse"
    );
}

#[test]
fn stale_section_flags_name_and_jump_change_nothing() {
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    let before = m.snapshot(&spec.id).unwrap().tempo;
    let fixture = disk(&spec).unwrap();
    let e = SectionEdit::Flags {
        index: 1,
        at: 10499.0,
        metronome: false,
        count_in: true,
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), STALE);
    let e = SectionEdit::Name {
        index: 5,
        at: 0.0,
        name: "x".into(),
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), STALE);
    assert_eq!(
        m.jump_section(&spec.id, 1, 10499.0, Edge::Start, t0)
            .unwrap_err(),
        STALE
    );
    let e = SectionEdit::Name {
        index: 1,
        at: 10500.0,
        name: "a".repeat(41),
    };
    assert_eq!(
        m.edit_sections(&spec.id, e, t0).unwrap_err(),
        "Section names can have at most 40 characters"
    );
    assert_eq!(m.snapshot(&spec.id).unwrap().tempo, before);
    m.tick(t0 + ms(5000));
    assert_eq!(disk(&spec).unwrap(), fixture);
}

fn grid_frames(m: &PlayerManager) -> Vec<u64> {
    m.session
        .as_ref()
        .unwrap()
        .grid()
        .iter()
        .map(|b| b.0)
        .collect()
}

#[test]
fn a_repeating_loop_ending_at_a_count_in_section_skips_that_count_in() {
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    // loop-1 repeats; make it end at the start of the 90 BPM section
    m.loop_action(
        &spec.id,
        LoopAction::Set(Mark::End, Boundary::Tick { tick: 304 }),
        t0,
    )
    .unwrap();
    let e = SectionEdit::Flags {
        index: 1,
        at: 10500.0,
        metronome: true,
        count_in: true,
    };
    m.edit_sections(&spec.id, e, t0).unwrap();
    // 90 BPM: count-in clicks at 8500, 9166.67 and 9833.33 ms
    let count_in = ((29500.0f64 / 3.0) * RATE as f64 / 1000.0).round() as u64;
    assert!(!grid_frames(&m).contains(&count_in), "loop on: no count-in");
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: false,
            count_in: false,
        },
        t0,
    )
    .unwrap();
    assert!(grid_frames(&m).contains(&count_in), "loop off: back");
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: true,
            count_in: false,
        },
        t0,
    )
    .unwrap();
    assert!(!grid_frames(&m).contains(&count_in));
}

#[test]
fn click_off_silences_the_section_clicks_and_count_in_on_gives_the_play_pre_roll() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    flags(&mut m, &spec.id, 0, 500.0, false, false, t0);
    flags(&mut m, &spec.id, 1, 10500.0, false, false, t0);
    let start = 8000 * RATE / 1000;
    m.seek(&spec.id, 8000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    let total = RATE * 3;
    let mut want = expected(&pcm, start, total);
    fade_in(&mut want, 0);
    close(&b.pull(total), &want);

    m.pause(&spec.id, t0).unwrap();
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
    m.seek(&spec.id, 8000, t0).unwrap();
    assert!(m.play(&spec.id, t0).unwrap().counting_in);
    assert!(peak(&b.pull(5000)) > 0.0, "the pre-roll clicks");
}

#[test]
fn play_counts_in_per_section_at_that_sections_tempo() {
    let (_t, spec, mut m, b, _e, pcm) = with_tempo();
    let t0 = Instant::now();
    // Click off everywhere; Count In only on the 90 BPM section
    flags(&mut m, &spec.id, 0, 500.0, false, false, t0);
    flags(&mut m, &spec.id, 1, 10500.0, false, true, t0);
    // paused inside it, between beats: three clicks one 90 BPM beat (14700 frames) apart, then the audio from
    // there 500 ms (11025 frames) before the 4th beat, so that the next beat (12500 ms) falls on it
    m.seek(&spec.id, 12000, t0).unwrap();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(st.counting_in && st.playing);
    let pre = 14700 * 3 - 11025;
    let got = b.pull(pre + 2000);
    let mut want = vec![0.0f32; (pre + 2000) * 2];
    for f in [0usize, 14700, 29400] {
        add_click(&mut want, f);
    }
    let from = 12000 * RATE / 1000;
    for (k, x) in expected(&pcm, from, 2000).iter().enumerate() {
        want[pre * 2 + k] = *x;
    }
    fade_in(&mut want, pre);
    close(&got, &want);

    // inside the section without it: the audio at once
    m.pause(&spec.id, t0).unwrap();
    m.seek(&spec.id, 3000, t0).unwrap();
    assert!(!m.play(&spec.id, t0).unwrap().counting_in);
    let got = b.pull(2000);
    let first = got
        .chunks(2)
        .position(|f| f[0] != 0.0 || f[1] != 0.0)
        .unwrap();
    assert!(first <= fade_frames() + 1, "first sound at frame {first}");
    // a play that is already running never counts in again
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
    assert!(!m.play(&spec.id, t0).unwrap().counting_in);
}

#[test]
fn loop_play_counts_in_only_with_the_loops_own_count_in() {
    let (_t, spec, mut m, _b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    flags(&mut m, &spec.id, 0, 500.0, true, true, t0); // loop-1 lies in this section
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.loop_playing && !snap.transport.counting_in);
    m.pause(&spec.id, t0).unwrap();
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: true,
            count_in: true,
        },
        t0,
    )
    .unwrap();
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.counting_in);
}

#[test]
fn plus_while_playing_makes_a_complete_loop_that_wraps_at_the_next_section() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.seek(&spec.id, 8000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(100);
    let snap = m.loop_action(&spec.id, LoopAction::Add, t0).unwrap();
    let l = &snap.loops[snap.current_loop.unwrap()];
    assert!(l.repeat && l.end.is_some() && !l.count_in);
    assert!((l.start_ms - 8000.0).abs() < 60.0, "{}", l.start_ms);
    assert!((l.end_ms.unwrap() - 10500.0).abs() < 22.0, "{:?}", l.end_ms);
    let w0 = wraps(&m);
    b.pull(RATE * 3);
    assert_eq!(wraps(&m), w0 + 1);
    let p = pos_ms(&m, &spec.id);
    assert!((8000..10500).contains(&p), "{p}");
}

#[test]
fn jump_section_is_instant_while_playing_and_moves_the_playhead_while_paused() {
    let (_t, spec, mut m, b, events, pcm) = with_tempo();
    let t0 = Instant::now();
    flags(&mut m, &spec.id, 0, 500.0, false, false, t0);
    flags(&mut m, &spec.id, 1, 10500.0, false, false, t0);
    m.seek(&spec.id, 1000, t0).unwrap();
    m.jump_section(&spec.id, 1, 10500.0, Edge::Start, t0)
        .unwrap();
    assert_eq!(pos_ms(&m, &spec.id), 10500);
    assert_eq!(peak(&b.pull(500)), 0.0, "paused: nothing sounds");
    m.tick(t0 + ms(500));
    assert_eq!(disk(&spec).unwrap()["position_ms"], 10500);

    m.play(&spec.id, t0).unwrap();
    b.pull(100);
    events.lock().unwrap().clear();
    let st = m.snapshot(&spec.id).unwrap();
    let end_ms = st.duration_ms as f64;
    // End of section 0 (the last section's End would leave nothing to play)
    let target = m
        .session
        .as_ref()
        .unwrap()
        .state
        .tempo
        .section_jump_ms(0, Edge::End, end_ms)
        .unwrap();
    assert_eq!(target, 10500.0);
    let s = m.jump_section(&spec.id, 0, 500.0, Edge::End, t0).unwrap();
    assert!(s.transport.playing && !s.transport.resume_pending);
    // the last frame before the section end
    let f = (target * RATE as f64 / 1000.0 - 1e-6).ceil() as usize - 1;
    assert_eq!(m.session.as_ref().unwrap().pos() as usize, f);
    let got = b.pull(2000);
    let mut want = expected(&pcm, f, 2000);
    fade_in(&mut want, 0);
    close(&got, &want);
    assert!(
        events.lock().unwrap().iter().all(|e| !matches!(e, PlayerEvent::Transport { state, .. } if !state.playing || state.resume_pending)),
        "no pause or seek event"
    );
}

/// A detected map has fractional section starts; the jump lands on the next frame and the transport must name that section.
#[test]
fn start_jump_on_a_fractional_section_start_stays_in_that_section() {
    const AT: f64 = 10500.01;
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    m.session.as_mut().unwrap().state.tempo.segments[1].start_ms = AT;
    for playing in [false, true] {
        if playing {
            m.play(&spec.id, t0).unwrap();
            b.pull(100);
        }
        m.seek(&spec.id, 1000, t0).unwrap();
        let mut first = None;
        for _ in 0..3 {
            let s = m.jump_section(&spec.id, 1, AT, Edge::Start, t0).unwrap();
            assert_eq!(s.transport.tempo_index, Some(1), "playing={playing}");
            assert_eq!(
                s.transport.bpm,
                m.session.as_ref().unwrap().state.tempo.segments[1].bpm
            );
            let pos = m.session.as_ref().unwrap().pos();
            assert_eq!(*first.get_or_insert(pos), pos);
        }
        assert_eq!(m.snapshot(&spec.id).unwrap().transport.tempo_index, Some(1));
    }
}

#[test]
fn jump_section_cancels_a_pre_roll_count_in() {
    let (_t, spec, mut m, b, _e, _pcm) = with_tempo();
    let t0 = Instant::now();
    flags(&mut m, &spec.id, 0, 500.0, false, true, t0);
    m.seek(&spec.id, 1000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(15000);
    assert!(m.snapshot(&spec.id).unwrap().transport.counting_in);
    m.jump_section(&spec.id, 1, 10500.0, Edge::Start, t0)
        .unwrap();
    assert!(peak(&b.pull(2000)) > 0.0);
    let t = m.snapshot(&spec.id).unwrap().transport;
    assert!(!t.counting_in && t.playing);
    assert_eq!(t.position_ms, 10500 + 2000 * 1000 / RATE as u64);
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
    assert!(
        peak(&got[2 * 3000..2 * 3200]) > 0.0,
        "audio from the start, no count-in"
    );
    let e = SectionEdit::Flags {
        index: 0,
        at: 500.0,
        metronome: true,
        count_in: true,
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), STALE);
}

#[test]
fn section_flags_are_refused_without_a_tempo() {
    let (_t, spec, mut m, _b, _e) = opened(PRACTICE);
    let t0 = Instant::now();
    let e = SectionEdit::Flags {
        index: 0,
        at: 0.0,
        metronome: true,
        count_in: false,
    };
    assert_eq!(m.edit_sections(&spec.id, e, t0).unwrap_err(), STALE);
    // the speed works without a tempo; there is no BPM to play
    let s = m.speed(&spec.id, SpeedAction::Step(-1), t0).unwrap();
    assert_eq!(
        (s.speed_pct, s.transport.bpm, s.transport.played_bpm),
        (95, 0.0, 0.0)
    );
}

#[test]
fn speed_actions_clamp_refuse_and_save() {
    let (_t, spec, mut m, _b, _e) = opened(WITH_TEMPO);
    let t0 = Instant::now();
    let speed = |m: &mut PlayerManager, a| m.speed(&spec.id, a, t0);
    assert_eq!(speed(&mut m, SpeedAction::Set(80)).unwrap().speed_pct, 80);
    let t = speed(&mut m, SpeedAction::Step(0)).unwrap().transport;
    assert_eq!((t.bpm, t.played_bpm, t.speed_pct), (120.0, 96.0, 80));
    assert_eq!(
        speed(&mut m, SpeedAction::Set(50))
            .unwrap()
            .transport
            .played_bpm,
        60.0
    );
    assert_eq!(
        speed(&mut m, SpeedAction::Set(250)).unwrap_err(),
        crate::speed::SPEED_RANGE
    );
    assert_eq!(m.snapshot(&spec.id).unwrap().speed_pct, 50);
    assert_eq!(speed(&mut m, SpeedAction::Set(25)).unwrap().speed_pct, 25);
    assert_eq!(speed(&mut m, SpeedAction::Step(-1)).unwrap().speed_pct, 25);
    assert_eq!(speed(&mut m, SpeedAction::Set(200)).unwrap().speed_pct, 200);
    assert_eq!(speed(&mut m, SpeedAction::Step(1)).unwrap().speed_pct, 200);
    assert_eq!(speed(&mut m, SpeedAction::Reset).unwrap().speed_pct, 100);
    speed(&mut m, SpeedAction::Set(80)).unwrap();
    m.tick(t0 + ms(500));
    assert_eq!(disk(&spec).unwrap()["speed"], 0.8);
}

#[test]
fn a_training_run_refuses_speed_actions_and_reports_progress() {
    let (_t, spec, mut m, b, _e) = opened(WITH_TEMPO);
    let t0 = Instant::now();
    let t = Training {
        on: true,
        start: 60,
        end: 80,
        step: 10,
        repeats: 2,
    };
    let s = m
        .loop_action(&spec.id, LoopAction::Training(t), t0)
        .unwrap();
    assert_eq!(s.loops[0].training, t);
    assert_eq!(s.transport.training, None);
    assert_eq!(s.loops[1].training, Training::default());
    m.speed(&spec.id, SpeedAction::Set(90), t0).unwrap();
    m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    // before the callback has shown the run, the manager knows it started
    assert_eq!(
        m.speed(&spec.id, SpeedAction::Set(70), t0).unwrap_err(),
        TRAINING_RUNNING
    );
    b.pull(4000);
    m.tick(t0 + ms(20));
    let tr = m.snapshot(&spec.id).unwrap().transport;
    assert_eq!(
        tr.training,
        Some(TrainingProgress {
            loop_index: 0,
            speed_pct: 60,
            repeat: 1,
            of: Some(2)
        })
    );
    assert_eq!(tr.speed_pct, 60);
    for a in [
        SpeedAction::Step(1),
        SpeedAction::Reset,
        SpeedAction::Set(70),
    ] {
        assert_eq!(m.speed(&spec.id, a, t0).unwrap_err(), TRAINING_RUNNING);
    }
    assert_eq!(m.snapshot(&spec.id).unwrap().speed_pct, 90);
    m.pause(&spec.id, t0).unwrap();
    b.pull(4000);
    m.tick(t0 + ms(40));
    let snap = m.snapshot(&spec.id).unwrap();
    assert_eq!(
        (snap.transport.training, snap.transport.speed_pct),
        (None, 90)
    );
    assert!(m.speed(&spec.id, SpeedAction::Reset, t0).is_ok());
}

#[test]
fn loop_play_with_training_counts_in_at_the_first_pass_speed() {
    let (_t, spec, mut m, _b, _e) = opened(WITH_TEMPO);
    let t0 = Instant::now();
    let t = Training {
        on: true,
        start: 60,
        end: 80,
        step: 10,
        repeats: 2,
    };
    m.loop_action(&spec.id, LoopAction::Training(t), t0)
        .unwrap();
    m.loop_action(
        &spec.id,
        LoopAction::Flags {
            repeat: true,
            count_in: true,
        },
        t0,
    )
    .unwrap();
    let snap = m.loop_action(&spec.id, LoopAction::Play, t0).unwrap();
    assert!(snap.transport.counting_in);
    let s = m.session.as_ref().unwrap();
    let start_ms = s.boundary_ms(s.state.current().unwrap().start);
    let plan = s.plan_at(start_ms, 60).unwrap();
    assert_eq!(s.ms_of(plan.clicks[1]), 833); // the log line's beat_ms at 120 BPM and 60 %
    let spec_loop = s.loop_spec().unwrap();
    assert_eq!(spec_loop.training, Some(t));
    assert_eq!(spec_loop.count_in_bpm_milli, Some(120_000));
    assert_eq!(
        spec_loop.count_in_lead, 0,
        "a loop starting on a tick of the grid"
    );
}

#[test]
fn the_count_in_plan_leads_into_the_next_beat_at_the_speed() {
    let (_t, _spec, m, _b, _e) = opened(WITH_TEMPO);
    let s = m.session.as_ref().unwrap();
    // 90 BPM from 10500 ms: at 50 % a beat is 29400 output frames; 12000 is 500 ms (11025 song frames,
    // 22050 output frames) before the beat at 12500
    let p = s.plan_at(12000.0, 50).unwrap();
    assert_eq!(p.clicks, [0, 29400, 58800]);
    assert_eq!(p.start, 3 * 29400 - 22050);
    assert_eq!(s.plan_at(12000.0, 100).unwrap().start, 3 * 14700 - 11025);
    // on a beat, or a fraction of a frame after it: the audio on the 4th beat
    assert_eq!(s.plan_at(12500.0, 50).unwrap().start, 3 * 29400);
    assert_eq!(s.plan_at(12500.01, 50).unwrap().start, 3 * 29400);
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
    let spec_loop = LoopSpec {
        start: 20000,
        end: 40000,
        repeat: true,
        count_in_bpm_milli: Some(120_000),
        count_in_lead: 0,
        training: None,
    };
    let g2 = Arc::new(grid.clone());
    let mut out = vec![0.0f32; 2 * 1024];
    let empty = Arc::new(Vec::new());
    COUNTING.with(|c| c.set(true));
    ALLOCS.with(|a| a.set(0));
    en.apply(Cmd::SetGrid(empty));
    en.apply(Cmd::SetGrid(g2));
    en.apply(Cmd::SetMetronome(true));
    en.apply(Cmd::SetLoop(Some(spec_loop)));
    en.apply(Cmd::Play {
        count_in: Some(plan),
    });
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

fn new_map() -> crate::tempo_map::TempoMap {
    crate::tempo_map::TempoMap {
        beats_per_bar: 4,
        segments: vec![crate::tempo_map::Segment {
            start_ms: 0.0,
            bpm: 100.0,
            detected_ms: None,
            on_beat: false,
            meta: Default::default(),
        }],
    }
}

fn closed(e: &Events, id: &str) -> usize {
    e.lock()
        .unwrap()
        .iter()
        .filter(|x| **x == PlayerEvent::Closed { id: id.into() })
        .count()
}

#[test]
fn release_flushes_the_pending_state_then_the_tempo_write_keeps_it() {
    use crate::tempo_store::{FileTempoStore, TempoStore};
    let (_t, spec, mut m, b, e) = opened(WITH_TEMPO);
    let t0 = Instant::now();
    m.seek(&spec.id, 7000, t0).unwrap();
    m.loop_action(&spec.id, LoopAction::Add, t0).unwrap();
    let loops = m.snapshot(&spec.id).unwrap().loops;
    assert_eq!(loops.len(), 4);
    let before = disk(&spec).unwrap();
    assert_eq!(
        before["loops"].as_array().unwrap().len(),
        3,
        "the add is still pending"
    );

    assert!(m.release(&spec.id).unwrap());
    assert!(!m.is_open(&spec.id));
    assert_eq!(closed(&e, &spec.id), 1);
    assert!(!m.release(&spec.id).unwrap(), "nothing left to release");
    assert_eq!(closed(&e, &spec.id), 1);
    let flushed = disk(&spec).unwrap();
    assert_eq!(flushed["position_ms"], 7000);
    assert_eq!(flushed["loops"].as_array().unwrap().len(), 4);

    FileTempoStore
        .replace(&spec.id, &spec.dir, &|_| Ok(new_map()))
        .unwrap();
    let v = disk(&spec).unwrap();
    assert_eq!(v["position_ms"], 7000);
    assert_eq!(v["loops"].as_array().unwrap().len(), 4);
    assert_eq!(v["tempo"]["segments"].as_array().unwrap().len(), 1);
    assert_eq!(v["tempo"]["segments"][0]["bpm"], 100.0);

    // the released session's debounced writer is gone: nothing brings the old tempo back
    m.tick(t0 + ms(600));
    m.tick(t0 + ms(20_000));
    assert_eq!(disk(&spec).unwrap(), v);
    drop(b);
}

#[test]
fn a_failed_flush_keeps_the_session_and_refuses_the_replace() {
    use crate::tempo_store::{FileTempoStore, TempoStore};
    let (_t, spec, mut m, _b, e) = opened(PRACTICE);
    m.seek(&spec.id, 3000, Instant::now()).unwrap();
    std::fs::remove_dir_all(&spec.dir).unwrap();
    let err = m.release(&spec.id).unwrap_err();
    assert!(
        err.starts_with("Could not save the Player's settings first:"),
        "{err}"
    );
    assert!(m.is_open(&spec.id));
    assert_eq!(closed(&e, &spec.id), 0);
    // the store only writes after a successful release, and never creates the folder
    assert!(FileTempoStore
        .replace(&spec.id, &spec.dir, &|_| Ok(new_map()))
        .is_err());
    assert!(!spec.dir.exists());
}

#[test]
fn release_refuses_a_newer_schema_file_and_keeps_the_session() {
    let (_t, spec, mut m, _b, e) = opened(PRACTICE);
    std::fs::write(spec.dir.join("player.json"), br#"{"schema_version": 9}"#).unwrap();
    let err = m.release(&spec.id).unwrap_err();
    assert_eq!(err, crate::tempo_store::NEWER);
    assert!(m.is_open(&spec.id));
    assert_eq!(closed(&e, &spec.id), 0);
}

#[test]
fn flush_if_open_saves_a_dirty_session_and_leaves_it_open() {
    let (_t, spec, mut m, _b, e) = opened(PRACTICE);
    m.flush_if_open("other").unwrap();
    m.flush_if_open(&spec.id).unwrap();
    assert!(disk(&spec).is_none(), "a clean session writes nothing");
    let t0 = Instant::now();
    m.seek(&spec.id, 3000, t0).unwrap();
    m.flush_if_open(&spec.id).unwrap();
    assert_eq!(disk(&spec).unwrap()["position_ms"], 3000);
    assert!(m.is_open(&spec.id));
    assert_eq!(closed(&e, &spec.id), 0);
}

#[test]
fn finish_open_installs_a_player_json_written_after_the_read() {
    use crate::tempo_store::{FileTempoStore, TempoStore};
    let (_t, spec) = spec_of(WITH_TEMPO);
    let (mut m, _b, _e) = manager();
    let ticket = m.begin_open();
    let loaded = load(&spec, &ticket, &m.sink()).unwrap();
    assert!(m.finish_open(&ticket, &spec, loaded).is_ok());
    // a second open: read, then the Editor writes, then install
    let ticket = m.begin_open();
    let loaded = load(&spec, &ticket, &m.sink()).unwrap();
    FileTempoStore
        .replace(&spec.id, &spec.dir, &|_| Ok(new_map()))
        .unwrap();
    let snap = m.finish_open(&ticket, &spec, loaded).unwrap();
    assert_eq!(snap.tempo.segments.len(), 1);
    assert_eq!(snap.tempo.segments[0].bpm, 100.0);
    assert_eq!(snap.loops.len(), 3, "the Player's loops are kept");
    // an unchanged file is not re-read
    let ticket = m.begin_open();
    let loaded = load(&spec, &ticket, &m.sink()).unwrap();
    assert_eq!(
        m.finish_open(&ticket, &spec, loaded)
            .unwrap()
            .tempo
            .segments
            .len(),
        1
    );
}

// ------------------------------------------------------------- stretch mode (plan 2.4)

const BEAT: usize = RATE / 2; // one beat at 120 BPM
const PULL: usize = 64;

fn pcm_of(f: impl Fn(usize) -> f32, frames: usize) -> Pcm {
    Pcm {
        rate: RATE as u32,
        channels: 1,
        frames: frames as u64,
        samples: (0..frames).map(|i| (f(i) * 32767.0) as i16).collect(),
    }
}

/// A 300 Hz burst of 10 ms (sine-squared envelope) on every beat of 120 BPM.
fn burst_pcm(secs: usize) -> Pcm {
    let len = RATE / 100;
    pcm_of(
        |i| {
            let k = i % BEAT;
            if k >= len {
                return 0.0;
            }
            let env = (std::f32::consts::PI * k as f32 / len as f32).sin().powi(2);
            0.8 * env * (std::f32::consts::TAU * 300.0 * i as f32 / RATE as f32).sin()
        },
        secs * RATE,
    )
}

fn tone_pcm(secs: usize) -> Pcm {
    pcm_of(
        |i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin(),
        secs * RATE,
    )
}

fn beat_grid(frames: usize) -> Arc<Vec<Beat>> {
    Arc::new(
        (0..frames / BEAT + 1)
            .map(|k| ((k * BEAT) as u64, false))
            .collect(),
    )
}

/// Renders `total` frames in 64-frame pulls, applying each command before the first pull at or after its frame.
/// Returns the left channel and the position after every pull.
fn render_cmds(pcm: Pcm, cmds: &[(usize, Cmd)], total: usize) -> (Vec<f32>, Vec<u64>, Arc<Shared>) {
    let shared = Arc::new(Shared::default());
    let mut en = Engine::new(Arc::new(pcm), 0, shared.clone());
    let (mut left, mut pos) = (Vec::new(), Vec::new());
    let mut buf = vec![0.0f32; 2 * PULL];
    let mut next = 0;
    for k in 0..total / PULL {
        while next < cmds.len() && cmds[next].0 <= k * PULL {
            en.apply(cmds[next].1.clone());
            next += 1;
        }
        en.render(&mut buf);
        left.extend(buf.iter().step_by(2));
        pos.push(shared.position.load(Ordering::Acquire));
    }
    (left, pos, shared)
}

/// RMS per 5 ms window.
fn envelope(x: &[f32]) -> Vec<f32> {
    let w = RATE / 200;
    x.chunks(w)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

/// Energy centres (in frames) of the clusters of 5 ms windows above 30 % of the maximum, i.e. the bursts.
fn burst_peaks(x: &[f32]) -> Vec<usize> {
    let env = envelope(x);
    let top = env.iter().cloned().fold(0.0, f32::max);
    let (mut out, mut sum, mut moment) = (Vec::new(), 0.0f64, 0.0f64);
    for (k, &e) in env.iter().chain([&0.0]).enumerate() {
        if e > 0.3 * top {
            sum += f64::from(e * e);
            moment += f64::from(e * e) * ((k * (RATE / 200) + RATE / 400) as f64);
        } else if sum > 0.0 {
            out.push((moment / sum) as usize);
            (sum, moment) = (0.0, 0.0);
        }
    }
    out
}

/// First non-zero frame of every sound that follows at least 50 ms of silence.
fn onsets(x: &[f32]) -> Vec<usize> {
    let gap = RATE / 20;
    let mut out = Vec::new();
    let mut last: Option<usize> = None;
    for (i, v) in x.iter().enumerate() {
        if v.abs() > 1e-6 {
            if last.is_none_or(|l| i - l > gap) {
                out.push(i);
            }
            last = Some(i);
        }
    }
    out
}

fn tone_rms() -> f32 {
    0.5 / 2f32.sqrt()
}

/// No 10 ms window of `x[from..to]` is below half the tone's RMS.
fn assert_no_gap(x: &[f32], from: usize, to: usize, what: &str) {
    let w = RATE / 100;
    for (k, c) in x[from..to.min(x.len())].chunks_exact(w).enumerate() {
        let rms = (c.iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt();
        assert!(
            rms >= 0.5 * tone_rms(),
            "{what}: window at frame {} has rms {rms}",
            from + k * w
        );
    }
}

fn scenario(name: &str, pct: u32) -> (Vec<(usize, Cmd)>, usize) {
    let sec = RATE;
    let mut c = vec![(0, Cmd::SetMetronome(true))];
    match name {
        "play" => {
            c.extend([(0, Cmd::SetSpeed(pct)), (0, Cmd::Play { count_in: None })]);
            (c, 6 * sec)
        }
        "jump" => {
            c.extend([
                (0, Cmd::SetSpeed(pct)),
                (0, Cmd::Play { count_in: None }),
                (3 * sec / 2, Cmd::Jump(72_765)),
            ]);
            (c, 6 * sec)
        }
        "wrap" => {
            let l = LoopSpec {
                start: 2 * sec as u64,
                end: 4 * sec as u64,
                repeat: true,
                count_in_bpm_milli: None,
                count_in_lead: 0,
                training: None,
            };
            c.extend([
                (0, Cmd::SetSpeed(pct)),
                (0, Cmd::SetLoop(Some(l))),
                (0, Cmd::Play { count_in: None }),
            ]);
            (c, 12 * sec)
        }
        _ => {
            // 100 % -> pct while playing, then on to another stretched speed
            let other = if pct == 150 { 50 } else { 150 };
            c.extend([
                (0, Cmd::Play { count_in: None }),
                (3 * sec / 2, Cmd::SetSpeed(pct)),
                (4 * sec, Cmd::SetSpeed(other)),
            ]);
            (c, 8 * sec)
        }
    }
}

#[test]
fn clicks_stay_on_the_beats_of_the_music_at_any_speed() {
    for pct in [50u32, 85, 150] {
        for name in ["play", "jump", "wrap", "change"] {
            let (cmds, total) = scenario(name, pct);
            let mut click_cmds = vec![(0, Cmd::SetGrid(beat_grid(30 * RATE)))];
            click_cmds.extend(cmds.iter().cloned());
            let (clicks, _, _) = render_cmds(pcm_of(|_| 0.0, 30 * RATE), &click_cmds, total);
            let (music, _, _) = render_cmds(burst_pcm(30), &cmds, total);
            let (on, pk) = (onsets(&clicks), burst_peaks(&music));
            assert!(
                on.len() >= 5 && pk.len() >= 4,
                "{name} {pct}: {} clicks, {} bursts",
                on.len(),
                pk.len()
            );
            let tol = RATE / 100; // 10 ms
            for &o in &on {
                // the burst's peak is the middle of its 10 ms, stretched
                let centre = o + (RATE as f64 / 200.0 * 100.0 / f64::from(pct)) as usize;
                let near = pk.iter().map(|&p| p.abs_diff(centre)).min().unwrap();
                assert!(
                    near <= tol,
                    "{name} {pct} %: click at {o} is {near} frames from the nearest burst peak"
                );
            }
            if name == "play" {
                for w in on.windows(2) {
                    let want = BEAT as f64 * 100.0 / f64::from(pct);
                    assert!(
                        (w[1] - w[0]) as f64 - want <= 1.0 && want - (w[1] - w[0]) as f64 <= 1.0,
                        "{name} {pct}: spacing {}",
                        w[1] - w[0]
                    );
                }
            }
            if name == "change" {
                // 5.5 s on, the speed is `other`: spacing follows it
                let other = if pct == 150 { 50.0 } else { 150.0 };
                let tail: Vec<usize> = on
                    .iter()
                    .cloned()
                    .filter(|&o| o > 5 * RATE + RATE / 2)
                    .collect();
                for w in tail.windows(2) {
                    let want = BEAT as f64 * 100.0 / other;
                    assert!(
                        ((w[1] - w[0]) as f64 - want).abs() <= 1.0,
                        "{name} {pct}: tail spacing {}",
                        w[1] - w[0]
                    );
                }
            }
        }
    }
}

#[test]
fn speed_100_with_set_speed_is_still_bit_exact() {
    let pcm = decoded(&spec_of(PRACTICE).1, "backing:backing");
    let want = expected(&pcm, 0, 4000);
    let shared = Arc::new(Shared::default());
    let mut en = Engine::new(Arc::new(pcm), 0, shared);
    en.apply(Cmd::SetSpeed(100));
    en.apply(Cmd::Play { count_in: None });
    let mut got = vec![0.0f32; 2 * 4000];
    en.render(&mut got);
    // the 3 ms fade-in is the only difference from the source, as in the other exact tests
    let f = fade_frames();
    assert_eq!(got[2 * f..], want[2 * f..]);
}

#[test]
fn speed_change_while_playing_has_no_gap_and_the_position_never_goes_back() {
    let sec = RATE;
    let cmds = [
        (0, Cmd::Play { count_in: None }),
        (sec, Cmd::SetSpeed(70)),
        (3 * sec, Cmd::SetSpeed(100)),
    ];
    let (x, pos, shared) = render_cmds(tone_pcm(10), &cmds, 6 * sec);
    assert_no_gap(&x, 0, 6 * sec, "100 -> 70 -> 100 %");
    assert!(
        pos.windows(2).all(|w| w[1] >= w[0]),
        "the position went back"
    );
    let at = |s: f64| pos[(s * sec as f64) as usize / PULL] as f64;
    let rate = (at(2.9) - at(1.9)) / sec as f64;
    assert!((rate - 0.7).abs() < 0.07, "advances {rate} s per second");
    // back at 100 %: the bypass again, bit-exact from the source once the 3 ms crossfade has passed
    let src = tone_pcm(10);
    let from = 3 * sec / PULL + 2 * fade_frames() / PULL + 2;
    for k in from..6 * sec / PULL {
        for j in 0..PULL {
            let f = pos[k] as usize - PULL + j;
            assert_eq!(
                x[k * PULL + j],
                f32::from(src.samples[f]) / 32768.0,
                "pull {k} frame {j}"
            );
        }
    }
    assert_eq!(shared.speed_pct.load(Ordering::Acquire), 100);
}

#[test]
fn a_repeating_loop_at_80_percent_wraps_without_a_gap() {
    let sec = RATE;
    let l = LoopSpec {
        start: sec as u64,
        end: 2 * sec as u64,
        repeat: true,
        count_in_bpm_milli: None,
        count_in_lead: 0,
        training: None,
    };
    let cmds = [
        (0, Cmd::SetSpeed(80)),
        (0, Cmd::SetLoop(Some(l))),
        (0, Cmd::Seek(sec as u64)),
        (0, Cmd::Play { count_in: None }),
        (0, Cmd::Resume),
    ];
    let (x, pos, shared) = render_cmds(tone_pcm(10), &cmds, 5 * sec);
    assert!(shared.wraps.load(Ordering::Acquire) >= 2);
    // after the first wrap the heard position is back at the loop start
    let first = pos
        .windows(2)
        .position(|w| w[1] + 1000 < w[0])
        .expect("the position wrapped");
    assert!(
        pos[first + 1].abs_diff(sec as u64) < 2000,
        "back at {}",
        pos[first + 1]
    );
    assert!(pos.iter().all(|&p| p <= 2 * sec as u64 + 100));
    assert_no_gap(&x, fade_frames() * 4, 5 * sec, "loop wrap");
}

#[test]
fn loop_end_with_count_in_at_50_percent_counts_three_clicks_then_the_music() {
    let sec = RATE;
    let l = LoopSpec {
        start: 2 * BEAT as u64,
        end: 6 * BEAT as u64,
        repeat: true,
        count_in_bpm_milli: Some(120_000),
        count_in_lead: 0,
        training: None,
    };
    let cmds = [
        (0, Cmd::SetSpeed(50)),
        (0, Cmd::SetLoop(Some(l))),
        (0, Cmd::PlayLoop { count_in: None }),
    ];
    let total = 9 * sec;
    let (clicks, _, shared) = render_cmds(pcm_of(|_| 0.0, 30 * sec), &cmds, total);
    let (music, _, _) = render_cmds(burst_pcm(30), &cmds, total);
    assert!(shared.wraps.load(Ordering::Acquire) >= 1);
    let on = onsets(&clicks);
    assert!(on.len() >= 3, "{on:?}");
    for w in on[..3].windows(2) {
        assert!(
            w[1].abs_diff(w[0] + sec) <= 2,
            "clicks {} frames apart",
            w[1] - w[0]
        );
    }
    // the burst on the silent fourth beat: the loop start is a beat
    let after: Vec<usize> = burst_peaks(&music[on[2] + sec / 2..])
        .iter()
        .map(|p| p + on[2] + sec / 2)
        .collect();
    let want = on[2] + sec + RATE / 100; // the middle of the burst, stretched to 20 ms
    assert!(
        after[0].abs_diff(want) <= RATE / 100,
        "burst at {} want {want}",
        after[0]
    );
}

/// A silent track's clicks after `cmds`: the count-in, then the 120 BPM grid (a click on every beat).
fn count_in_then_grid(pct: u32, cmds: Vec<(usize, Cmd)>) -> Vec<i64> {
    let grid: Vec<Beat> = (0..60).map(|k| ((k * BEAT) as u64, false)).collect();
    let mut all = vec![
        (0, Cmd::SetGrid(Arc::new(grid))),
        (0, Cmd::SetMetronome(true)),
        (0, Cmd::SetSpeed(pct)),
    ];
    all.extend(cmds);
    let (x, _, _) = render_cmds(pcm_of(|_| 0.0, 30 * RATE), &all, 10 * RATE);
    onsets(&x)
        .windows(2)
        .map(|w| w[1] as i64 - w[0] as i64)
        .take(6)
        .collect()
}

#[test]
fn a_count_in_between_beats_lands_on_the_grid_at_every_speed() {
    for pct in [100u32, 50, 75] {
        let out_beat = BEAT as f64 * 100.0 / f64::from(pct);
        for start in [
            4 * BEAT,
            4 * BEAT + BEAT / 2,
            4 * BEAT + BEAT / 10,
            5 * BEAT - BEAT / 10,
        ] {
            let lead = (((5 * BEAT - start) % BEAT) as f64 * 100.0 / f64::from(pct)).round() as u64;
            let plan = count_in_plan(120.0 * f64::from(pct) / 100.0, RATE as u32).lead_in(lead);
            let gaps = count_in_then_grid(
                pct,
                vec![
                    (0, Cmd::Seek(start as u64)),
                    (
                        0,
                        Cmd::Play {
                            count_in: Some(plan),
                        },
                    ),
                ],
            );
            assert_eq!(gaps.len(), 6, "{pct} % from {start}: {gaps:?}");
            for g in &gaps {
                // the stretch path places a click within a sub-block
                assert!(
                    (*g as f64 - out_beat).abs() <= 300.0,
                    "{pct} % from {start}: {gaps:?}"
                );
            }
        }
    }
}

#[test]
fn a_loop_count_in_between_beats_lands_on_the_grid_on_every_pass() {
    for pct in [100u32, 50] {
        let out_beat = BEAT as f64 * 100.0 / f64::from(pct);
        let start = 2 * BEAT + BEAT / 2;
        let l = LoopSpec {
            start: start as u64,
            end: 6 * BEAT as u64,
            repeat: true,
            count_in_bpm_milli: Some(120_000),
            count_in_lead: (BEAT / 2) as u64,
            training: None,
        };
        let plan = count_in_plan(120.0 * f64::from(pct) / 100.0, RATE as u32)
            .lead_in((BEAT as f64 / 2.0 * 100.0 / f64::from(pct)).round() as u64);
        let grid: Vec<Beat> = (0..60).map(|k| ((k * BEAT) as u64, false)).collect();
        let cmds = [
            (0, Cmd::SetGrid(Arc::new(grid))),
            (0, Cmd::SetMetronome(true)),
            (0, Cmd::SetSpeed(pct)),
            (0, Cmd::SetLoop(Some(l))),
            (
                0,
                Cmd::PlayLoop {
                    count_in: Some(plan),
                },
            ),
        ];
        let (x, _, shared) = render_cmds(
            pcm_of(|_| 0.0, 30 * RATE),
            &cmds,
            14 * RATE * 100 / pct as usize / 2,
        );
        assert!(shared.wraps.load(Ordering::Acquire) >= 1);
        // count-in 3 clicks, beats 3, 4, 5 of the pass, the loop end on beat 6 starts the next count-in, ...:
        // one beat apart all the way
        let gaps: Vec<i64> = onsets(&x)
            .windows(2)
            .map(|w| w[1] as i64 - w[0] as i64)
            .take(11)
            .collect();
        assert_eq!(gaps.len(), 11, "{pct} %: {gaps:?}");
        for (k, g) in gaps.iter().enumerate() {
            assert!(
                (*g as f64 - out_beat).abs() <= 300.0,
                "{pct} %: gap {k} in {gaps:?}"
            );
        }
    }
}

#[test]
fn the_track_end_at_150_percent_plays_the_last_50ms_then_ends() {
    let sec = RATE;
    let cmds = [
        (0, Cmd::SetSpeed(150)),
        (0, Cmd::Seek(sec as u64)),
        (0, Cmd::Play { count_in: None }),
    ];
    let shared = Arc::new(Shared::default());
    let mut en = Engine::new(Arc::new(tone_pcm(3)), 0, shared.clone());
    for (_, c) in cmds {
        en.apply(c);
    }
    let (mut left, mut buf) = (Vec::new(), vec![0.0f32; 2 * PULL]);
    while !shared.ended.load(Ordering::Acquire) && left.len() < 5 * sec {
        en.render(&mut buf);
        left.extend(buf.iter().step_by(2));
    }
    assert!(shared.ended.load(Ordering::Acquire));
    let end = left.len();
    // 50 ms of source are 33 ms of output
    assert_no_gap(&left, end - 3 * RATE / 100 - PULL, end - PULL, "the tail");
    assert!(end > 2 * sec / 3 * 2 - sec / 20, "ended after {end} frames");
}

#[test]
fn seek_while_playing_at_85_percent_stays_silent_until_resume_then_plays_from_the_target() {
    let t = 3 * BEAT + 3000; // the next burst is 8075 frames on
    let cmds = [
        (0, Cmd::SetSpeed(85)),
        (0, Cmd::Play { count_in: None }),
        (RATE, Cmd::Seek(t as u64)),
        (RATE + 8 * RATE / 10, Cmd::Resume),
    ];
    let (x, _, _) = render_cmds(burst_pcm(30), &cmds, 3 * RATE);
    let resume = RATE + 8 * RATE / 10;
    assert!(
        x[RATE + PULL..resume].iter().all(|v| *v == 0.0),
        "not silent while scrubbing"
    );
    let pk = burst_peaks(&x[resume..]);
    let want = (8075.0 / 0.85) as usize;
    assert!(
        pk[0].abs_diff(want) <= RATE / 100,
        "burst at {} want {want}",
        pk[0]
    );
}

// ------------------------------------------------------------ speed training (engine)

/// An engine pulled in `PULL`-frame blocks, recording the left channel.
struct Rig {
    en: Engine,
    shared: Arc<Shared>,
    left: Vec<f32>,
    buf: Vec<f32>,
}

impl Rig {
    fn new(pcm: Pcm) -> Rig {
        let shared = Arc::new(Shared::default());
        let en = Engine::new(Arc::new(pcm), 0, shared.clone());
        Rig {
            en,
            shared,
            left: Vec::new(),
            buf: vec![0.0; 2 * PULL],
        }
    }

    fn cmd(&mut self, c: Cmd) -> &mut Rig {
        self.en.apply(c);
        self
    }

    fn pull(&mut self) {
        self.en.render(&mut self.buf);
        self.left.extend(self.buf.iter().step_by(2));
    }

    fn pass(&self) -> u64 {
        self.shared.pass.load(Ordering::Acquire)
    }

    fn pct(&self) -> u64 {
        self.shared.speed_pct.load(Ordering::Acquire)
    }

    /// Pulls until `pass()` is `n` (at most `limit` frames).
    fn until_pass(&mut self, n: u64, limit: usize) {
        while self.pass() != n {
            assert!(
                self.left.len() < limit,
                "pass {n} not reached (pass {} at {} %)",
                self.pass(),
                self.pct()
            );
            self.pull();
        }
    }

    fn pull_for(&mut self, frames: usize) {
        for _ in 0..frames / PULL {
            self.pull();
        }
    }
}

/// A repeating 2 s loop (1 s..3 s) of a 30 s tone.
fn loop_2s(training: Option<Training>, count_in: bool) -> LoopSpec {
    LoopSpec {
        start: RATE as u64,
        end: 3 * RATE as u64,
        repeat: true,
        count_in_bpm_milli: count_in.then_some(120_000),
        count_in_lead: 0,
        training,
    }
}

fn plan(start: u32, end: u32, step: u32, repeats: u32) -> Option<Training> {
    Some(Training {
        on: true,
        start,
        end,
        step,
        repeats,
    })
}

#[test]
fn a_training_run_plays_each_pass_at_its_speed_without_a_gap() {
    let mut r = Rig::new(tone_pcm(30));
    r.cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 2), false))))
        .cmd(Cmd::PlayLoop { count_in: None });
    let mut bounds = vec![0usize];
    let mut seen = vec![(r.pass(), r.pct())];
    r.pull();
    seen[0] = (r.pass(), r.pct());
    assert_eq!(seen[0], (1, 60));
    let limit = 30 * RATE;
    for want in 2..=7u64 {
        r.until_pass(want, limit);
        bounds.push(r.left.len());
        seen.push((r.pass(), r.pct()));
    }
    let speeds = [60, 60, 70, 70, 80, 80];
    for k in 0..6 {
        let len = (bounds[k + 1] - bounds[k]) as f64;
        let want = 2.0 * RATE as f64 * 100.0 / f64::from(speeds[k]);
        assert!(
            (len - want).abs() <= 0.02 * want,
            "pass {} is {len} frames, want {want}",
            k + 1
        );
        assert_eq!(seen[k].0, k as u64 + 1);
    }
    // each pass is published with its speed as it is heard
    assert_eq!(
        seen.iter().map(|s| s.1).collect::<Vec<_>>(),
        [60, 60, 70, 70, 80, 80, 80]
    );
    assert_no_gap(
        &r.left,
        fade_frames() * 4,
        r.left.len() - 2 * PULL,
        "pass changes",
    );
}

#[test]
fn every_pass_is_counted_in_at_its_own_speed() {
    let mut r = Rig::new(pcm_of(|_| 0.0, 30 * RATE));
    r.cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 1), true))))
        .cmd(Cmd::PlayLoop { count_in: None });
    r.until_pass(5, 40 * RATE);
    let on = onsets(&r.left);
    // no count-in before pass 1 (none was asked for); three clicks before each of passes 2, 3 and 4 (70/80/80 %)
    assert!(on.len() >= 9, "{on:?}");
    for (g, pct) in [70.0, 80.0, 80.0].into_iter().enumerate() {
        let want = BEAT as f64 * 100.0 / pct;
        for w in on[3 * g..3 * g + 3].windows(2) {
            assert!(
                ((w[1] - w[0]) as f64 - want).abs() <= 2.0,
                "group {g}: {} apart, want {want}",
                w[1] - w[0]
            );
        }
    }
}

#[test]
fn pause_ends_the_run_and_the_next_loop_play_starts_at_the_first_speed() {
    let mut r = Rig::new(tone_pcm(30));
    r.cmd(Cmd::SetSpeed(90))
        .cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 2), false))));
    r.cmd(Cmd::PlayLoop { count_in: None });
    r.until_pass(3, 20 * RATE);
    assert_eq!(r.pct(), 70);
    r.cmd(Cmd::Pause);
    r.pull();
    assert_eq!((r.pass(), r.pct()), (0, 90));
    r.cmd(Cmd::PlayLoop { count_in: None });
    r.pull();
    assert_eq!((r.pass(), r.pct()), (1, 60));
}

#[test]
fn a_jump_outside_the_loop_ends_the_run_and_a_jump_inside_keeps_it() {
    let mut r = Rig::new(tone_pcm(30));
    r.cmd(Cmd::SetSpeed(90))
        .cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 2), false))));
    r.cmd(Cmd::PlayLoop { count_in: None });
    r.until_pass(3, 20 * RATE);
    r.cmd(Cmd::Jump(RATE as u64 + RATE as u64 / 2));
    r.pull_for(RATE / 2);
    assert_eq!(
        (r.pass(), r.pct()),
        (3, 70),
        "a jump inside the loop keeps the run"
    );
    let from = r.left.len();
    r.cmd(Cmd::Jump(10 * RATE as u64));
    r.pull();
    assert_eq!((r.pass(), r.pct()), (0, 90));
    r.pull_for(2 * RATE);
    assert_eq!((r.pass(), r.pct()), (0, 90));
    assert_no_gap(
        &r.left,
        from + fade_frames() * 4 + 2 * PULL,
        r.left.len(),
        "after the jump",
    );
    assert!(
        r.shared.position.load(Ordering::Acquire) > 10 * RATE as u64,
        "playback continues past the jump target"
    );
}

#[test]
fn set_loop_without_training_ends_the_run_at_once_and_new_settings_apply_from_the_next_pass() {
    let mut r = Rig::new(tone_pcm(30));
    r.cmd(Cmd::SetSpeed(90))
        .cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 2), false))));
    r.cmd(Cmd::PlayLoop { count_in: None });
    r.until_pass(2, 20 * RATE);
    // repeats 2 -> 1 in the middle of pass 2: this pass stays at 60 %, the next one is pass 3 of (60, 80, 10, 1) = 80 %
    r.pull_for(RATE / 2);
    r.cmd(Cmd::SetLoop(Some(loop_2s(plan(60, 80, 10, 1), false))));
    r.pull();
    assert_eq!((r.pass(), r.pct()), (2, 60));
    r.until_pass(3, 20 * RATE);
    assert_eq!(r.pct(), 80);
    r.cmd(Cmd::SetLoop(Some(loop_2s(None, false))));
    r.pull();
    assert_eq!(r.pass(), 0);
    r.pull_for(RATE / 4); // the new ratio is heard after the stretcher's latency
    assert_eq!(r.pct(), 90);
    // the loop itself goes on repeating, at the track speed
    let wraps = r.shared.wraps.load(Ordering::Acquire);
    r.pull_for(3 * RATE);
    assert!(r.shared.wraps.load(Ordering::Acquire) > wraps);
    assert_eq!(r.pct(), 90);
}

#[test]
fn a_run_from_100_to_100_stays_in_stretch_mode_with_2s_passes() {
    let mut r = Rig::new(tone_pcm(30));
    r.cmd(Cmd::SetLoop(Some(loop_2s(plan(100, 100, 5, 1), false))))
        .cmd(Cmd::PlayLoop { count_in: None });
    r.pull();
    assert!(r.en.mode == Mode::Stretch);
    let mut bounds = vec![0usize];
    for want in 2..=4u64 {
        r.until_pass(want, 10 * RATE);
        bounds.push(r.left.len());
    }
    for w in bounds.windows(2) {
        let len = (w[1] - w[0]) as f64;
        assert!(
            (len - 2.0 * RATE as f64).abs() <= 0.02 * 2.0 * RATE as f64,
            "pass of {len} frames"
        );
    }
    assert!(r.en.mode == Mode::Stretch);
}

#[test]
fn a_jump_into_the_last_100ms_of_a_repeating_loop_still_wraps() {
    for pct in [100u32, 85] {
        let mut r = Rig::new(tone_pcm(30));
        r.cmd(Cmd::SetSpeed(pct))
            .cmd(Cmd::SetLoop(Some(loop_2s(None, false))))
            .cmd(Cmd::Seek(RATE as u64 * 2));
        r.cmd(Cmd::Play { count_in: None }).cmd(Cmd::Resume);
        r.pull_for(RATE / 4);
        r.cmd(Cmd::Jump(3 * RATE as u64 - RATE as u64 / 20));
        r.pull_for(RATE / 2);
        assert!(
            r.shared.wraps.load(Ordering::Acquire) >= 1,
            "{pct} %: the loop end was ignored"
        );
        let pos = r.shared.position.load(Ordering::Acquire);
        assert!(
            pos < 2 * RATE as u64,
            "{pct} %: position {pos} is not back in the loop"
        );
    }
}
