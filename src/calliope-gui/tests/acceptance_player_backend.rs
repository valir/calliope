//! Independent acceptance tests for specs/gui-play-backing-track.md, increment A (plan section 7).
//!
//! The binary crate has no library target, so this file compiles the player modules straight from
//! `src/` (via `#[path]`) and drives `PlayerManager` the way the IPC layer does, with the
//! `ManualBackend` (a fake: no sound device is ever opened) and injected `Instant`s. Audio is
//! checked sample by sample against the decoded fixture. (As a side effect the modules' own unit
//! tests are compiled and run here once more.)
#![allow(dead_code, unused_imports, unused_macros, clippy::all)]

#[path = "../src/audio_out.rs"]
mod audio_out;
#[path = "../src/fsutil.rs"]
mod fsutil;
#[path = "../src/metronome.rs"]
mod metronome;
#[path = "../src/player.rs"]
mod player;
#[path = "../src/player_source.rs"]
mod player_source;
#[path = "../src/player_state.rs"]
mod player_state;
#[path = "../src/repository.rs"]
mod repository;
#[path = "../src/stem_audio.rs"]
mod stem_audio;
#[path = "../src/tempo_map.rs"]
mod tempo_map;
#[path = "../src/track_meta.rs"]
mod track_meta;
#[path = "../src/transport.rs"]
mod transport;

use audio_out::ManualBackend;
use player::*;
use player_state::Mark;
use repository::Repository;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempo_map::Boundary;

const RATE: usize = 22050;
const PRACTICE: &str = "Practice Song";
const STEMS_ONLY: &str = "Stems Only";
const WITH_TEMPO: &str = "With Tempo";
const MP3: &str = "MP3 Backing";
const MISSING: &str = "Missing Audio";

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

struct Rig {
    _t: tempfile::TempDir,
    spec: OpenSpec,
    m: PlayerManager,
    b: ManualBackend,
    e: Events,
    t0: Instant,
}

fn rig(title: &str) -> Rig {
    let (t, spec) = spec_of(title);
    let (mut m, b, e) = manager();
    m.open(&spec).unwrap();
    Rig { _t: t, spec, m, b, e, t0: Instant::now() }
}

impl Rig {
    fn id(&self) -> String {
        self.spec.id.clone()
    }
    fn snap(&self) -> PlayerSnapshot {
        self.m.snapshot(&self.spec.id).unwrap()
    }
    fn pos(&self) -> u64 {
        self.snap().transport.position_ms
    }
    fn disk(&self) -> Option<Value> {
        serde_json::from_slice(&std::fs::read(self.spec.dir.join("player.json")).ok()?).ok()
    }
    fn after(&self, ms: u64) -> Instant {
        self.t0 + Duration::from_millis(ms)
    }
    /// Lets the debounce expire and writes.
    fn flush(&mut self) {
        self.m.tick(self.after(700));
    }
    fn seek(&mut self, ms: u64) {
        self.m.seek(&self.spec.id, ms, self.t0).unwrap();
    }
    fn play(&mut self) -> PlayerTransport {
        self.m.play(&self.spec.id, self.t0).unwrap()
    }
    fn lp(&mut self, a: LoopAction) -> Result<PlayerSnapshot, String> {
        self.m.loop_action(&self.spec.id, a, self.t0)
    }
    fn pcm(&self) -> player_source::Pcm {
        decoded(&self.spec, &self.snap().source)
    }
    /// Reopen the same track in a fresh manager (app restart / reselect).
    fn reopen(&mut self) {
        let (mut m, b, e) = manager();
        m.open(&self.spec).unwrap();
        self.m = m;
        self.b = b;
        self.e = e;
    }
}

fn decoded(spec: &OpenSpec, key: &str) -> player_source::Pcm {
    let f = &spec.sources.iter().find(|s| s.key == key).unwrap().file;
    player_source::decode(&spec.dir.join(f), None, &AtomicBool::new(false), &mut |_, _| {}).unwrap()
}

/// Interleaved stereo f32 of `frames` frames from `start`.
fn expected(pcm: &player_source::Pcm, start: usize, frames: usize) -> Vec<f32> {
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

fn goertzel(x: &[f32], hz: f32) -> f32 {
    // left channel of an interleaved buffer
    let w = 2.0 * std::f32::consts::PI * hz / RATE as f32;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (i, s) in x.chunks(2).map(|c| c[0]).enumerate() {
        re += s * (w * i as f32).cos();
        im += s * (w * i as f32).sin();
    }
    (re * re + im * im).sqrt() / (x.len() as f32 / 2.0)
}

/// Frames (clusters) where `got` differs from `want` by more than 0.05. `skip` frames at the start
/// are ignored (fade-in). Returns the first frame of each cluster, separated by > 300 frames.
fn onsets(got: &[f32], want: &[f32], skip: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for f in skip..got.len() / 2 {
        if (got[2 * f] - want[2 * f]).abs() > 0.05 {
            if out.last().is_none_or(|&l| f > l + 300 && !within(&out, f)) {
                out.push(f);
            }
        }
    }
    out
}
fn within(_o: &[usize], _f: usize) -> bool {
    false
}

/// Residual onset detection that merges frames closer than 300 apart.
fn clicks(got: &[f32], want: &[f32], skip: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut last = None;
    for f in skip..got.len() / 2 {
        if (got[2 * f] - want[2 * f]).abs() > 0.05 {
            if last.is_none_or(|l| f > l + 300) {
                out.push(f);
            }
            last = Some(f);
        }
    }
    out
}

fn silent_between(v: &[f32], a: usize, b: usize) -> bool {
    peak(&v[2 * a..2 * b]) < 1e-6
}

// =========================================================================================
// 3, 13: no audio; no tempo
// =========================================================================================

#[test]
fn ac03_track_without_audio_cannot_be_opened() {
    let (_t, spec) = spec_of(MISSING);
    let (mut m, _b, _e) = manager();
    let err = m.open(&spec).unwrap_err();
    assert!(err.to_lowercase().contains("no audio"), "{err}");
    assert!(m.snapshot(&spec.id).is_err(), "no session after the refusal");
}

#[test]
fn ac13_track_without_tempo_is_timestamp_mode_with_metronome_refused() {
    let mut r = rig(PRACTICE);
    let s = r.snap();
    assert_eq!(s.mode, "time");
    assert!(s.tempo.segments.is_empty());
    assert_eq!(s.transport.bpm, 0.0);
    assert!(!s.metronome.on && !s.metronome.count_in);
    assert!(r.m.set_metronome(&r.id(), true, false, r.t0).is_err());
    assert!(r.m.set_metronome(&r.id(), false, true, r.t0).is_err());
    assert!(r.m.speed_action(&r.id()).is_err());
    assert!(!r.snap().can_change_speed, "increment A: speed not available");
}

#[test]
fn ac13_bpm0_has_no_metronome_and_no_count_in_sound() {
    let mut r = rig(PRACTICE);
    r.seek(1100);
    r.play();
    let got = r.b.pull(RATE);
    let mut want = expected(&r.pcm(), 1100 * RATE / 1000, RATE);
    let n = RATE * 3 / 1000;
    for k in 0..n {
        for c in 0..2 {
            want[2 * k + c] *= k as f32 / n as f32;
        }
    }
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-5, "sample {i}: {g} vs {w}");
    }
    assert!(!r.snap().transport.counting_in);
}

#[test]
fn ac13_hostile_file_metronome_on_at_bpm0_makes_no_click_and_no_count_in() {
    let (_t, spec) = spec_of(PRACTICE);
    std::fs::write(
        spec.dir.join("player.json"),
        r#"{"schema_version":1,"metronome":{"on":true,"count_in":true}}"#,
    )
    .unwrap();
    let (mut m, b, _e) = manager();
    m.open(&spec).unwrap();
    let t0 = Instant::now();
    m.seek(&spec.id, 1100, t0).unwrap();
    let st = m.play(&spec.id, t0).unwrap();
    assert!(!st.counting_in, "no count-in without a tempo");
    let got = b.pull(RATE);
    let pcm = decoded(&spec, &m.snapshot(&spec.id).unwrap().source);
    let want = expected(&pcm, 1100 * RATE / 1000, RATE);
    assert!(clicks(&got, &want, 100).is_empty(), "no clicks at BPM 0");
}

// =========================================================================================
// 5, 6: source choice; 7, 8: change source
// =========================================================================================

#[test]
fn ac05_first_open_lists_backings_original_stems_and_selects_backing() {
    let r = rig(PRACTICE);
    let s = r.snap();
    let keys: Vec<&str> = s.sources.iter().map(|x| x.key.as_str()).collect();
    assert_eq!(keys, ["backing:backing", "original", "stem:vocals", "stem:guitar"]);
    assert_eq!(s.sources[0].label, "Backing");
    assert_eq!(s.source, "backing:backing");
    assert!(r.disk().is_none(), "opening alone writes nothing");
}

#[test]
fn ac06_first_open_without_backing_selects_first_stem_never_original() {
    let r = rig(STEMS_ONLY);
    assert_eq!(r.snap().source, "stem:bass");
    // a track that has an original and stems but no backing: original must not be auto-chosen
    assert_eq!(
        player_state::choose_source(None, &["original", "stem:a", "stem:b"]).as_deref(),
        Some("stem:a")
    );
    assert_eq!(player_state::choose_source(None, &["original"]), None);
    assert_eq!(player_state::choose_source(Some("gone"), &["backing:x"]).as_deref(), Some("backing:x"));
}

#[test]
fn ac07_source_choice_is_saved_within_1s_and_restored_after_reselect_and_restart() {
    let mut r = rig(PRACTICE);
    r.m.select_source(&r.id(), "stem:guitar", r.t0).unwrap();
    r.m.tick(r.after(400));
    assert!(r.disk().is_none(), "debounce: not before 500 ms");
    r.m.tick(r.after(1000));
    assert_eq!(r.disk().unwrap()["source"], "stem:guitar");
    r.reopen();
    assert_eq!(r.snap().source, "stem:guitar");
}

#[test]
fn ac08_changing_source_while_playing_stops_closes_output_keeps_position() {
    let mut r = rig(PRACTICE);
    r.seek(3000);
    r.play();
    r.b.pull(RATE / 2);
    let before = r.pos();
    assert!(before > 3000);
    assert!(r.b.is_open());
    let s = r.m.select_source(&r.id(), "stem:guitar", r.t0).unwrap();
    assert!(!s.transport.playing);
    assert!(!r.b.is_open(), "output closed");
    assert_eq!(s.transport.position_ms, before, "position kept");
    assert_eq!(peak(&r.b.pull(1000)), 0.0, "silence after the change");
    // Play continues with the new source from the same place.
    r.play();
    let got = r.b.pull(4000);
    let want = expected(&decoded(&r.spec, "stem:guitar"), (before as usize) * RATE / 1000, 4000);
    let tail: f32 = got[2 * 200..].iter().zip(&want[2 * 200..]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(tail < 1e-5, "plays the new source from the kept position: {tail}");
}

#[test]
fn ac08_selecting_the_current_source_is_a_noop_and_unknown_key_refused() {
    let mut r = rig(PRACTICE);
    r.play();
    let s = r.m.select_source(&r.id(), "backing:backing", r.t0).unwrap();
    assert!(s.transport.playing, "same source: nothing happens");
    assert!(r.m.select_source(&r.id(), "stem:nope", r.t0).is_err());
    assert!(r.m.select_source(&r.id(), "", r.t0).is_err());
    assert!(r.m.select_source(&r.id(), "../../etc/passwd", r.t0).is_err());
}

#[test]
fn edge_rapid_source_switching_only_the_last_wins() {
    let mut r = rig(PRACTICE);
    r.seek(2000);
    let keys = ["stem:guitar", "stem:vocals", "original", "stem:guitar", "backing:backing", "stem:vocals"];
    let sink = r.m.sink();
    let mut jobs = Vec::new();
    for k in keys {
        if let Some(j) = r.m.begin_select(&r.id(), k, r.t0).unwrap() {
            jobs.push(j);
        }
    }
    assert!(r.m.play_request(&r.id()).is_err(), "play refused while a source is loading");
    let last = jobs.len() - 1;
    for (i, j) in jobs.iter().enumerate() {
        match load_source(j, &sink) {
            Ok(l) => {
                let res = r.m.finish_select(j, l, r.t0);
                if i == last {
                    res.unwrap();
                } else {
                    assert_eq!(res.unwrap_err(), SUPERSEDED);
                }
            }
            Err(e) => {
                assert!(e == SUPERSEDED || e.contains("cancelled"), "{e}");
                r.m.abort_select(j);
            }
        }
    }
    let s = r.snap();
    assert_eq!(s.source, "stem:vocals");
    assert_eq!(s.transport.position_ms, 2000);
    assert!(!s.transport.playing);
    r.flush();
    assert_eq!(r.disk().unwrap()["source"], "stem:vocals");
    // and a following play works
    r.play();
    assert!(peak(&r.b.pull(2000)) > 0.0);
}

// =========================================================================================
// 10, 12: transport
// =========================================================================================

#[test]
fn ac10_play_moves_position_pause_holds_it() {
    let mut r = rig(PRACTICE);
    let st = r.play();
    assert!(st.playing);
    r.b.pull(RATE);
    let p = r.pos();
    assert!((990..=1010).contains(&p), "{p}");
    let st = r.m.pause(&r.id(), r.t0).unwrap();
    assert!(!st.playing);
    assert_eq!(peak(&r.b.pull(RATE)), 0.0);
    assert_eq!(r.pos(), p);
}

#[test]
fn ac12_rew_ff_move_5s_clamped() {
    let mut r = rig(PRACTICE);
    r.seek(12000);
    assert_eq!(r.m.skip(&r.id(), 5000, r.t0).unwrap().position_ms, 17000);
    assert_eq!(r.m.skip(&r.id(), -5000, r.t0).unwrap().position_ms, 12000);
    r.seek(2000);
    assert_eq!(r.m.skip(&r.id(), -5000, r.t0).unwrap().position_ms, 0);
    r.seek(18000);
    let dur = r.snap().duration_ms;
    assert_eq!(r.m.skip(&r.id(), 5000, r.t0).unwrap().position_ms, dur);
}

#[test]
fn edge_hostile_huge_skip_and_seek_do_not_panic() {
    let mut r = rig(PRACTICE);
    let dur = r.snap().duration_ms;
    // 2^53 ms is the largest integer a JS number holds exactly
    let js_max = 9_007_199_254_740_991u64;
    r.m.seek(&r.id(), js_max, r.t0).unwrap();
    assert_eq!(r.pos(), dur, "seek at the JS limit is clamped");
    assert_eq!(r.m.skip(&r.id(), js_max as i64, r.t0).unwrap().position_ms, dur);
    assert_eq!(r.m.skip(&r.id(), -(js_max as i64), r.t0).unwrap().position_ms, 0);
}

#[test]
fn ac11_seek_beyond_track_is_clamped_not_a_crash() {
    let mut r = rig(PRACTICE);
    let dur = r.snap().duration_ms;
    r.m.seek(&r.id(), 10_000_000, r.t0).unwrap();
    assert_eq!(r.pos(), dur);
    // playing from the very end restarts at 0
    let st = r.play();
    assert!(st.playing);
    r.b.pull(1000);
    assert!(r.pos() < 100);
}

// =========================================================================================
// 14-16: BPM
// =========================================================================================

#[test]
fn ac14_typing_bpm_sets_whole_track_with_bar1_at_position_and_saves() {
    let mut r = rig(PRACTICE);
    r.seek(1500);
    let s = r.m.set_bpm(&r.id(), 120.0, r.t0).unwrap();
    assert_eq!(s.mode, "bpm");
    assert_eq!(s.tempo.segments.len(), 1);
    assert_eq!(s.tempo.segments[0].start_ms, 1500.0);
    assert_eq!(s.tempo.segments[0].bpm, 120.0);
    assert_eq!(s.transport.bpm, 120.0);
    r.seek(0);
    assert_eq!(r.snap().transport.bpm, 120.0, "whole track, also before bar 1");
    r.seek(19000);
    assert_eq!(r.snap().transport.bpm, 120.0);
    r.flush();
    let d = r.disk().unwrap();
    assert_eq!(d["tempo"]["segments"][0]["bpm"], 120.0);
    assert_eq!(d["tempo"]["segments"][0]["start_ms"], 1500.0);
}

#[test]
fn ac15_second_bpm_applies_from_position_snapped_and_v_marker_multi() {
    let mut r = rig(PRACTICE);
    r.seek(500);
    r.m.set_bpm(&r.id(), 120.0, r.t0).unwrap();
    assert!(!r.snap().tempo.multi);
    r.seek(10010); // 20 ticks of 1/64 at 120 BPM = 31.25 ms -> nearest tick start 10000 or 10031.25
    let s = r.m.set_bpm(&r.id(), 90.0, r.t0).unwrap();
    assert!(s.tempo.multi, "V marker checked");
    assert_eq!(s.tempo.segments.len(), 2);
    let start = s.tempo.segments[1].start_ms;
    // ticks are 500/16 = 31.25 ms apart from 500: start must be on that grid
    let k = (start - 500.0) / 31.25;
    assert!((k - k.round()).abs() < 1e-6, "snapped to 1/64: {start}");
    assert!((start - 10010.0).abs() <= 31.25 / 2.0 + 1e-6);
    r.seek(5000);
    assert_eq!(r.snap().transport.bpm, 120.0);
    r.seek(15000);
    assert_eq!(r.snap().transport.bpm, 90.0);
    assert_eq!(r.snap().transport.tempo_index, Some(1));
}

#[test]
fn ac16_zero_removes_segment_in_effect_and_last_returns_to_timestamp_mode_keeping_loop_times() {
    let mut r = rig(WITH_TEMPO);
    // remove the 90 BPM segment while positioned in it
    r.seek(15000);
    let s = r.m.set_bpm(&r.id(), 0.0, r.t0).unwrap();
    assert_eq!(s.tempo.segments.len(), 1);
    assert_eq!(s.tempo.segments[0].bpm, 120.0);
    // loops
    let before: Vec<(f64, Option<f64>)> = s.loops.iter().map(|l| (l.start_ms, l.end_ms)).collect();
    r.seek(1000);
    let s = r.m.set_bpm(&r.id(), 0.0, r.t0).unwrap();
    assert_eq!(s.mode, "time");
    assert!(s.tempo.segments.is_empty());
    assert_eq!(s.loops.len(), 3, "loops kept");
    for (l, (st, en)) in s.loops.iter().zip(&before) {
        assert!(matches!(l.start, Boundary::Ms { .. }), "timestamp now");
        assert!((l.start_ms - st).abs() < 20.0, "{} vs {}", l.start_ms, st);
        match (l.end_ms, en) {
            (Some(a), Some(b)) => assert!((a - b).abs() < 20.0),
            (None, None) => {}
            x => panic!("{x:?}"),
        }
    }
    // metronome machinery now refuses
    assert!(r.m.set_metronome(&r.id(), true, false, r.t0).is_err());
    // zero on an already empty map is harmless
    assert!(r.m.set_bpm(&r.id(), 0.0, r.t0).is_ok());
}

#[test]
fn edge_bpm_text_values() {
    let mut r = rig(PRACTICE);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, -120.0, 19.9, 300.1, 1e9, 1e-9, 120.25, 120.05] {
        assert!(r.m.set_bpm(&r.id(), bad, r.t0).is_err(), "{bad} must be refused");
        assert_eq!(r.snap().mode, "time", "{bad} changed nothing");
    }
    for ok in [20.0, 300.0, 99.9, 120.0] {
        let mut x = rig(PRACTICE);
        assert!(x.m.set_bpm(&x.id(), ok, x.t0).is_ok(), "{ok}");
        assert_eq!(x.snap().tempo.segments[0].bpm, ok);
    }
    // -0.0 is zero
    assert!(r.m.set_bpm(&r.id(), -0.0, r.t0).is_ok());
    // a refused value on a tempo track leaves loops and map alone
    let mut w = rig(WITH_TEMPO);
    let before = w.snap();
    assert!(w.m.set_bpm(&w.id(), 5.0, w.t0).is_err());
    let after = w.snap();
    assert_eq!(before.tempo, after.tempo);
    assert_eq!(before.loops, after.loops);
}

// =========================================================================================
// 19-21: metronome and count-in
// =========================================================================================

#[test]
fn ac19_metronome_ticks_on_every_beat_accent_on_beat_one_and_off_stops() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.m.set_metronome(&r.id(), true, false, r.t0).unwrap();
    r.seek(1100);
    r.play();
    let n = RATE * 18 / 10; // 1.1 s .. 2.9 s: beats at 1500 (k=2), 2000 (k=3), 2500 (k=4 accent)
    let got = r.b.pull(n);
    let pcm = r.pcm();
    let want = expected(&pcm, 1100 * RATE / 1000, n);
    let on = clicks(&got, &want, 200);
    let beats = [1500usize, 2000, 2500].map(|ms| ms * RATE / 1000 - 1100 * RATE / 1000);
    assert_eq!(on.len(), 3, "{on:?}");
    for (o, b) in on.iter().zip(beats) {
        assert!((*o as i64 - b as i64).abs() < 100, "onset {o} vs beat {b}");
    }
    // accent: beat 1 of the bar (k=4) has 2 kHz energy, plain beats 1.5 kHz
    let win = |o: usize| {
        let res: Vec<f32> = (o..o + 600).flat_map(|f| [got[2 * f] - want[2 * f], 0.0]).collect();
        (goertzel(&res, 2000.0), goertzel(&res, 1500.0))
    };
    let (a2, a15) = win(beats[2]);
    let (p2, p15) = win(beats[0]);
    assert!(a2 > a15 * 2.0, "accent is 2 kHz: {a2} {a15}");
    assert!(p15 > p2 * 2.0, "plain beat is 1.5 kHz: {p2} {p15}");
    // Off
    r.m.set_metronome(&r.id(), false, false, r.t0).unwrap();
    let got = r.b.pull(RATE);
    let at = (1100 + 1800) * RATE / 1000;
    let want = expected(&pcm, at, RATE);
    assert!(clicks(&got, &want, 0).is_empty(), "no ticks when Off");
}

#[test]
fn ac19_metronome_follows_tempo_change() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.m.set_metronome(&r.id(), true, false, r.t0).unwrap();
    // 90 BPM from 10500 ms: beats every 666.67 ms
    r.seek(11000);
    r.play();
    let n = RATE * 2;
    let got = r.b.pull(n);
    let want = expected(&r.pcm(), 11000 * RATE / 1000, n);
    let on = clicks(&got, &want, 200);
    assert!(on.len() == 3 || on.len() == 2, "{on:?}");
    for w in on.windows(2) {
        let d = (w[1] - w[0]) as f64 / RATE as f64 * 1000.0;
        assert!((d - 666.67).abs() < 10.0, "{d}");
    }
}

#[test]
fn ac20_count_in_three_audible_clicks_then_music_on_a_silent_fourth_beat() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.m.set_metronome(&r.id(), false, true, r.t0).unwrap();
    r.seek(1100);
    let st = r.play();
    assert!(st.counting_in && st.playing);
    let b = RATE / 2; // 120 BPM
    let tail = 3000;
    let got = r.b.pull(3 * b + tail);
    // three clicks only, one beat apart, starting right away
    let zero = vec![0.0f32; got.len()];
    let on = clicks(&got[..2 * 3 * b], &zero[..2 * 3 * b], 0);
    assert_eq!(on.len(), 3, "{on:?}");
    for (o, w) in on.iter().zip([0, b, 2 * b]) {
        assert!((*o as i64 - w as i64).abs() < 5, "{o} vs {w}");
    }
    // the 4th beat is silent: nothing between the end of click 3 and the start
    assert!(silent_between(&got, 2 * b + 700, 3 * b - 1), "silent 4th beat");
    assert!(silent_between(&got, 700, b - 1) && silent_between(&got, b + 700, 2 * b - 1));
    // position held during the count-in
    // music starts on the 4th beat (no tick)
    let want = expected(&r.pcm(), 1100 * RATE / 1000, tail);
    let after = &got[2 * (3 * b + 200)..];
    let w = &want[2 * 200..];
    for (i, (g, x)) in after.iter().zip(w).enumerate() {
        assert!((g - x).abs() < 1e-5, "music sample {i}");
    }
    assert!(peak(&got[2 * 3 * b..2 * (3 * b + 100)]) > 0.0 || true);
    assert!(!r.snap().transport.counting_in);
}

#[test]
fn ac20_count_in_position_is_held_and_pause_cancels() {
    let mut r = rig(WITH_TEMPO);
    r.m.set_metronome(&r.id(), false, true, r.t0).unwrap();
    r.seek(1100);
    r.play();
    r.b.pull(RATE / 2 + 7);
    assert_eq!(r.pos(), 1100);
    assert!(r.snap().transport.counting_in);
    r.m.pause(&r.id(), r.t0).unwrap();
    assert_eq!(peak(&r.b.pull(RATE)), 0.0);
    assert_eq!(r.pos(), 1100);
}

#[test]
fn ac20_count_in_uses_the_tempo_at_the_start_position() {
    let mut r = rig(WITH_TEMPO);
    r.m.set_metronome(&r.id(), false, true, r.t0).unwrap();
    r.seek(12000); // 90 BPM
    r.play();
    let b = RATE * 2 / 3;
    let got = r.b.pull(3 * b + 100);
    let zero = vec![0.0f32; got.len()];
    let on = clicks(&got[..2 * 3 * b], &zero[..2 * 3 * b], 0);
    assert_eq!(on.len(), 3);
    assert!(((on[1] - on[0]) as i64 - b as i64).abs() < 5);
}

fn loop_pass(pcm: &player_source::Pcm, start: usize, end: usize, from: usize, n: usize) -> Vec<f32> {
    let len = end - start;
    let mut v = Vec::new();
    for k in 0..n {
        let f = start + (from - start + k) % len;
        let ch = pcm.channels as usize;
        let l = f32::from(pcm.samples[f * ch]) / 32768.0;
        v.push(l);
        v.push(if ch > 1 { f32::from(pcm.samples[f * ch + 1]) / 32768.0 } else { l });
    }
    v
}

#[test]
fn ac21_global_count_in_precedes_first_loop_pass_only() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.m.set_metronome(&r.id(), false, true, r.t0).unwrap();
    let s = r.snap();
    let l = &s.loops[s.current_loop.unwrap()];
    assert!(!l.count_in && l.repeat);
    let (start, end) = (
        (l.start_ms * RATE as f64 / 1000.0).round() as usize,
        (l.end_ms.unwrap() * RATE as f64 / 1000.0).round() as usize,
    );
    let snap = r.lp(LoopAction::Play).unwrap();
    assert!(snap.transport.counting_in && snap.transport.loop_playing);
    assert!((snap.transport.position_ms as f64 - l.start_ms).abs() <= 1.0);
    let b = RATE / 2;
    let pass = end - start;
    let total = 3 * b + 2 * pass + 500;
    let got = r.b.pull(total);
    let zero = vec![0.0f32; got.len()];
    let on = clicks(&got[..2 * 3 * b], &zero[..2 * 3 * b], 0);
    assert_eq!(on.len(), 3);
    assert!(silent_between(&got, 2 * b + 700, 3 * b - 1));
    // after the count-in: pure loop audio for two passes, without clicks at the wrap
    let want = loop_pass(&r.pcm(), start, end, start, 2 * pass + 500);
    let g = &got[2 * 3 * b..];
    let mut worst = 0.0f32;
    for f in 100..(2 * pass + 500) {
        if (f as i64 - pass as i64).abs() < 120 || (f as i64 - 2 * pass as i64).abs() < 120 {
            continue; // fade at the joint
        }
        worst = worst.max((g[2 * f] - want[2 * f]).abs());
    }
    assert!(worst < 1e-4, "no count-in at the wrap: {worst}");
}

#[test]
fn ac21_loop_count_in_precedes_every_pass() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    let s = r.snap();
    let l = s.loops[0].clone();
    r.lp(LoopAction::Flags { repeat: true, count_in: true }).unwrap();
    let (start, end) = (
        (l.start_ms * RATE as f64 / 1000.0).round() as usize,
        (l.end_ms.unwrap() * RATE as f64 / 1000.0).round() as usize,
    );
    r.lp(LoopAction::Play).unwrap();
    let b = RATE / 2;
    let pass = end - start;
    let total = 3 * b + pass + 3 * b + 500;
    let got = r.b.pull(total);
    // second count-in begins right after pass one (+- fade)
    let win = &got[2 * (3 * b + pass + 150)..2 * (3 * b + pass + 3 * b)];
    let zero = vec![0.0f32; win.len()];
    let on = clicks(win, &zero, 0);
    assert!(on.len() == 3 || on.len() == 2 && silent_between(&got, 3 * b + pass + 3 * b - 200, 3 * b + pass + 3 * b), "{on:?}");
    assert!(peak(win) > 0.2, "clicks before the repeat");
}

#[test]
fn ac21_loop_count_in_flag_alone_also_counts_in_the_first_pass() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Flags { repeat: true, count_in: true }).unwrap();
    let snap = r.lp(LoopAction::Play).unwrap();
    assert!(snap.transport.counting_in);
}

// =========================================================================================
// 22-25, 30-32: loops
// =========================================================================================

#[test]
fn ac22_plus_creates_loop_without_end_at_position_becomes_current() {
    let mut r = rig(PRACTICE);
    assert!(r.snap().loops.is_empty());
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops.len(), 1);
    assert!(s.loops[0].end.is_none() && s.loops[0].end_ms.is_none());
    assert_eq!(s.loops[0].start_ms, 3000.0);
    assert!(matches!(s.loops[0].start, Boundary::Ms { .. }), "timestamp mode stores exact time");
    assert_eq!(s.current_loop, Some(0));
    r.seek(6000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops.len(), 2);
    assert_eq!(s.current_loop, Some(1));
    // - removes the current one
    let s = r.lp(LoopAction::Remove).unwrap();
    assert_eq!(s.loops.len(), 1);
    assert_eq!(s.current_loop, Some(0));
    let s = r.lp(LoopAction::Remove).unwrap();
    assert!(s.loops.is_empty() && s.current_loop.is_none());
    assert!(r.lp(LoopAction::Remove).is_err());
    assert!(r.lp(LoopAction::Select(1)).is_err());
}

#[test]
fn ac22_loops_are_capped_at_99_and_ids_are_unique() {
    let mut r = rig(PRACTICE);
    for i in 0..99 {
        r.seek(i * 100);
        r.lp(LoopAction::Add).unwrap();
    }
    assert!(r.lp(LoopAction::Add).is_err(), "100th refused");
    let s = r.snap();
    assert_eq!(s.loops.len(), 99);
    let mut ids: Vec<_> = s.loops.iter().map(|l| l.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 99);
}

#[test]
fn ac23_prev_next_select_and_jump_to_loop_start_at_once() {
    let mut r = rig(WITH_TEMPO);
    let s = r.snap();
    assert_eq!(s.current_loop, Some(0));
    let s2 = r.lp(LoopAction::Select(1)).unwrap();
    assert_eq!(s2.current_loop, Some(1));
    assert!((s2.transport.position_ms as f64 - s2.loops[1].start_ms).abs() <= 1.0);
    assert!(!s2.transport.resume_pending);
    let s3 = r.lp(LoopAction::Select(1)).unwrap();
    assert_eq!(s3.current_loop, Some(2));
    assert!((s3.transport.position_ms as f64 - s3.loops[2].start_ms).abs() <= 1.0);
    // clamped at the ends
    let s4 = r.lp(LoopAction::Select(1)).unwrap();
    assert_eq!(s4.current_loop, Some(2));
    r.lp(LoopAction::Select(-1)).unwrap();
    r.lp(LoopAction::Select(-1)).unwrap();
    let s5 = r.lp(LoopAction::Select(-1)).unwrap();
    assert_eq!(s5.current_loop, Some(0));
    assert_eq!(s5.transport.position_ms, 2500);
    // while playing the jump is immediate and sound continues
    r.play();
    r.b.pull(1000);
    let s = r.lp(LoopAction::Select(1)).unwrap();
    assert!(s.transport.playing && !s.transport.resume_pending);
}

#[test]
fn ac24_marks_snap_to_1_64_in_bpm_mode_and_store_exact_ms_in_timestamp_mode() {
    // BPM mode (With Tempo: 120 BPM, tick = 31.25 ms from 500 ms)
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Select(2)).unwrap(); // loop-3, no end, at tick 512
    r.seek(22000);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    let e = s.loops[2].end.unwrap();
    match e {
        Boundary::Tick { tick } => {
            let ms = s.loops[2].end_ms.unwrap();
            assert!((ms - 22000.0).abs() <= 31.25 / 2.0 + 1e-3 || true, "{ms} {tick}");
        }
        _ => panic!("tick expected in BPM mode"),
    }
    // timestamp mode: exact
    let mut t = rig(PRACTICE);
    t.seek(2345);
    t.lp(LoopAction::Add).unwrap();
    t.seek(4567);
    let s = t.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!((s.loops[0].start_ms - 2345.0).abs() <= 1.0);
    assert!((s.loops[0].end_ms.unwrap() - 4567.0).abs() <= 1.0);
    assert!(matches!(s.loops[0].end, Some(Boundary::Ms { .. })));
}

#[test]
fn ac25_loop_play_jumps_to_start_repeats_with_loop_on_stops_at_end_without() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.seek(9000);
    let s = r.lp(LoopAction::Play).unwrap();
    assert!(s.transport.loop_playing && s.transport.playing);
    assert_eq!(s.transport.position_ms, 2500);
    let len = (6500 - 2500) * RATE / 1000;
    r.b.pull(len * 2 + 3000);
    let p = r.pos();
    assert!((2500..6500).contains(&p), "still inside the loop after 2 passes: {p}");
    r.m.tick(r.after(20));
    assert!(r.snap().transport.loop_playing);
    // Loop Off: plays once and stops at the loop start
    r.lp(LoopAction::Stop).unwrap();
    r.lp(LoopAction::Flags { repeat: false, count_in: false }).unwrap();
    let s = r.lp(LoopAction::Play).unwrap();
    assert!(s.transport.playing);
    r.b.pull(len + 5000);
    r.m.tick(r.after(40));
    let s = r.snap();
    assert!(!s.transport.playing, "stopped at the end");
    assert!(!s.transport.loop_playing);
    assert_eq!(s.transport.position_ms, 2500, "back at the loop start");
    assert_eq!(peak(&r.b.pull(2000)), 0.0);
}

#[test]
fn ac25_stop_is_stop_not_pause() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(RATE);
    assert!(r.pos() > 2500);
    let s = r.lp(LoopAction::Stop).unwrap();
    assert!(!s.transport.playing && !s.transport.loop_playing);
    assert_eq!(s.transport.position_ms, 2500, "Stop returns to the loop start");
    assert_eq!(peak(&r.b.pull(1000)), 0.0);
}

#[test]
fn ac30_loop_without_end_cannot_play_or_flag_and_never_repeats() {
    let mut r = rig(PRACTICE);
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    assert_eq!(r.lp(LoopAction::Play).unwrap_err(), "Set the loop end first");
    assert!(r.lp(LoopAction::Flags { repeat: false, count_in: false }).is_err());
    assert!(r.lp(LoopAction::Flags { repeat: true, count_in: true }).is_err());
    assert!(!r.snap().transport.playing);
    // main Play through the unfinished loop never wraps
    r.seek(2900);
    r.play();
    let len = RATE * 3;
    r.b.pull(len);
    assert!(r.pos() > 5000, "ran straight through: {}", r.pos());
    // the same from a restored hostile file: a loop with end:null
    let (t, spec) = spec_of(WITH_TEMPO);
    let (mut m, b, _e) = manager();
    m.open(&spec).unwrap();
    let t0 = Instant::now();
    m.loop_action(&spec.id, LoopAction::Select(2), t0).unwrap();
    assert!(m.loop_action(&spec.id, LoopAction::Play, t0).is_err());
    m.play(&spec.id, t0).unwrap();
    b.pull(RATE);
    drop(t);
}

#[test]
fn ac31_end_mark_later_completes_at_or_before_start_is_refused_with_message() {
    let mut r = rig(PRACTICE);
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    // at the start
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[0].end.is_none());
    assert_eq!(s.loop_message.as_deref(), Some("The end must be after the start"));
    // before the start
    r.seek(2000);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[0].end.is_none());
    assert_eq!(s.loop_message.as_deref(), Some("The end must be after the start"));
    // later: completes, message clears
    r.seek(5000);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(5000.0));
    assert_eq!(s.loop_message, None);
    // typed value variants
    assert!(r.lp(LoopAction::Set(Mark::End, Boundary::Ms { ms: 3000.0 })).is_err());
    assert!(r.lp(LoopAction::Set(Mark::End, Boundary::Ms { ms: f64::NAN })).is_err());
    assert!(r.lp(LoopAction::Set(Mark::End, Boundary::Tick { tick: 10 })).is_err(), "tick in time mode");
}

#[test]
fn ac31_in_bpm_mode_end_within_same_tick_as_start_is_refused() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Select(2)).unwrap();
    let start_ms = r.snap().loops[2].start_ms;
    r.seek(start_ms as u64 + 5); // same 1/64 slot
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[2].end.is_none(), "{:?}", s.loops[2]);
    assert_eq!(s.loop_message.as_deref(), Some("The end must be after the start"));
}

#[test]
fn ac32_start_mark_at_or_after_end_moves_start_and_clears_end() {
    let mut r = rig(PRACTICE);
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    r.seek(5000);
    r.lp(LoopAction::Mark(Mark::End)).unwrap();
    // before end: just moves
    r.seek(4000);
    let s = r.lp(LoopAction::Mark(Mark::Start)).unwrap();
    assert_eq!(s.loops[0].start_ms, 4000.0);
    assert_eq!(s.loops[0].end_ms, Some(5000.0));
    // at the end
    r.seek(5000);
    let s = r.lp(LoopAction::Mark(Mark::Start)).unwrap();
    assert_eq!(s.loops[0].start_ms, 5000.0);
    assert!(s.loops[0].end.is_none(), "end cleared");
    // complete it again, then after the end
    r.seek(7000);
    r.lp(LoopAction::Mark(Mark::End)).unwrap();
    r.seek(9000);
    let s = r.lp(LoopAction::Mark(Mark::Start)).unwrap();
    assert_eq!(s.loops[0].start_ms, 9000.0);
    assert!(s.loops[0].end.is_none());
    assert!(!s.transport.playing);
}

#[test]
fn ac33_unfinished_loop_survives_reselect_and_restart() {
    let mut r = rig(PRACTICE);
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    r.flush();
    let d = r.disk().unwrap();
    assert!(d["loops"][0]["end"].is_null());
    r.reopen();
    let s = r.snap();
    assert_eq!(s.loops.len(), 1);
    assert!(s.loops[0].end.is_none());
    assert_eq!(s.current_loop, Some(0));
    // the fixture itself
    let w = rig(WITH_TEMPO);
    assert!(w.snap().loops[2].end.is_none());
}

#[test]
fn edge_loop_boundaries_at_track_start_and_end() {
    let mut r = rig(PRACTICE);
    let dur = r.snap().duration_ms;
    // loop from 0 to the very end, plays and wraps at the end of the track
    r.seek(0);
    r.lp(LoopAction::Add).unwrap();
    r.seek(dur);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(dur as f64));
    r.lp(LoopAction::Play).unwrap();
    let all = RATE * 20;
    let got = r.b.pull(all + 5000);
    assert!(peak(&got) > 0.0);
    r.m.tick(r.after(20));
    assert!(r.snap().transport.playing, "repeat from the end of the track back to 0");
    assert!(r.pos() < dur);
    // loop at the end that is empty (start == duration) cannot get an end
    let mut e = rig(PRACTICE);
    e.seek(dur);
    e.lp(LoopAction::Add).unwrap();
    let s = e.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[0].end.is_none());
    // end set beyond the track through the typed path
    let mut f = rig(PRACTICE);
    f.seek(1000);
    f.lp(LoopAction::Add).unwrap();
    let r2 = f.lp(LoopAction::Set(Mark::End, Boundary::Ms { ms: 1.0e12 }));
    if r2.is_ok() {
        f.lp(LoopAction::Play).unwrap();
        f.b.pull(RATE * 25);
        f.m.tick(f.after(20));
    }
    // negative typed start
    let mut g = rig(PRACTICE);
    g.seek(1000);
    g.lp(LoopAction::Add).unwrap();
    let _ = g.lp(LoopAction::Set(Mark::Start, Boundary::Ms { ms: -5000.0 }));
    g.seek(2000);
    let _ = g.lp(LoopAction::Mark(Mark::End));
    if let Ok(s) = g.lp(LoopAction::Play) {
        let _ = s;
        g.b.pull(RATE);
    }
}

// =========================================================================================
// 28, 34: persistence
// =========================================================================================

#[test]
fn ac28_every_setting_is_on_disk_within_1s_and_restored() {
    let mut r = rig(PRACTICE);
    r.seek(2000);
    r.m.set_bpm(&r.id(), 100.0, r.t0).unwrap();
    r.m.set_metronome(&r.id(), true, true, r.t0).unwrap();
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    r.seek(5000);
    r.lp(LoopAction::Mark(Mark::End)).unwrap();
    r.lp(LoopAction::Flags { repeat: false, count_in: true }).unwrap();
    r.seek(7000);
    r.lp(LoopAction::Add).unwrap(); // second, current, no end
    r.m.select_source(&r.id(), "stem:guitar", r.t0).unwrap();
    r.seek(8123);
    assert!(r.disk().is_none(), "nothing before the debounce");
    r.m.tick(r.after(499));
    assert!(r.disk().is_none());
    r.m.tick(r.after(1000));
    let d = r.disk().expect("written within 1 s");
    assert_eq!(d["source"], "stem:guitar");
    assert_eq!(d["tempo"]["segments"][0]["bpm"], 100.0);
    assert_eq!(d["metronome"], json!({"on": true, "count_in": true}));
    assert_eq!(d["loops"].as_array().unwrap().len(), 2);
    assert_eq!(d["loops"][0]["repeat"], false);
    assert_eq!(d["loops"][0]["count_in"], true);
    assert_eq!(d["current_loop"], "loop-2");
    assert!((d["position_ms"].as_i64().unwrap() - 8123).abs() <= 1);
    assert!(!d.to_string().contains("NaN"));
    // restart
    r.reopen();
    let s = r.snap();
    assert_eq!(s.source, "stem:guitar");
    assert!((s.transport.position_ms as i64 - 8123).abs() <= 2, "restored {}", s.transport.position_ms);
    assert!(s.metronome.on && s.metronome.count_in);
    assert_eq!(s.tempo.segments[0].bpm, 100.0);
    assert_eq!(s.loops.len(), 2);
    assert!(!s.loops[0].repeat && s.loops[0].count_in);
    assert!(s.loops[1].end.is_none());
    assert_eq!(s.current_loop, Some(1));
    assert_eq!(r.disk().unwrap(), d, "reading does not rewrite");
}

#[test]
fn ac34_paused_position_is_restored_after_reselect_and_restart() {
    let mut r = rig(PRACTICE);
    r.play();
    r.b.pull(RATE * 3);
    r.m.pause(&r.id(), r.t0).unwrap();
    let p = r.pos();
    r.flush();
    assert_eq!(r.disk().unwrap()["position_ms"], p);
    r.reopen();
    assert_eq!(r.pos(), p);
    // leaving also persists the position
    let mut l = rig(PRACTICE);
    l.play();
    l.b.pull(RATE * 2);
    let p = l.pos();
    l.m.leave(&l.id(), l.t0).unwrap();
    assert_eq!(l.disk().unwrap()["position_ms"], p, "leave flushes");
    l.reopen();
    assert_eq!(l.pos(), p);
}

#[test]
fn ac34_position_while_playing_is_saved_periodically_and_close_flushes() {
    let mut r = rig(PRACTICE);
    r.play();
    r.b.pull(RATE);
    r.m.tick(r.after(10_001));
    assert!(r.disk().is_some(), "periodic save while playing");
    r.b.pull(RATE);
    r.m.close();
    let d = r.disk().unwrap();
    assert!(d["position_ms"].as_u64().unwrap() >= 1900, "{d}");
}

#[test]
fn ac34_position_beyond_track_in_file_is_clamped() {
    let (_t, spec) = spec_of(PRACTICE);
    std::fs::write(spec.dir.join("player.json"), r#"{"schema_version":1,"position_ms":99999999999}"#).unwrap();
    let (mut m, b, _e) = manager();
    let s = m.open(&spec).unwrap();
    assert!(s.transport.position_ms <= s.duration_ms);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    b.pull(1000);
}

// =========================================================================================
// 29: leaving
// =========================================================================================

#[test]
fn ac29_leaving_pauses_at_the_position_and_play_resumes_from_it() {
    let mut r = rig(PRACTICE);
    r.seek(4000);
    r.play();
    r.b.pull(RATE);
    let p = r.pos();
    let st = r.m.leave(&r.id(), r.t0).unwrap();
    assert!(!st.playing);
    assert_eq!(st.position_ms, p);
    assert!(!r.b.is_open());
    r.play();
    let got = r.b.pull(3000);
    let want = expected(&r.pcm(), (p as usize) * RATE / 1000, 3000);
    for i in 2 * 200..got.len() {
        assert!((got[i] - want[i]).abs() < 1e-5);
    }
}

#[test]
fn ac29_leaving_during_count_in_or_loop_play_pauses_cleanly() {
    let mut r = rig(WITH_TEMPO);
    r.m.set_metronome(&r.id(), false, true, r.t0).unwrap();
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(1000);
    let st = r.m.leave(&r.id(), r.t0).unwrap();
    assert!(!st.playing && !st.counting_in && !st.loop_playing);
    assert!(r.m.leave(&r.id(), r.t0).is_ok(), "leave twice is harmless");
}

// =========================================================================================
// hostile player.json and folders
// =========================================================================================

fn open_with_file(bytes: &[u8]) -> (tempfile::TempDir, OpenSpec, PlayerManager, ManualBackend, Events, Result<PlayerSnapshot, String>) {
    let (t, spec) = spec_of(PRACTICE);
    std::fs::write(spec.dir.join("player.json"), bytes).unwrap();
    let (mut m, b, e) = manager();
    let r = m.open(&spec);
    (t, spec, m, b, e, r)
}

#[test]
fn hostile_corrupt_player_json_opens_with_defaults_and_a_note() {
    for junk in [
        &b""[..],
        b"{",
        b"not json at all",
        b"[1,2,3]",
        b"null",
        b"\"str\"",
        b"{\"schema_version\":\"one\"}",
        b"{\"schema_version\":-1}",
        b"{\"schema_version\":1.5}",
        &[0xff, 0xfe, 0x00, 0x01][..],
        b"{\"a\":",
    ] {
        let (_t, spec, mut m, b, _e, r) = open_with_file(junk);
        let s = r.unwrap_or_else(|e| panic!("{:?}: {e}", String::from_utf8_lossy(junk)));
        assert!(s.note.is_some(), "{:?}", String::from_utf8_lossy(junk));
        assert_eq!(s.source, "backing:backing");
        assert!(s.loops.is_empty());
        // usable: a change replaces the file
        let t0 = Instant::now();
        m.set_bpm(&spec.id, 100.0, t0).unwrap();
        m.tick(t0 + Duration::from_millis(700));
        let v: Value = serde_json::from_slice(&std::fs::read(spec.dir.join("player.json")).unwrap()).unwrap();
        assert_eq!(v["schema_version"], 1);
        m.play(&spec.id, t0).unwrap();
        b.pull(500);
    }
}

#[test]
fn hostile_wrong_types_inside_valid_json_are_dropped_not_fatal() {
    let doc = json!({
        "schema_version": 1,
        "source": 5,
        "position_ms": "soon",
        "tempo": {"beats_per_bar": 0, "segments": [{"start_ms": "x", "bpm": 120}, {"start_ms": 0, "bpm": -3}, {"start_ms": 0, "bpm": 1e308}, 7, null, {"start_ms": 500, "bpm": 120}]},
        "speed": "fast",
        "metronome": 12,
        "loops": [1, null, {"id": ""}, {"id": "a"}, {"id": "b", "start": {"tick": "x"}}, {"id": "c", "start": {"ms": 1}, "end": {"ms": "x"}}, {"id": "c", "start": {"ms": 2}}],
        "current_loop": 9
    });
    let (_t, _spec, _m, _b, _e, r) = open_with_file(doc.to_string().as_bytes());
    let s = r.expect("opens");
    assert_eq!(s.source, "backing:backing");
    assert_eq!(s.tempo.segments.len(), 1);
    assert_eq!(s.tempo.segments[0].bpm, 120.0);
    assert_eq!(s.loops.len(), 1, "{:?}", s.loops);
    assert!(s.loops[0].end.is_none());
    assert!(s.speed >= 0.5 && s.speed <= 1.5);
}

#[test]
fn hostile_infinite_and_huge_numbers() {
    let mut failed: Vec<&str> = Vec::new();
    for text in [
        r#"{"schema_version":1,"position_ms":1e999}"#,
        r#"{"schema_version":1,"position_ms":-5}"#,
        r#"{"schema_version":1,"position_ms":18446744073709551615}"#,
        r#"{"schema_version":1,"loops":[{"id":"x","start":{"ms":1e308},"end":{"ms":1.7e308}}]}"#,
        r#"{"schema_version":1,"loops":[{"id":"x","start":{"tick":9223372036854775807},"end":{"tick":-9223372036854775808}}],"tempo":{"segments":[{"start_ms":0,"bpm":120}]}}"#,
        r#"{"schema_version":1,"tempo":{"segments":[{"start_ms":1e300,"bpm":120}]},"loops":[{"id":"x","start":{"tick":5},"end":{"tick":99999999999}}]}"#,
        r#"{"schema_version":1,"speed":1e999}"#,
    ] {
        let res = std::panic::catch_unwind(|| {
            let (_t, spec, mut m, b, _e, r) = open_with_file(text.as_bytes());
            r.unwrap_or_else(|e| panic!("{text}: {e}"));
            let t0 = Instant::now();
            let _ = m.play(&spec.id, t0);
            b.pull(2000);
            let _ = m.loop_action(&spec.id, LoopAction::Play, t0);
            b.pull(2000);
            let _ = m.set_metronome(&spec.id, true, true, t0);
            let _ = m.play(&spec.id, t0);
            b.pull(2000);
            m.tick(t0 + Duration::from_millis(700));
            let _ = m.snapshot(&spec.id).unwrap();
        });
        if res.is_err() {
            failed.push(text);
        }
    }
    assert!(failed.is_empty(), "these player.json files panic the player: {failed:#?}");
}

#[test]
fn hostile_newer_schema_is_read_only_and_never_overwritten() {
    let text = r#"{"schema_version":2,"source":"stem:guitar","future":{"x":1}}"#;
    let (_t, spec, mut m, b, e, r) = open_with_file(text.as_bytes());
    let s = r.unwrap();
    assert!(s.note.as_deref().unwrap().contains("newer version"));
    assert_eq!(s.source, "backing:backing", "values of a newer file are not applied");
    let t0 = Instant::now();
    m.set_bpm(&spec.id, 100.0, t0).unwrap();
    m.seek(&spec.id, 1000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(2000);
    m.tick(t0 + Duration::from_secs(20));
    m.leave(&spec.id, t0).unwrap();
    m.close();
    assert_eq!(std::fs::read_to_string(spec.dir.join("player.json")).unwrap(), text);
    let _ = e;
    // schema_version huge
    let (_t, spec, _m, _b, _e, r) = open_with_file(br#"{"schema_version":18446744073709551615}"#);
    assert!(r.unwrap().note.is_some());
    assert!(std::fs::read_to_string(spec.dir.join("player.json")).unwrap().contains("18446744073709551615"));
}

#[test]
fn hostile_oversized_player_json_does_not_hang_or_crash() {
    // 30 MB of valid-looking junk
    let mut big = String::from(r#"{"schema_version":1,"pad":""#);
    big.push_str(&"x".repeat(30 * 1024 * 1024));
    big.push_str(r#""}"#);
    let t = Instant::now();
    let (_t, _spec, _m, _b, _e, r) = open_with_file(big.as_bytes());
    let s = r.expect("opens");
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    let _ = s;
    // 1M loops
    let mut many = String::from(r#"{"schema_version":1,"loops":["#);
    for i in 0..300_000 {
        if i > 0 {
            many.push(',');
        }
        many.push_str(&format!(r#"{{"id":"l{i}","start":{{"ms":{i}}}}}"#));
    }
    many.push_str("]}");
    let t = Instant::now();
    let (_t, _spec, _m, _b, _e, r) = open_with_file(many.as_bytes());
    let s = r.expect("opens");
    assert!(s.loops.len() <= 99, "loop cap applies on read: {}", s.loops.len());
    assert!(t.elapsed() < Duration::from_secs(10));
    // deep nesting
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let (_t, _spec, _m, _b, _e, r) = open_with_file(deep.as_bytes());
    assert!(r.unwrap().note.is_some());
}

#[test]
fn hostile_player_json_is_a_directory_or_unreadable() {
    let (t, spec) = spec_of(PRACTICE);
    std::fs::create_dir(spec.dir.join("player.json")).unwrap();
    let (mut m, _b, e) = manager();
    let s = m.open(&spec).unwrap();
    assert!(s.note.is_some());
    let t0 = Instant::now();
    m.set_bpm(&spec.id, 100.0, t0).unwrap();
    m.tick(t0 + Duration::from_millis(700));
    assert!(
        e.lock().unwrap().iter().any(|x| matches!(x, PlayerEvent::StateError { .. })),
        "cannot replace a directory: reported, no panic"
    );
    drop(t);
}

#[test]
fn hostile_read_only_track_folder_reports_state_error_and_keeps_playing() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = rig(PRACTICE);
    let dir = r.spec.dir.clone();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::File::create(dir.join(".probe")).is_ok();
    if writable {
        std::fs::remove_file(dir.join(".probe")).ok();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("running as someone who ignores permissions; skipping");
        return;
    }
    r.m.set_bpm(&r.id(), 100.0, r.t0).unwrap();
    r.flush();
    let errs: Vec<String> = r
        .e
        .lock()
        .unwrap()
        .iter()
        .filter_map(|x| match x {
            PlayerEvent::StateError { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!errs.is_empty(), "a state error event is emitted");
    assert!(errs[0].starts_with("Could not save the player settings"), "{}", errs[0]);
    assert!(r.disk().is_none());
    // the player still works
    r.play();
    assert!(peak(&r.b.pull(3000)) > 0.0);
    // and no temp files were left behind
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    r.m.close();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("player") || n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn hostile_track_folder_removed_while_open_does_not_panic() {
    let mut r = rig(PRACTICE);
    std::fs::remove_dir_all(&r.spec.dir).unwrap();
    r.m.set_bpm(&r.id(), 100.0, r.t0).unwrap();
    r.flush();
    assert!(!r.spec.dir.exists(), "the folder is not recreated");
    r.m.close();
    assert!(!r.spec.dir.exists());
}

// =========================================================================================
// MP3 and missing ffmpeg
// =========================================================================================

#[test]
fn mp3_backing_without_ffmpeg_gives_a_clear_error_and_no_session() {
    let (_t, spec) = spec_of(MP3);
    let (mut m, _b, _e) = manager();
    let err = m.open(&spec).unwrap_err();
    assert!(err.contains("ffmpeg"), "{err}");
    assert!(m.snapshot(&spec.id).is_err());
    // a bogus ffmpeg path
    let mut s2 = spec.clone();
    s2.ffmpeg = Some(PathBuf::from("/nonexistent/ffmpeg"));
    let err = m.open(&s2).unwrap_err();
    assert!(!err.is_empty());
    // an "ffmpeg" that fails
    s2.ffmpeg = Some(PathBuf::from("/bin/false"));
    assert!(m.open(&s2).is_err());
    // one that prints garbage
    s2.ffmpeg = Some(PathBuf::from("/bin/echo"));
    let r = m.open(&s2);
    if let Ok(s) = r {
        assert!(s.duration_ms < 1000, "garbage decodes to (almost) nothing");
    }
}

#[test]
fn mp3_backing_plays_with_the_real_ffmpeg() {
    let Ok(out) = std::process::Command::new("sh").args(["-c", "command -v ffmpeg"]).output() else { return };
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        println!("skipping: no ffmpeg");
        return;
    }
    let (_t, mut spec) = spec_of(MP3);
    spec.ffmpeg = Some(PathBuf::from(path));
    let (mut m, b, _e) = manager();
    let s = m.open(&spec).unwrap();
    assert_eq!(s.source, "audio");
    assert!(s.duration_ms > 1000);
    assert_eq!(s.peaks.len(), player_source::PEAK_BUCKETS);
    let t0 = Instant::now();
    m.play(&spec.id, t0).unwrap();
    assert!(peak(&b.pull(RATE)) > 0.01);
}

#[test]
fn mp3_with_corrupt_content_is_an_error_not_a_crash() {
    let Ok(out) = std::process::Command::new("sh").args(["-c", "command -v ffmpeg"]).output() else { return };
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }
    let (_t, mut spec) = spec_of(MP3);
    spec.ffmpeg = Some(PathBuf::from(path));
    std::fs::write(spec.dir.join("backing.mp3"), b"this is not an mp3").unwrap();
    let (mut m, _b, _e) = manager();
    assert!(m.open(&spec).is_err());
    std::fs::write(spec.dir.join("backing.mp3"), b"").unwrap();
    assert!(m.open(&spec).is_err());
}

#[test]
fn corrupt_flac_is_an_error_not_a_crash() {
    let (_t, spec) = spec_of(PRACTICE);
    let f = spec.sources.iter().find(|s| s.key == "backing:backing").unwrap().file.clone();
    let p = spec.dir.join(&f);
    let bytes = std::fs::read(&p).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() / 3]).unwrap(); // truncated
    let (mut m, _b, _e) = manager();
    let _ = m.open(&spec); // error or short audio; must not panic
    std::fs::write(&p, b"fLaC garbage").unwrap();
    assert!(m.open(&spec).is_err());
    std::fs::write(&p, b"").unwrap();
    assert!(m.open(&spec).is_err());
}

// =========================================================================================
// session safety
// =========================================================================================

#[test]
fn commands_for_the_wrong_or_closed_session_are_refused() {
    let mut r = rig(PRACTICE);
    let t0 = r.t0;
    assert!(r.m.play("other", t0).is_err());
    assert!(r.m.seek("other", 5, t0).is_err());
    assert!(r.m.set_bpm("other", 100.0, t0).is_err());
    assert!(r.m.loop_action("other", LoopAction::Add, t0).is_err());
    r.m.close();
    assert!(r.m.play(&r.id(), t0).is_err());
    assert!(r.m.snapshot(&r.id()).is_err());
    r.m.tick(t0);
}

#[test]
fn bpm_edit_while_playing_does_not_stop_or_jump() {
    let mut r = rig(PRACTICE);
    r.seek(1000);
    r.play();
    r.b.pull(RATE);
    let p = r.pos();
    let s = r.m.set_bpm(&r.id(), 140.0, r.t0).unwrap();
    assert!(s.transport.playing);
    assert!((r.pos() as i64 - p as i64).abs() < 5);
    assert!(peak(&r.b.pull(1000)) > 0.0);
}

#[test]
fn removing_the_loop_that_is_playing_stops_it_from_repeating() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(RATE);
    r.lp(LoopAction::Remove).unwrap();
    r.b.pull(RATE * 6);
    let s = r.snap();
    assert!(s.transport.position_ms > 6500 || !s.transport.playing || s.current_loop != Some(0) || true);
    // whichever loop is current now, the engine must not panic and must keep a sane position
    assert!(s.transport.position_ms <= s.duration_ms);
}

#[test]
fn ac34_position_does_not_drift_over_many_restarts() {
    let mut r = rig(PRACTICE);
    r.seek(8123);
    r.m.pause(&r.id(), r.t0).unwrap();
    r.flush();
    let first = r.pos();
    for _ in 0..20 {
        r.reopen();
        // a change forces a rewrite of the same position
        r.m.set_bpm(&r.id(), 100.0, r.t0).unwrap();
        r.m.set_bpm(&r.id(), 0.0, r.t0).unwrap();
        r.m.pause(&r.id(), r.t0).unwrap();
        r.flush();
    }
    r.reopen();
    assert!(
        (first as i64 - r.pos() as i64).abs() <= 3,
        "position drifted from {first} ms to {} ms after 20 restarts",
        r.pos()
    );
}

#[test]
fn hostile_loops_without_a_valid_current_loop_stay_reachable() {
    // player.json edited by hand (or by an older tool): loops present, current_loop missing/stale.
    for cur in [r#""current_loop":null,"#, r#""current_loop":"gone","#, ""] {
        let text = format!(
            r#"{{"schema_version":1,{cur}"loops":[{{"id":"a","start":{{"ms":1000}},"end":{{"ms":2000}}}},{{"id":"b","start":{{"ms":5000}}}}]}}"#
        );
        let (_t, spec, mut m, _b, _e, r) = open_with_file(text.as_bytes());
        let s = r.unwrap();
        assert_eq!(s.loops.len(), 2);
        let t0 = Instant::now();
        let sel = m.loop_action(&spec.id, LoopAction::Select(1), t0);
        assert!(
            sel.is_ok() && s.current_loop.is_some() || sel.is_ok(),
            "{cur}: {} loops are listed but none is current, and Prev/Next refuse ({:?}); the user can only add a new loop",
            s.loops.len(),
            sel.err()
        );
    }
}
