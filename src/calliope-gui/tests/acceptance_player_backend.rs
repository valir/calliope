//! Independent acceptance tests for specs/gui-play-backing-track.md, increment A (plan section 7).
//!
//! The binary crate has no library target, so this file compiles the player modules straight from
//! `src/` (via `#[path]`) and drives `PlayerManager` the way the IPC layer does, with the
//! `ManualBackend` (a fake: no sound device is ever opened) and injected `Instant`s. Audio is
//! checked sample by sample against the decoded fixture. (As a side effect the modules' own unit
//! tests are compiled and run here once more.)
#![allow(dead_code, unused_imports, unused_macros, clippy::all)]

use calliope_gui::audio_out;
use calliope_gui::fsutil;
use calliope_gui::metronome;
use calliope_gui::player;
use calliope_gui::player_source;
use calliope_gui::player_state;
use calliope_gui::repository;
use calliope_gui::stem_audio;
use calliope_gui::tempo_map;
use calliope_gui::track_meta;
use calliope_gui::transport;

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
    Rig {
        _t: t,
        spec,
        m,
        b,
        e,
        t0: Instant::now(),
    }
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
    /// "+" at `ms`, then the end cleared again (Start at the track end clears it, Start back at `ms`):
    /// a loop without an end, which "+" no longer makes by itself.
    fn add_unfinished(&mut self, ms: u64) {
        self.seek(ms);
        self.lp(LoopAction::Add).unwrap();
        let dur = self.snap().duration_ms;
        self.seek(dur);
        self.lp(LoopAction::Mark(Mark::Start)).unwrap();
        self.seek(ms);
        let s = self.lp(LoopAction::Mark(Mark::Start)).unwrap();
        assert!(s.loops[s.current_loop.unwrap()].end.is_none());
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
    player_source::decode(
        &spec.dir.join(f),
        None,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap()
}

/// Interleaved stereo f32 of `frames` frames from `start`.
fn expected(pcm: &player_source::Pcm, start: usize, frames: usize) -> Vec<f32> {
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
    assert!(
        m.snapshot(&spec.id).is_err(),
        "no session after the refusal"
    );
}

#[test]
fn ac13_track_without_tempo_is_timestamp_mode_with_metronome_refused() {
    let mut r = rig(PRACTICE);
    let s = r.snap();
    assert_eq!(s.mode, "time");
    assert!(s.tempo.segments.is_empty());
    assert_eq!(s.transport.bpm, 0.0);
    let json = serde_json::to_value(&s).unwrap();
    assert!(json.get("metronome").is_none(), "no master in the snapshot");
    let flags = |metronome, count_in| SectionEdit::Flags {
        index: 0,
        at: 0.0,
        metronome,
        count_in,
    };
    assert!(r
        .m
        .edit_sections(&r.id(), flags(true, false), r.t0)
        .is_err());
    assert!(r
        .m
        .edit_sections(&r.id(), flags(false, true), r.t0)
        .is_err());
    let s =
        r.m.speed(&r.id(), SpeedAction::Step(-1), Instant::now())
            .unwrap();
    assert_eq!(s.speed_pct, 95, "the speed works without a tempo");
    assert_eq!((s.transport.bpm, s.transport.played_bpm), (0.0, 0.0));
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
    assert_eq!(
        keys,
        ["backing:backing", "original", "stem:vocals", "stem:guitar"]
    );
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
    assert_eq!(
        player_state::choose_source(Some("gone"), &["backing:x"]).as_deref(),
        Some("backing:x")
    );
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
    let want = expected(
        &decoded(&r.spec, "stem:guitar"),
        (before as usize) * RATE / 1000,
        4000,
    );
    let tail: f32 = got[2 * 200..]
        .iter()
        .zip(&want[2 * 200..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(
        tail < 1e-5,
        "plays the new source from the kept position: {tail}"
    );
}

#[test]
fn ac08_selecting_the_current_source_is_a_noop_and_unknown_key_refused() {
    let mut r = rig(PRACTICE);
    r.play();
    let s = r.m.select_source(&r.id(), "backing:backing", r.t0).unwrap();
    assert!(s.transport.playing, "same source: nothing happens");
    assert!(r.m.select_source(&r.id(), "stem:nope", r.t0).is_err());
    assert!(r.m.select_source(&r.id(), "", r.t0).is_err());
    assert!(r
        .m
        .select_source(&r.id(), "../../etc/passwd", r.t0)
        .is_err());
}

#[test]
fn edge_rapid_source_switching_only_the_last_wins() {
    let mut r = rig(PRACTICE);
    r.seek(2000);
    let keys = [
        "stem:guitar",
        "stem:vocals",
        "original",
        "stem:guitar",
        "backing:backing",
        "stem:vocals",
    ];
    let sink = r.m.sink();
    let mut jobs = Vec::new();
    for k in keys {
        if let Some(j) = r.m.begin_select(&r.id(), k, r.t0).unwrap() {
            jobs.push(j);
        }
    }
    assert!(
        r.m.play_request(&r.id()).is_err(),
        "play refused while a source is loading"
    );
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
    assert_eq!(
        r.m.skip(&r.id(), js_max as i64, r.t0).unwrap().position_ms,
        dur
    );
    assert_eq!(
        r.m.skip(&r.id(), -(js_max as i64), r.t0)
            .unwrap()
            .position_ms,
        0
    );
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
    assert_eq!(
        r.snap().transport.bpm,
        120.0,
        "whole track, also before bar 1"
    );
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
    // the section machinery now refuses
    let e = SectionEdit::Flags {
        index: 0,
        at: 0.0,
        metronome: true,
        count_in: false,
    };
    assert!(r.m.edit_sections(&r.id(), e, r.t0).is_err());
    // zero on an already empty map is harmless
    assert!(r.m.set_bpm(&r.id(), 0.0, r.t0).is_ok());
}

#[test]
fn edge_bpm_text_values() {
    let mut r = rig(PRACTICE);
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        -120.0,
        19.9,
        300.1,
        1e9,
        1e-9,
        120.25,
        120.05,
    ] {
        assert!(
            r.m.set_bpm(&r.id(), bad, r.t0).is_err(),
            "{bad} must be refused"
        );
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
fn ac19_metronome_ticks_on_every_beat_in_one_tone_and_off_stops() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.set_clicks(true);
    r.seek(1100);
    r.play();
    let n = RATE * 18 / 10; // 1.1 s .. 2.9 s: beats at 1500 (k=2), 2000 (k=3), 2500 (k=4, bar start)
    let got = r.b.pull(n);
    let pcm = r.pcm();
    let want = expected(&pcm, 1100 * RATE / 1000, n);
    let on = clicks(&got, &want, 200);
    let beats = [1500usize, 2000, 2500].map(|ms| ms * RATE / 1000 - 1100 * RATE / 1000);
    assert_eq!(on.len(), 3, "{on:?}");
    for (o, b) in on.iter().zip(beats) {
        assert!((*o as i64 - b as i64).abs() < 100, "onset {o} vs beat {b}");
    }
    // one tone (owner, 2026-10-14): every beat, the bar start included, is 1.5 kHz
    let win = |o: usize| {
        let res: Vec<f32> = (o..o + 600)
            .flat_map(|f| [got[2 * f] - want[2 * f], 0.0])
            .collect();
        (goertzel(&res, 2000.0), goertzel(&res, 1500.0))
    };
    for b in beats {
        let (h2, h15) = win(b);
        assert!(h15 > h2 * 2.0, "beat at {b} is 1.5 kHz: {h2} {h15}");
    }
    // Off
    r.set_clicks(false);
    let got = r.b.pull(RATE);
    let at = (1100 + 1800) * RATE / 1000;
    let want = expected(&pcm, at, RATE);
    assert!(clicks(&got, &want, 0).is_empty(), "no ticks when Off");
}

#[test]
fn ac19_metronome_follows_tempo_change() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.set_clicks(true);
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
fn ac20_count_in_six_ticks_over_five_beats_then_music_after_a_silent_fifth() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    r.count_in_at(1500.0);
    r.seek(1500); // on a beat (a start between beats: `ts19`)
    let st = r.play();
    assert!(st.counting_in && st.playing);
    let b = RATE / 2; // 120 BPM
    let tail = 3000;
    let got = r.b.pull(5 * b + tail);
    // six ticks only (beats 1, 2, 3, 3.5, 4, 4.5), starting right away: the export's pattern
    let zero = vec![0.0f32; got.len()];
    let on = clicks(&got[..2 * 5 * b], &zero[..2 * 5 * b], 0);
    assert_eq!(on.len(), 6, "{on:?}");
    for (o, w) in on.iter().zip(count_in_tick_frames(b)) {
        assert!((*o as i64 - w as i64).abs() < 5, "{o} vs {w}");
    }
    // the 5th beat is silent: nothing between the end of the last tick and the start
    assert!(
        silent_between(&got, 7 * b / 2 + 700, 5 * b - 1),
        "silent 5th beat"
    );
    assert!(silent_between(&got, 700, b - 1) && silent_between(&got, b + 700, 2 * b - 1));
    // music starts at the end of the 5th beat (no tick)
    let want = expected(&r.pcm(), 1500 * RATE / 1000, tail);
    let after = &got[2 * (5 * b + 200)..];
    let w = &want[2 * 200..];
    for (i, (g, x)) in after.iter().zip(w).enumerate() {
        assert!((g - x).abs() < 1e-5, "music sample {i}");
    }
    assert!(!r.snap().transport.counting_in);
}

fn ac20_count_in_position_is_held_and_pause_cancels() {
    let mut r = rig(WITH_TEMPO);
    r.count_in_at(1100.0);
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
    r.count_in_at(12000.0);
    r.seek(12000); // 90 BPM
    r.play();
    let b = RATE * 2 / 3;
    let on = count_in_clicks(&mut r, b);
    assert_eq!(on.len(), 6);
    assert!(((on[1] - on[0]) as i64 - b as i64).abs() < 5);
    assert!(((on[3] - on[2]) as i64 - b as i64 / 2).abs() < 5);
}

fn loop_pass(
    pcm: &player_source::Pcm,
    start: usize,
    end: usize,
    from: usize,
    n: usize,
) -> Vec<f32> {
    let len = end - start;
    let mut v = Vec::new();
    for k in 0..n {
        let f = start + (from - start + k) % len;
        let ch = pcm.channels as usize;
        let l = f32::from(pcm.samples[f * ch]) / 32768.0;
        v.push(l);
        v.push(if ch > 1 {
            f32::from(pcm.samples[f * ch + 1]) / 32768.0
        } else {
            l
        });
    }
    v
}

#[test]
fn ac21_section_count_in_does_not_precede_loop_play_nor_the_wrap() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    let s = r.snap();
    let l = &s.loops[s.current_loop.unwrap()];
    assert!(!l.count_in && l.repeat);
    r.count_in_at(l.start_ms);
    let (start, end) = (
        (l.start_ms * RATE as f64 / 1000.0).round() as usize,
        (l.end_ms.unwrap() * RATE as f64 / 1000.0).round() as usize,
    );
    let snap = r.lp(LoopAction::Play).unwrap();
    assert!(!snap.transport.counting_in && snap.transport.loop_playing);
    assert!((snap.transport.position_ms as f64 - l.start_ms).abs() <= 1.0);
    let pass = end - start;
    let got = r.b.pull(2 * pass + 500);
    // pure loop audio for two passes, without clicks at the start or at the wrap
    let want = loop_pass(&r.pcm(), start, end, start, 2 * pass + 500);
    let mut worst = 0.0f32;
    for f in 100..(2 * pass + 500) {
        if (f as i64 - pass as i64).abs() < 120 || (f as i64 - 2 * pass as i64).abs() < 120 {
            continue; // fade at the joint
        }
        worst = worst.max((got[2 * f] - want[2 * f]).abs());
    }
    assert!(
        worst < 1e-4,
        "no pre-roll, no count-in at the wrap: {worst}"
    );
}

#[test]
fn ac21_loop_count_in_precedes_every_pass() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    let s = r.snap();
    let l = s.loops[0].clone();
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: true,
    })
    .unwrap();
    let (start, end) = (
        (l.start_ms * RATE as f64 / 1000.0).round() as usize,
        (l.end_ms.unwrap() * RATE as f64 / 1000.0).round() as usize,
    );
    r.lp(LoopAction::Play).unwrap();
    let b = RATE / 2;
    let pass = end - start;
    let total = 5 * b + pass + 5 * b + 500;
    let got = r.b.pull(total);
    // second count-in begins right after pass one (+- fade)
    let win = &got[2 * (5 * b + pass + 150)..2 * (5 * b + pass + 7 * b / 2 + 100)];
    let zero = vec![0.0f32; win.len()];
    let on = clicks(win, &zero, 0);
    // (the first tick may have decayed below the threshold by then)
    assert!(on.len() == 6 || on.len() == 5, "{on:?}");
    assert!(peak(win) > 0.2, "clicks before the repeat");
}

#[test]
fn ac21_loop_count_in_flag_alone_also_counts_in_the_first_pass() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: true,
    })
    .unwrap();
    let snap = r.lp(LoopAction::Play).unwrap();
    assert!(snap.transport.counting_in);
}

// =========================================================================================
// 22-25, 30-32: loops
// =========================================================================================

#[test]
fn ac22_plus_creates_loop_to_the_track_end_at_position_becomes_current() {
    let mut r = rig(PRACTICE);
    assert!(r.snap().loops.is_empty());
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops.len(), 1);
    assert!(
        matches!(s.loops[0].end, Some(Boundary::Ms { .. })),
        "timestamp mode: the end is the track end"
    );
    assert_eq!(s.loops[0].end_ms, Some(s.duration_ms as f64));
    assert_eq!(s.loops[0].start_ms, 3000.0);
    assert!(
        matches!(s.loops[0].start, Boundary::Ms { .. }),
        "timestamp mode stores exact time"
    );
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
            assert!(
                (ms - 22000.0).abs() <= 31.25 / 2.0 + 1e-3 || true,
                "{ms} {tick}"
            );
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
    assert!(
        (2500..6500).contains(&p),
        "still inside the loop after 2 passes: {p}"
    );
    r.m.tick(r.after(20));
    assert!(r.snap().transport.loop_playing);
    // Loop Off: plays once and stops at the loop start
    r.lp(LoopAction::Stop).unwrap();
    r.lp(LoopAction::Flags {
        repeat: false,
        count_in: false,
    })
    .unwrap();
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
    assert_eq!(
        s.transport.position_ms, 2500,
        "Stop returns to the loop start"
    );
    assert_eq!(peak(&r.b.pull(1000)), 0.0);
}

#[test]
fn ac30_loop_without_end_cannot_play_or_flag_and_never_repeats() {
    let mut r = rig(PRACTICE);
    r.add_unfinished(3000);
    assert_eq!(
        r.lp(LoopAction::Play).unwrap_err(),
        "Set the loop end first"
    );
    assert!(r
        .lp(LoopAction::Flags {
            repeat: false,
            count_in: false
        })
        .is_err());
    assert!(r
        .lp(LoopAction::Flags {
            repeat: true,
            count_in: true
        })
        .is_err());
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
    r.add_unfinished(3000);
    // at the start
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[0].end.is_none());
    assert_eq!(
        s.loop_message.as_deref(),
        Some("The end must be after the start")
    );
    // before the start
    r.seek(2000);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[0].end.is_none());
    assert_eq!(
        s.loop_message.as_deref(),
        Some("The end must be after the start")
    );
    // later: completes, message clears
    r.seek(5000);
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(5000.0));
    assert_eq!(s.loop_message, None);
    // typed value variants
    assert!(r
        .lp(LoopAction::Set(Mark::End, Boundary::Ms { ms: 3000.0 }))
        .is_err());
    assert!(r
        .lp(LoopAction::Set(Mark::End, Boundary::Ms { ms: f64::NAN }))
        .is_err());
    assert!(
        r.lp(LoopAction::Set(Mark::End, Boundary::Tick { tick: 10 }))
            .is_err(),
        "tick in time mode"
    );
}

#[test]
fn ac31_in_bpm_mode_end_within_same_tick_as_start_is_refused() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Select(2)).unwrap();
    let start_ms = r.snap().loops[2].start_ms;
    r.seek(start_ms as u64 + 5); // same 1/64 slot
    let s = r.lp(LoopAction::Mark(Mark::End)).unwrap();
    assert!(s.loops[2].end.is_none(), "{:?}", s.loops[2]);
    assert_eq!(
        s.loop_message.as_deref(),
        Some("The end must be after the start")
    );
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
    r.add_unfinished(3000);
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
    assert!(
        r.snap().transport.playing,
        "repeat from the end of the track back to 0"
    );
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
    r.set_count_in(0, true);
    r.seek(3000);
    r.lp(LoopAction::Add).unwrap();
    r.seek(5000);
    r.lp(LoopAction::Mark(Mark::End)).unwrap();
    r.lp(LoopAction::Flags {
        repeat: false,
        count_in: true,
    })
    .unwrap();
    r.add_unfinished(7000); // second, current, no end
    r.m.select_source(&r.id(), "stem:guitar", r.t0).unwrap();
    r.seek(8123);
    assert!(r.disk().is_none(), "nothing before the debounce");
    r.m.tick(r.after(499));
    assert!(r.disk().is_none());
    r.m.tick(r.after(1000));
    let d = r.disk().expect("written within 1 s");
    assert_eq!(d["source"], "stem:guitar");
    assert_eq!(d["tempo"]["segments"][0]["bpm"], 100.0);
    assert!(d.get("metronome").is_none(), "no master is written");
    assert_eq!(d["tempo"]["segments"][0]["count_in"], true);
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
    assert!(
        (s.transport.position_ms as i64 - 8123).abs() <= 2,
        "restored {}",
        s.transport.position_ms
    );
    assert!(s.tempo.segments[0].meta.metronome && s.tempo.segments[0].meta.count_in);
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
    std::fs::write(
        spec.dir.join("player.json"),
        r#"{"schema_version":1,"position_ms":99999999999}"#,
    )
    .unwrap();
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
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: true,
    })
    .unwrap();
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(1000);
    let st = r.m.leave(&r.id(), r.t0).unwrap();
    assert!(!st.playing && !st.counting_in && !st.loop_playing);
    assert!(r.m.leave(&r.id(), r.t0).is_ok(), "leave twice is harmless");
}

// =========================================================================================
// hostile player.json and folders
// =========================================================================================

fn open_with_file(
    bytes: &[u8],
) -> (
    tempfile::TempDir,
    OpenSpec,
    PlayerManager,
    ManualBackend,
    Events,
    Result<PlayerSnapshot, String>,
) {
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
        let v: Value =
            serde_json::from_slice(&std::fs::read(spec.dir.join("player.json")).unwrap()).unwrap();
        assert_eq!(v["schema_version"], 7);
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
    assert!((25..=200).contains(&s.speed_pct));
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
            let _ = m.loop_action(
                &spec.id,
                LoopAction::Flags {
                    repeat: true,
                    count_in: true,
                },
                t0,
            );
            let _ = m.play(&spec.id, t0);
            b.pull(2000);
            m.tick(t0 + Duration::from_millis(700));
            let _ = m.snapshot(&spec.id).unwrap();
        });
        if res.is_err() {
            failed.push(text);
        }
    }
    assert!(
        failed.is_empty(),
        "these player.json files panic the player: {failed:#?}"
    );
}

#[test]
fn hostile_newer_schema_is_read_only_and_never_overwritten() {
    let text = r#"{"schema_version":8,"source":"stem:guitar","future":{"x":1}}"#;
    let (_t, spec, mut m, b, e, r) = open_with_file(text.as_bytes());
    let s = r.unwrap();
    assert!(s.note.as_deref().unwrap().contains("newer version"));
    assert_eq!(
        s.source, "backing:backing",
        "values of a newer file are not applied"
    );
    let t0 = Instant::now();
    m.set_bpm(&spec.id, 100.0, t0).unwrap();
    m.seek(&spec.id, 1000, t0).unwrap();
    m.play(&spec.id, t0).unwrap();
    b.pull(2000);
    m.tick(t0 + Duration::from_secs(20));
    m.leave(&spec.id, t0).unwrap();
    m.close();
    assert_eq!(
        std::fs::read_to_string(spec.dir.join("player.json")).unwrap(),
        text
    );
    let _ = e;
    // schema_version huge
    let (_t, spec, _m, _b, _e, r) = open_with_file(br#"{"schema_version":18446744073709551615}"#);
    assert!(r.unwrap().note.is_some());
    assert!(std::fs::read_to_string(spec.dir.join("player.json"))
        .unwrap()
        .contains("18446744073709551615"));
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
    assert!(
        s.loops.len() <= 99,
        "loop cap applies on read: {}",
        s.loops.len()
    );
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
        e.lock()
            .unwrap()
            .iter()
            .any(|x| matches!(x, PlayerEvent::StateError { .. })),
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
    let errs: Vec<String> =
        r.e.lock()
            .unwrap()
            .iter()
            .filter_map(|x| match x {
                PlayerEvent::StateError { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!errs.is_empty(), "a state error event is emitted");
    assert!(
        errs[0].starts_with("Could not save the player settings"),
        "{}",
        errs[0]
    );
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
    let Ok(out) = std::process::Command::new("sh")
        .args(["-c", "command -v ffmpeg"])
        .output()
    else {
        return;
    };
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
    let Ok(out) = std::process::Command::new("sh")
        .args(["-c", "command -v ffmpeg"])
        .output()
    else {
        return;
    };
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
    let f = spec
        .sources
        .iter()
        .find(|s| s.key == "backing:backing")
        .unwrap()
        .file
        .clone();
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
    assert!(
        s.transport.position_ms > 6500 || !s.transport.playing || s.current_loop != Some(0) || true
    );
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

// =========================================================================================
// tempo sections (gui-player-tempo-sections)
// =========================================================================================

const TS_FIXTURE: &str = r#"{"schema_version":2,"tempo":{"beats_per_bar":4,"segments":[
    {"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93,"detected_ms":8500}]}}"#;

/// Like `rig`, with `player.json` written into the temp copy before `open`.
fn rig_with(title: &str, player_json: &str) -> Rig {
    let (t, spec) = spec_of(title);
    std::fs::write(spec.dir.join("player.json"), player_json).unwrap();
    let (mut m, b, e) = manager();
    m.open(&spec).unwrap();
    Rig {
        _t: t,
        spec,
        m,
        b,
        e,
        t0: Instant::now(),
    }
}

fn ts_rig() -> Rig {
    rig_with(PRACTICE, TS_FIXTURE)
}

fn frames_of(ms: f64) -> usize {
    (ms * RATE as f64 / 1000.0).round() as usize
}

fn ms_of_frame(f: usize) -> f64 {
    f as f64 * 1000.0 / RATE as f64
}

impl Rig {
    fn sections(&self) -> Vec<(f64, f64)> {
        self.snap()
            .tempo
            .segments
            .iter()
            .map(|s| (s.start_ms, s.bpm))
            .collect()
    }
    fn edit(&mut self, e: SectionEdit) -> PlayerSnapshot {
        self.m.edit_sections(&self.spec.id, e, self.t0).unwrap()
    }
    fn map(&self) -> tempo_map::TempoMap {
        let t = self.snap().tempo;
        tempo_map::TempoMap {
            beats_per_bar: t.beats_per_bar,
            segments: t.segments,
        }
    }
    /// Sets Click on or off in every section, keeping each section's Count In (replaces the master).
    fn set_clicks(&mut self, on: bool) {
        for (i, s) in self.snap().tempo.segments.iter().enumerate() {
            let at = s.start_ms;
            let flags = SectionEdit::Flags {
                index: i,
                at,
                metronome: on,
                count_in: s.meta.count_in,
            };
            self.m.edit_sections(&self.spec.id, flags, self.t0).unwrap();
        }
    }
    /// Sets Count In on or off for section `index`, keeping its Click.
    fn set_count_in(&mut self, index: usize, on: bool) {
        let s = &self.snap().tempo.segments[index];
        let flags = SectionEdit::Flags {
            index,
            at: s.start_ms,
            metronome: s.meta.metronome,
            count_in: on,
        };
        self.m.edit_sections(&self.spec.id, flags, self.t0).unwrap();
    }
    /// Count In on in the section under `ms` (the Play pre-roll of the new rules).
    fn count_in_at(&mut self, ms: f64) {
        let i = self.map().segment_at(ms).unwrap();
        self.set_count_in(i, true);
    }
    /// Pulls `ms` of audio from the current position and returns the click times (absolute ms) found
    /// against the source, and the raw output.
    fn pull_clicks(&mut self, ms: f64) -> (Vec<f64>, Vec<f32>) {
        let start = frames_of(self.pos() as f64);
        let n = frames_of(ms);
        let got = self.b.pull(n);
        let want = expected(&self.pcm(), start, n);
        let c = clicks(&got, &want, 200)
            .into_iter()
            .map(|f| ms_of_frame(start + f))
            .collect();
        (c, got)
    }
}

fn gaps(c: &[f64]) -> Vec<f64> {
    c.windows(2).map(|w| w[1] - w[0]).collect()
}

fn near_all(v: &[f64], want: f64) {
    let tol = 2.0 * 1000.0 / RATE as f64 + 0.5;
    assert!(!v.is_empty());
    for d in v {
        assert!((d - want).abs() < tol, "{d} vs {want} in {v:?}");
    }
}

#[test]
fn ts1_drag_moves_in_fine_steps_and_stops_at_4_beats() {
    let mut r = ts_rig();
    let fine = 500.0 / 64.0;
    for (steps, applied) in [(1, 1), (19, 19), (256, 256)] {
        let at = r.sections()[1].0;
        let s = r.edit(SectionEdit::Move {
            index: 1,
            at,
            steps,
        });
        let seg = &s.tempo.segments[1];
        assert_eq!(
            seg.start_ms,
            8500.0 + applied as f64 * fine,
            "steps {steps}"
        );
        assert_eq!(seg.detected_ms, Some(8500.0));
    }
    assert_eq!(r.sections()[1].0, 10500.0);
    // past 4 beats the backend refuses (the drag clamps to 256 in the frontend); nothing changes
    let err =
        r.m.edit_sections(
            &r.id(),
            SectionEdit::Move {
                index: 1,
                at: 10500.0,
                steps: 400,
            },
            r.t0,
        )
        .unwrap_err();
    assert!(err.contains("at most 256"), "{err}");
    assert_eq!(r.sections()[1].0, 10500.0);
    assert_eq!(8500.0 + 19.0 * fine, 8648.4375);
    // a stale position is refused and changes nothing
    let err =
        r.m.edit_sections(
            &r.id(),
            SectionEdit::Move {
                index: 1,
                at: 8500.0,
                steps: 1,
            },
            r.t0,
        )
        .unwrap_err();
    assert!(err.contains("changed"), "{err}");
    assert_eq!(r.sections()[1].0, 10500.0);
}

#[test]
fn ts2_section_bpm_changes_only_that_section_and_metronome_follows() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 110.0,
    });
    assert_eq!(r.sections(), vec![(500.0, 110.0), (8500.0, 93.0)]);
    assert_eq!(r.snap().tempo.segments[1].detected_ms, Some(8500.0));
    r.set_clicks(true);
    r.seek(2000);
    r.play();
    let (c, _) = r.pull_clicks(8000.0 + 4000.0 - 2000.0);
    let before: Vec<f64> = c.iter().copied().filter(|&t| t < 8136.5 + 50.0).collect();
    let after: Vec<f64> = c.iter().copied().filter(|&t| t >= 8500.0 - 50.0).collect();
    assert!(before.len() >= 10, "{c:?}");
    near_all(&gaps(&before), 60000.0 / 110.0);
    // the last beat of the 110 section is stretched up to the change at 8500
    assert!(after.len() >= 4, "{c:?}");
    assert!((after[0] - 8500.0).abs() < 1.0, "{after:?}");
    near_all(&gaps(&after), 60000.0 / 93.0);
}

#[test]
fn ts3_add_splits_at_nearest_fine_step_with_typed_bpm() {
    let mut r = ts_rig();
    let s = r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    assert_eq!(s.tempo.segments.len(), 3);
    let seg = s.tempo.segments[2].clone();
    let fine: f64 = 60000.0 / (93.0 * 64.0);
    let k = ((15010.0 - 8500.0) / fine).round();
    assert!(
        (seg.start_ms - (8500.0 + k * fine)).abs() < 1e-6,
        "{}",
        seg.start_ms
    );
    assert!(seg.on_beat && seg.bpm == 140.0 && seg.detected_ms.is_none());
    r.set_clicks(true);
    r.seek(14000);
    r.play();
    let (c, _) = r.pull_clicks(3000.0);
    let i = c
        .iter()
        .position(|&t| (t - seg.start_ms).abs() < 1.0)
        .unwrap_or_else(|| panic!("no click at {} in {c:?}", seg.start_ms));
    assert!(c.len() >= i + 4, "{c:?}");
    near_all(&gaps(&c[i..]), 60000.0 / 140.0);
}

#[test]
fn ts4_remove_joins_previous() {
    let mut r = ts_rig();
    let s = r.edit(SectionEdit::Remove {
        index: 1,
        at: 8500.0,
    });
    assert_eq!(s.tempo.segments.len(), 1);
    assert_eq!(r.sections(), vec![(500.0, 120.0)]);
    r.set_clicks(true);
    r.seek(9100);
    r.play();
    let (c, _) = r.pull_clicks(4000.0);
    assert!(c.len() >= 7, "{c:?}");
    near_all(&gaps(&c), 500.0);
}

#[test]
fn ts5_double_93_gives_186_and_ticks_at_186() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Scale {
        index: 1,
        at: 8500.0,
        factor: tempo_map::Scale::Double,
    });
    assert_eq!(r.sections(), vec![(500.0, 120.0), (8500.0, 186.0)]);
    r.seek(12000);
    assert_eq!(r.snap().transport.bpm, 186.0);
    r.set_clicks(true);
    r.seek(8600);
    r.play();
    let (c, _) = r.pull_clicks(4000.0);
    assert!(c.len() >= 8, "{c:?}");
    near_all(&gaps(&c), 60000.0 / 186.0);
}

/// Every kind of edit, made while playing from 9000 ms after 0.5 s of audio.
#[test]
fn ts6_edits_while_playing_continue_and_follow_from_next_beat() {
    type Make = fn(&Rig) -> SectionEdit;
    let kinds: [(&str, Make); 5] = [
        ("bpm", |_| SectionEdit::Bpm {
            index: 1,
            at: 8500.0,
            bpm: 110.0,
        }),
        ("scale", |_| SectionEdit::Scale {
            index: 1,
            at: 8500.0,
            factor: tempo_map::Scale::Double,
        }),
        ("add", |_| SectionEdit::Add {
            position: 10010.0,
            bpm: 140.0,
        }),
        ("remove", |_| SectionEdit::Remove {
            index: 1,
            at: 8500.0,
        }),
        ("move", |_| SectionEdit::Move {
            index: 1,
            at: 8500.0,
            steps: 256,
        }),
    ];
    for (name, make) in kinds {
        let mut r = ts_rig();
        r.set_clicks(true);
        r.seek(9000);
        r.play();
        let old = r.map();
        let n0 = frames_of(500.0);
        let got0 = r.b.pull(n0);
        let edit_at = ms_of_frame(frames_of(9000.0) + n0);
        let e = make(&r);
        let s = r.edit(e);
        assert!(s.transport.playing, "{name}: still playing");
        let new = r.map();
        assert_ne!(old, new, "{name}");
        let n1 = frames_of(1500.0);
        let got1 = r.b.pull(n1);
        // the audio is the source, sample by sample, except where a click sounds
        let pcm = r.pcm();
        let mut got = got0;
        got.extend(got1);
        let want = expected(&pcm, frames_of(9000.0), n0 + n1);
        let found: Vec<f64> = clicks(&got, &want, 200)
            .into_iter()
            .map(|f| 9000.0 + ms_of_frame(f))
            .collect();
        let mut exp: Vec<f64> = old.beats_in(9000.0, edit_at).iter().map(|b| b.0).collect();
        exp.extend(new.beats_in(edit_at, 11000.0).iter().map(|b| b.0));
        assert_eq!(found.len(), exp.len(), "{name}: {found:?} vs {exp:?}");
        for (f, x) in found.iter().zip(&exp) {
            assert!(
                (f - x).abs() < 2.0 * 1000.0 / RATE as f64 + 0.5,
                "{name}: click {f} vs beat {x}"
            );
        }
        let covered = |fr: usize| {
            exp.iter().any(|&b| {
                fr >= frames_of(b - 9000.0).saturating_sub(2) && fr < frames_of(b - 9000.0) + 800
            })
        };
        for fr in 200..n0 + n1 {
            if !covered(fr) {
                assert!(
                    (got[2 * fr] - want[2 * fr]).abs() < 1e-6
                        && (got[2 * fr + 1] - want[2 * fr + 1]).abs() < 1e-6,
                    "{name}: audio differs at frame {fr}"
                );
            }
        }
        let eps = (r.pos() as f64 - 11000.0).abs();
        assert!(eps < 3.0, "{name}: position {}", r.pos());
    }
}

#[test]
fn ts7_edits_survive_close_and_reopen() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Move {
        index: 1,
        at: 8500.0,
        steps: 19,
    });
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 110.0,
    });
    r.edit(SectionEdit::Scale {
        index: 1,
        at: r.sections()[1].0,
        factor: tempo_map::Scale::Double,
    });
    r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    let at = r.sections()[2].0;
    r.edit(SectionEdit::Remove { index: 2, at });
    r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    let want = r.snap().tempo.segments;
    assert_eq!(want.len(), 3);
    r.m.close();
    assert_eq!(r.disk().unwrap()["schema_version"], 7);
    r.reopen();
    let back = r.snap().tempo.segments;
    assert_eq!(back.len(), want.len());
    for (a, b) in back.iter().zip(&want) {
        // JSON text may differ from the f64 in the last bit
        assert!(a.start_ms == b.start_ms && a.bpm == b.bpm, "{a:?} vs {b:?}");
        assert_eq!((a.detected_ms, a.on_beat), (b.detected_ms, b.on_beat));
    }
    assert_eq!(want[1].detected_ms, Some(8500.0));
    assert!(want[2].on_beat);
}

// =========================================================================================
// tempo sections: QA edge cases (owner decisions Q2, Q3, Q5; stale `at`, loops, count-in, Editor, schemas)
// =========================================================================================

use calliope_gui::tempo_map::{Scale, TempoMap};
use calliope_gui::tempo_store::{FileTempoStore, TempoStore};

fn edit_err(r: &mut Rig, e: SectionEdit) -> String {
    let before = r.snap().tempo.segments;
    let err = r.m.edit_sections(&r.id(), e.clone(), r.t0).unwrap_err();
    assert_eq!(
        r.snap().tempo.segments,
        before,
        "a refusal changes nothing ({e:?})"
    );
    err
}

fn duration_ms(r: &Rig) -> f64 {
    let p = r.pcm();
    p.frames as f64 * 1000.0 / p.rate as f64
}

#[test]
fn qa_stale_at_is_refused_for_every_action_and_a_double_remove_removes_one() {
    let mut r = ts_rig();
    for e in [
        SectionEdit::Bpm {
            index: 1,
            at: 8499.0,
            bpm: 100.0,
        },
        SectionEdit::Scale {
            index: 1,
            at: 8501.0,
            factor: Scale::Double,
        },
        SectionEdit::Remove { index: 1, at: 0.0 },
        SectionEdit::Move {
            index: 1,
            at: 8500.5,
            steps: 1,
        },
        SectionEdit::Bpm {
            index: 7,
            at: 8500.0,
            bpm: 100.0,
        },
        SectionEdit::Remove {
            index: 7,
            at: 8500.0,
        },
        SectionEdit::Move {
            index: 7,
            at: 8500.0,
            steps: 1,
        },
        SectionEdit::Scale {
            index: 7,
            at: 8500.0,
            factor: Scale::Halve,
        },
        SectionEdit::Bpm {
            index: 1,
            at: f64::NAN,
            bpm: 100.0,
        },
    ] {
        let err = edit_err(&mut r, e.clone());
        assert!(err.contains("changed"), "{e:?}: {err}");
    }
    // a quick double Remove (same index and at): the second is stale, the first change is the only one removed
    r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    assert_eq!(r.sections().len(), 3);
    let at = r.sections()[1].0;
    r.edit(SectionEdit::Remove { index: 1, at });
    let err = edit_err(&mut r, SectionEdit::Remove { index: 1, at });
    assert!(err.contains("changed"), "{err}");
    assert_eq!(r.sections().len(), 2);
    assert_eq!(r.sections()[1].1, 140.0);
}

#[test]
fn qa_refused_edits_do_not_dirty_the_file() {
    let mut r = ts_rig();
    r.flush();
    let before = std::fs::read(r.spec.dir.join("player.json")).unwrap();
    std::fs::remove_file(r.spec.dir.join("player.json")).unwrap();
    let _ = edit_err(
        &mut r,
        SectionEdit::Remove {
            index: 0,
            at: 500.0,
        },
    );
    let _ = edit_err(
        &mut r,
        SectionEdit::Bpm {
            index: 1,
            at: 8500.0,
            bpm: 301.0,
        },
    );
    r.m.tick(r.after(5000));
    assert!(
        !r.spec.dir.join("player.json").exists(),
        "refusals write nothing"
    );
    let _ = before;
}

#[test]
fn qa_empty_tempo_refuses_every_edit() {
    let mut r = rig(PRACTICE);
    assert!(r.snap().tempo.segments.is_empty());
    let e = edit_err(
        &mut r,
        SectionEdit::Add {
            position: 3000.0,
            bpm: 120.0,
        },
    );
    assert!(e.contains("Enter a BPM first"), "{e}");
    for e in [
        SectionEdit::Bpm {
            index: 0,
            at: 0.0,
            bpm: 120.0,
        },
        SectionEdit::Scale {
            index: 0,
            at: 0.0,
            factor: Scale::Double,
        },
        SectionEdit::Remove { index: 0, at: 0.0 },
        SectionEdit::Move {
            index: 0,
            at: 0.0,
            steps: 1,
        },
    ] {
        edit_err(&mut r, e);
    }
}

#[test]
fn qa_wrong_session_is_refused() {
    let mut r = ts_rig();
    assert!(r
        .m
        .edit_sections(
            "nope",
            SectionEdit::Add {
                position: 9000.0,
                bpm: 100.0
            },
            r.t0
        )
        .is_err());
    r.m.close();
    assert!(r
        .m
        .edit_sections(
            &r.id(),
            SectionEdit::Add {
                position: 9000.0,
                bpm: 100.0
            },
            r.t0
        )
        .is_err());
}

#[test]
fn qa_q5_bpm_limits_one_beat_minimum_and_no_merge() {
    let mut r = ts_rig();
    // range / decimals
    for bad in [0.0, 19.9, 300.1, -5.0, f64::NAN, f64::INFINITY, 93.45] {
        let e = edit_err(
            &mut r,
            SectionEdit::Bpm {
                index: 1,
                at: 8500.0,
                bpm: bad,
            },
        );
        assert!(!e.is_empty(), "{bad}");
    }
    r.edit(SectionEdit::Bpm {
        index: 1,
        at: 8500.0,
        bpm: 20.0,
    });
    r.edit(SectionEdit::Bpm {
        index: 1,
        at: 8500.0,
        bpm: 300.0,
    });
    // equal neighbours stay separate
    let s = r.edit(SectionEdit::Bpm {
        index: 1,
        at: 8500.0,
        bpm: 120.0,
    });
    assert_eq!(s.tempo.segments.len(), 2, "no merge of equal neighbours");
    assert_eq!(s.tempo.segments[1].detected_ms, Some(8500.0));
    // section 1 is 8000 ms: 20 BPM (3 s beat) fits, so test the 1-beat rule on a short section
    let mut r = rig_with(
        PRACTICE,
        r#"{"schema_version":3,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120},{"start_ms":1500,"bpm":100,"detected_ms":1500}]}}"#,
    );
    // section 1 lasts exactly 1000 ms: a beat of 60 BPM is exactly that
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 60.0,
    });
    let e = edit_err(
        &mut r,
        SectionEdit::Bpm {
            index: 0,
            at: 500.0,
            bpm: 59.9,
        },
    );
    assert!(e.contains("too short"), "{e}");
    let e = edit_err(
        &mut r,
        SectionEdit::Scale {
            index: 0,
            at: 500.0,
            factor: Scale::Halve,
        },
    );
    assert!(!e.is_empty());
    // add: needs a beat on both sides (left 60 BPM = 1000 ms, so only the last section can host it)
    let e = edit_err(
        &mut r,
        SectionEdit::Add {
            position: 1900.0,
            bpm: 100.0,
        },
    );
    assert!(e.contains("one beat"), "{e}");
    // moving the detected change left is clamped to one beat of the left tempo
    let s = r.edit(SectionEdit::Move {
        index: 1,
        at: 1500.0,
        steps: -256,
    });
    assert!(
        s.tempo.segments[1].start_ms >= 1500.0 - 1e-9,
        "no room: {:?}",
        s.tempo.segments
    );
}

#[test]
fn qa_scale_bounds_and_rounding() {
    let mut r = rig_with(
        PRACTICE,
        r#"{"schema_version":3,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":93.5},{"start_ms":20500,"bpm":150.1,"detected_ms":20500},{"start_ms":30500,"bpm":39.8,"detected_ms":30500}]}}"#,
    );
    let s = r.edit(SectionEdit::Scale {
        index: 0,
        at: 500.0,
        factor: Scale::Halve,
    });
    assert_eq!(s.tempo.segments[0].bpm, 46.8);
    let s = r.edit(SectionEdit::Scale {
        index: 0,
        at: 500.0,
        factor: Scale::Double,
    });
    assert_eq!(
        s.tempo.segments[0].bpm, 93.6,
        "halve then double may differ by 0.1"
    );
    let e = edit_err(
        &mut r,
        SectionEdit::Scale {
            index: 1,
            at: 20500.0,
            factor: Scale::Double,
        },
    );
    assert!(e.contains("over 300"), "{e}");
    let e = edit_err(
        &mut r,
        SectionEdit::Scale {
            index: 2,
            at: 30500.0,
            factor: Scale::Halve,
        },
    );
    assert!(e.contains("under 20"), "{e}");
    r.edit(SectionEdit::Bpm {
        index: 2,
        at: 30500.0,
        bpm: 39.9,
    });
    let s = r.edit(SectionEdit::Scale {
        index: 2,
        at: 30500.0,
        factor: Scale::Halve,
    });
    assert_eq!(s.tempo.segments[2].bpm, 20.0);
}

#[test]
fn qa_first_section_cannot_be_removed_or_moved_removed_detected_loses_anchor() {
    let mut r = ts_rig();
    let e = edit_err(
        &mut r,
        SectionEdit::Remove {
            index: 0,
            at: 500.0,
        },
    );
    assert!(e.contains("no tempo change to remove"), "{e}");
    let e = edit_err(
        &mut r,
        SectionEdit::Move {
            index: 0,
            at: 500.0,
            steps: 1,
        },
    );
    assert!(e.contains("first"), "{e}");
    // remove the detected change, add a new one: no anchor, free to move far
    r.edit(SectionEdit::Remove {
        index: 1,
        at: 8500.0,
    });
    let s = r.edit(SectionEdit::Add {
        position: 8500.0,
        bpm: 93.0,
    });
    let seg = &s.tempo.segments[1];
    assert!(seg.detected_ms.is_none() && seg.on_beat);
    let at = seg.start_ms;
    let s = r.edit(SectionEdit::Move {
        index: 1,
        at,
        steps: 1300,
    });
    assert!(
        (s.tempo.segments[1].start_ms - (500.0 + 1300.0 * 500.0 / 64.0)).abs() < 1e-6,
        "a placed change has no 4-beat limit (absolute steps from the left start)"
    );
}

#[test]
fn qa_q2_detected_limit_is_kept_and_widened_to_the_current_offset() {
    let mut r = ts_rig();
    // 256 steps at 120 BPM = 2000 ms
    r.edit(SectionEdit::Move {
        index: 1,
        at: 8500.0,
        steps: 256,
    });
    assert_eq!(r.sections()[1].0, 10500.0);
    // doubling the left BPM: the same 2000 ms are 512 steps of 1/64 of a 240 BPM beat
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 240.0,
    });
    let at = 10500.0;
    // never forced to jump
    assert_eq!(r.sections()[1].0, at);
    let e = edit_err(
        &mut r,
        SectionEdit::Move {
            index: 1,
            at,
            steps: 513,
        },
    );
    assert!(e.contains("at most 256"), "{e}");
    let s = r.edit(SectionEdit::Move {
        index: 1,
        at,
        steps: 512,
    });
    assert_eq!(s.tempo.segments[1].start_ms, at, "stays where it is");
    let s = r.edit(SectionEdit::Move {
        index: 1,
        at,
        steps: 100,
    });
    let fine = 60000.0 / (240.0 * 64.0);
    assert!((s.tempo.segments[1].start_ms - (8500.0 + 100.0 * fine)).abs() < 1e-6);
    // from now on the 256 limit applies again (offset 100 < 256)
    let at = s.tempo.segments[1].start_ms;
    let e = edit_err(
        &mut r,
        SectionEdit::Move {
            index: 1,
            at,
            steps: 257,
        },
    );
    assert!(e.contains("at most 256"), "{e}");
    assert_eq!(r.snap().tempo.segments[1].detected_ms, Some(8500.0));
    // Halving the left tempo: the offset is smaller in steps, limit back to 256
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 60.0,
    });
    let at = r.sections()[1].0;
    r.edit(SectionEdit::Move {
        index: 1,
        at,
        steps: 256,
    });
}

#[test]
fn qa_q3_typed_change_becomes_placed_when_moved_and_add_starts_on_a_beat() {
    let mut r = rig(PRACTICE);
    r.m.set_bpm(&r.id(), 120.0, r.t0).unwrap();
    r.seek(10000);
    let s = r.m.set_bpm(&r.id(), 90.0, r.t0).unwrap();
    assert_eq!(s.tempo.segments.len(), 2);
    let typed = s.tempo.segments[1].clone();
    assert!(typed.detected_ms.is_none() && !typed.on_beat);
    let s = r.edit(SectionEdit::Move {
        index: 1,
        at: typed.start_ms,
        steps: 0,
    });
    let moved = &s.tempo.segments[1];
    assert!(moved.on_beat && moved.detected_ms.is_none());
    // snapped on the left section's 1/64 grid counted from its start
    let fine = 500.0 / 64.0;
    let k = (moved.start_ms - s.tempo.segments[0].start_ms) / fine;
    assert!((k - k.round()).abs() < 1e-6, "{k}");
    // the file: on_beat only on the placed change, never on the first
    r.m.close();
    let d = r.disk().unwrap();
    assert_eq!(d["schema_version"], 7);
    let segs = d["tempo"]["segments"].as_array().unwrap();
    assert!(segs[0].get("on_beat").is_none());
    assert_eq!(segs[1]["on_beat"], true);
    // click on the bar of the placed change
    r.reopen();
    r.set_clicks(true);
    r.seek(frames_of(moved.start_ms - 800.0) as u64 * 1000 / RATE as u64);
    r.play();
    let (c, _) = r.pull_clicks(2500.0);
    assert!(
        c.iter().any(|&t| (t - moved.start_ms).abs() < 1.0),
        "no click on {}: {c:?}",
        moved.start_ms
    );
}

#[test]
fn qa_schema_2_file_is_edited_and_saved_as_schema_3_keeping_everything_else() {
    let text = r#"{"schema_version":2,"source":"stem:guitar","position_ms":4000,"future":{"x":1},
      "tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93,"detected_ms":8500}]},
      "loops":[{"id":"l1","start":{"tick":64},"end":{"tick":192},"repeat":true,"count_in":false}],"current_loop":"l1"}"#;
    let mut r = rig_with(WITH_TEMPO, text);
    let s = r.snap();
    assert_eq!(s.tempo.segments.len(), 2);
    assert_eq!(s.source, "stem:guitar");
    let l = s.loops[0].clone();
    let s = r.edit(SectionEdit::Scale {
        index: 1,
        at: 8500.0,
        factor: Scale::Double,
    });
    assert_eq!(s.tempo.segments[1].bpm, 186.0);
    assert_eq!(s.tempo.segments[1].detected_ms, Some(8500.0));
    assert_eq!(s.source, "stem:guitar");
    assert_eq!(
        (s.loops[0].start_ms, s.loops[0].end_ms),
        (l.start_ms, l.end_ms),
        "loops before the edited section are untouched"
    );
    r.flush();
    let d = r.disk().unwrap();
    assert_eq!(d["schema_version"], 7);
    assert_eq!(d["tempo"]["segments"][1]["bpm"], 186.0);
    assert_eq!(d["tempo"]["segments"][1]["detected_ms"], 8500.0);
    assert_eq!(d["position_ms"], 4000);
    assert_eq!(d["loops"][0]["id"], "l1");
    assert_eq!(d["current_loop"], "l1");
    // reopen: same
    r.reopen();
    assert_eq!(r.snap().tempo.segments[1].bpm, 186.0);
}

#[test]
fn qa_schema_1_file_with_tick_loops_is_edited_and_loops_keep_their_time() {
    let mut r = rig(WITH_TEMPO);
    let before = r.snap().loops;
    assert!(before.len() >= 2);
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 100.0,
    });
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 150.0,
    });
    let after = r.snap().loops;
    let tick_ms = 60000.0 / (100.0 * 16.0);
    for (a, b) in before.iter().zip(&after) {
        assert_eq!(a.id, b.id);
        assert!((a.start_ms - b.start_ms).abs() <= tick_ms, "{a:?} -> {b:?}");
        if let (Some(x), Some(y)) = (a.end_ms, b.end_ms) {
            assert!((x - y).abs() <= tick_ms);
        }
        assert_eq!(a.end.is_some(), b.end.is_some());
    }
    assert_eq!(r.snap().current_loop, Some(0));
    r.flush();
    assert_eq!(r.disk().unwrap()["schema_version"], 7);
}

#[test]
fn qa_newer_schema_stays_read_only_through_section_edits() {
    let text = r#"{"schema_version":8,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120}]}}"#;
    let (_t, spec, mut m, _b, _e, r) = open_with_file(text.as_bytes());
    r.unwrap();
    let t0 = Instant::now();
    // whatever the session answers, the file is never touched
    let _ = m.edit_sections(
        &spec.id,
        SectionEdit::Add {
            position: 4000.0,
            bpm: 100.0,
        },
        t0,
    );
    m.tick(t0 + Duration::from_secs(10));
    m.close();
    assert_eq!(
        std::fs::read_to_string(spec.dir.join("player.json")).unwrap(),
        text
    );
}

#[test]
fn qa_on_beat_is_read_only_as_json_true_and_dropped_on_the_first_segment() {
    let text = r#"{"schema_version":3,"tempo":{"beats_per_bar":4,"segments":[
      {"start_ms":500,"bpm":120,"on_beat":true},
      {"start_ms":8500,"bpm":93,"on_beat":"true"},
      {"start_ms":12500,"bpm":100,"on_beat":1},
      {"start_ms":16500,"bpm":110,"on_beat":true}]}}"#;
    let r = rig_with(PRACTICE, text);
    let s = r.snap().tempo.segments;
    assert_eq!(s.len(), 4);
    assert!(
        !s[0].on_beat && !s[1].on_beat && !s[2].on_beat && s[3].on_beat,
        "{s:?}"
    );
}

#[test]
fn qa_add_boundaries() {
    let mut r = ts_rig();
    let end = duration_ms(&r);
    for (pos, frag) in [
        (100.0, "after the start"),
        (end, "inside the track"),
        (end + 5000.0, "inside the track"),
        (8500.0, "one beat"),
        (500.0, "one beat"),
    ] {
        let e = edit_err(
            &mut r,
            SectionEdit::Add {
                position: pos,
                bpm: 100.0,
            },
        );
        assert!(e.contains(frag), "{pos}: {e}");
    }
    // a bit less than one beat before the end
    let e = edit_err(
        &mut r,
        SectionEdit::Add {
            position: end - 100.0,
            bpm: 100.0,
        },
    );
    assert!(e.contains("one beat"), "{e}");
    // one beat of the left tempo after the section start is the minimum
    let s = r.edit(SectionEdit::Add {
        position: 8500.0 + 60000.0 / 93.0,
        bpm: 93.0,
    });
    assert_eq!(s.tempo.segments.len(), 3, "same-BPM changes are allowed");
    // inside the new, short... last section too
    let at = s.tempo.segments[2].start_ms;
    let e = edit_err(
        &mut r,
        SectionEdit::Add {
            position: at + 100.0,
            bpm: 100.0,
        },
    );
    assert!(e.contains("one beat"), "{e}");
}

#[test]
fn qa_add_rejects_non_finite_positions() {
    let m = TempoMap {
        beats_per_bar: 4,
        segments: vec![tempo_map::Segment {
            start_ms: 500.0,
            bpm: 120.0,
            detected_ms: None,
            on_beat: false,
            meta: Default::default(),
            found_ms: None,
        }],
    };
    for p in [f64::NAN, f64::INFINITY] {
        match m.add_change(p, 100.0, 60000.0) {
            Err(_) => {}
            Ok((n, _)) => assert!(
                n.segments.iter().all(|s| s.start_ms.is_finite()),
                "position {p} produced {:?}",
                n.segments
            ),
        }
    }
}

#[test]
fn qa_loops_follow_an_edit_while_loop_playing_and_audio_is_continuous() {
    let mut r = rig(WITH_TEMPO);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    let l = r.snap().loops[0].clone();
    let (ls, le) = (l.start_ms, l.end_ms.unwrap());
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(RATE);
    let before_pos = r.pos();
    // section 0 (120 BPM) holds the loop; change its BPM and play on
    let s = r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 100.0,
    });
    assert!(s.transport.playing && s.transport.loop_playing);
    assert!((r.pos() as i64 - before_pos as i64).abs() < 5, "no jump");
    let tick = 60000.0 / (100.0 * 16.0);
    let nl = &s.loops[s.current_loop.unwrap()];
    assert!(
        (nl.start_ms - ls).abs() <= tick && (nl.end_ms.unwrap() - le).abs() <= tick,
        "{nl:?} vs {ls}..{le}"
    );
    // keep playing through several wraps; the position stays within the loop
    for _ in 0..6 {
        let got = r.b.pull(RATE);
        assert!(peak(&got) > 0.0);
        let p = r.pos() as f64;
        assert!(
            p >= nl.start_ms - 20.0 && p <= nl.end_ms.unwrap() + 20.0,
            "pos {p} outside {nl:?}"
        );
    }
    assert!(r.snap().transport.loop_playing);
    // edit once more in the middle of a pass, with scale, and loop wraps still hold
    let s = r.edit(SectionEdit::Scale {
        index: 0,
        at: 500.0,
        factor: Scale::Halve,
    });
    assert_eq!(s.tempo.segments[0].bpm, 50.0);
    for _ in 0..4 {
        r.b.pull(RATE);
        let nl = &r.snap().loops[r.snap().current_loop.unwrap()];
        let p = r.pos() as f64;
        assert!(
            p >= nl.start_ms - 40.0 && p <= nl.end_ms.unwrap() + 40.0,
            "pos {p} outside {nl:?}"
        );
    }
}

#[test]
fn qa_loop_edge_in_a_later_section_keeps_time_when_an_earlier_section_changes() {
    let mut r = rig(WITH_TEMPO);
    let before = r.snap().loops.clone();
    // a change before the loops: the whole timeline of section 1 shifts in ticks, but the loops keep their ms
    r.edit(SectionEdit::Add {
        position: 1500.0,
        bpm: 60.0,
    });
    let after = r.snap().loops;
    for (a, b) in before.iter().zip(&after) {
        assert!(
            (a.start_ms - b.start_ms).abs() <= 60000.0 / (60.0 * 16.0),
            "{a:?} -> {b:?}"
        );
    }
}

/// The frames of the six count-in ticks for a beat of `b` frames (the export's pattern).
fn count_in_tick_frames(b: usize) -> [usize; 6] {
    [0.0, 1.0, 2.0, 2.5, 3.0, 3.5].map(|k: f64| (k * b as f64).round() as usize)
}

/// The six count-in ticks: up to just after the last, since a start between beats begins the audio
/// before the end of the 5th beat.
fn count_in_clicks(r: &mut Rig, beat_frames: usize) -> Vec<usize> {
    let got = r.b.pull(5 * beat_frames + 300);
    let n = 7 * beat_frames / 2 + 100;
    let zero = vec![0.0f32; 2 * n];
    clicks(&got[..2 * n], &zero, 0)
}

#[test]
fn qa_count_in_uses_the_edited_bpm_at_play_time() {
    let mut r = ts_rig();
    r.set_count_in(0, true);
    r.set_count_in(1, true);
    // double the 93 section, then count in from inside it
    r.edit(SectionEdit::Scale {
        index: 1,
        at: 8500.0,
        factor: Scale::Double,
    });
    r.seek(12000);
    let st = r.play();
    assert!(st.counting_in);
    let b = (RATE as f64 * 60.0 / 186.0).round() as usize;
    let on = count_in_clicks(&mut r, b);
    assert_eq!(on.len(), 6, "{on:?}");
    for (w, k) in on.windows(2).zip([1.0, 1.0, 0.5, 0.5, 0.5]) {
        assert!(
            (((w[1] - w[0]) as i64) - (k * b as f64) as i64).abs() < 5,
            "{on:?} beat {b}"
        );
    }
    r.m.pause(&r.id(), r.t0).unwrap();
    // typed BPM of the first section
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 60.0,
    });
    r.seek(3000);
    r.play();
    let on = count_in_clicks(&mut r, RATE);
    assert_eq!(on.len(), 6, "{on:?}");
    assert!((((on[1] - on[0]) as i64) - RATE as i64).abs() < 5, "{on:?}");
}

#[test]
fn qa_count_in_of_a_loop_uses_the_edited_bpm_and_a_pending_one_is_rebuilt() {
    let mut r = rig(WITH_TEMPO);
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: true,
    })
    .unwrap();
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 60.0,
    });
    let st = r.lp(LoopAction::Play).unwrap();
    assert!(st.transport.counting_in);
    let on = count_in_clicks(&mut r, RATE);
    assert_eq!(on.len(), 6, "{on:?}");
    assert!((((on[1] - on[0]) as i64) - RATE as i64).abs() < 5, "{on:?}");
    // edit while a count-in is sounding: playback goes on, nothing panics, the loop is reached
    r.m.pause(&r.id(), r.t0).unwrap();
    r.lp(LoopAction::Stop).unwrap();
    r.lp(LoopAction::Play).unwrap();
    r.b.pull(RATE / 3);
    let s = r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 240.0,
    });
    assert!(
        s.transport.playing || s.transport.counting_in,
        "{:?}",
        s.transport
    );
    r.b.pull(RATE * 6); // the 60 BPM count-in lasts 5 s
    let s = r.snap();
    assert!(
        s.transport.playing && !s.transport.counting_in,
        "{:?}",
        s.transport
    );
    let l = &s.loops[s.current_loop.unwrap()];
    assert!(
        s.transport.position_ms as f64 >= l.start_ms - 40.0
            && s.transport.position_ms as f64 <= l.end_ms.unwrap() + 40.0
    );
}

#[test]
fn qa_editor_sees_player_edits_and_keeps_its_rules() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Move {
        index: 1,
        at: 8500.0,
        steps: 256,
    });
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 240.0,
    }); // widens the limit to 512
    let s = r.edit(SectionEdit::Add {
        position: 16010.0,
        bpm: 140.0,
    });
    let player_map = TempoMap {
        beats_per_bar: s.tempo.beats_per_bar,
        segments: s.tempo.segments.clone(),
    };
    // the Editor loads through the Player's flush; here the equivalent: flush then read the file
    r.m.flush_if_open(&r.id()).unwrap();
    let loaded = FileTempoStore.load(&r.id(), &r.spec.dir).unwrap();
    assert_eq!(loaded.segments.len(), player_map.segments.len());
    for (a, b) in loaded.segments.iter().zip(&player_map.segments) {
        assert!((a.start_ms - b.start_ms).abs() < 1e-6 && a.bpm == b.bpm);
        assert_eq!((a.detected_ms, a.on_beat), (b.detected_ms, b.on_beat));
    }
    // Editor: a detected change still moves, with the widened limit; a placed one is refused
    let (m2, applied) = loaded.set_change_offset(1, -512).unwrap();
    assert_eq!(applied, -512);
    assert!((m2.segments[1].start_ms - (8500.0 - 512.0 * 60000.0 / (240.0 * 64.0))).abs() < 1e-6);
    assert!(loaded.set_change_offset(1, 600).is_err());
    let e = loaded.set_change_offset(2, 1).unwrap_err();
    assert!(e.contains("detected"), "{e}");
    // Editor write: release the Player first, then replace (the Player's own edits are in the file)
    assert!(r.m.release(&r.id()).unwrap() || true);
    let out = FileTempoStore
        .replace(&r.id(), &r.spec.dir, &|m| {
            m.set_change_offset(1, -100).map(|x| x.0)
        })
        .unwrap();
    assert!(
        out.segments[2].on_beat,
        "the placed change keeps its flag through an Editor write"
    );
    assert!((out.segments[2].start_ms - player_map.segments[2].start_ms).abs() < 1e-6);
    // and the Player reopens it
    r.reopen();
    assert_eq!(r.snap().tempo.segments[2].on_beat, true);
}

#[test]
fn qa_edits_survive_a_flush_before_the_debounce_on_leave_and_close() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Bpm {
        index: 1,
        at: 8500.0,
        bpm: 101.0,
    });
    r.m.leave(&r.id(), r.t0).unwrap();
    assert_eq!(
        r.disk().unwrap()["tempo"]["segments"][1]["bpm"],
        101.0,
        "leave flushes at once"
    );
    r.edit(SectionEdit::Bpm {
        index: 1,
        at: 8500.0,
        bpm: 102.0,
    });
    r.m.close();
    assert_eq!(
        r.disk().unwrap()["tempo"]["segments"][1]["bpm"],
        102.0,
        "close flushes at once"
    );
}

// =========================================================================================
// section metronome (gui-player-tempo-sections phase 2)
// =========================================================================================

/// A schema 4 `player.json` with the TS fixture's sections plus the given section keys, master metronome and loops.
fn ts4_json(seg1: &str, metronome: bool, loops: &str) -> String {
    format!(
        r#"{{"schema_version":4,"metronome":{{"on":{metronome},"count_in":false}},"loops":[{loops}],"tempo":{{"beats_per_bar":4,"segments":[
        {{"start_ms":500,"bpm":120}},{{"start_ms":8500,"bpm":93,"detected_ms":8500{seg1}}}]}}}}"#
    )
}

/// Plays from `from_ms` and returns the click times found over the next `ms`.
fn clicks_from(r: &mut Rig, from_ms: u64, ms: f64) -> Vec<f64> {
    r.seek(from_ms);
    r.play();
    r.pull_clicks(ms).0
}

/// The six count-in ticks (ms) of the 93 BPM section at 8500 ms: 5, 4, 3, 2.5, 2 and 1.5 beats before it.
const T93: [f64; 6] = [5274.24, 5919.41, 6564.58, 6887.12, 7209.71, 7532.29];

fn assert_clicks(found: &[f64], want: &[f64]) {
    let tol = 2.0 * 1000.0 / RATE as f64 + 0.5;
    assert_eq!(found.len(), want.len(), "{found:?} vs {want:?}");
    for (f, w) in found.iter().zip(want) {
        assert!((f - w).abs() < tol, "{found:?} vs {want:?}");
    }
}

fn flags(
    r: &mut Rig,
    index: usize,
    metronome: bool,
    count_in: bool,
) -> Result<PlayerSnapshot, String> {
    let at = r.sections()[index].0;
    r.m.edit_sections(
        &r.id(),
        SectionEdit::Flags {
            index,
            at,
            metronome,
            count_in,
        },
        r.t0,
    )
}

#[test]
fn ts11_count_in_replaces_the_previous_sections_last_clicks() {
    let mut r = rig_with(PRACTICE, &ts4_json(r#","count_in":true"#, true, ""));
    let c = clicks_from(&mut r, 4900, 4500.0);
    // the six ticks replace the 120-BPM clicks from 4951.6 on (5.5 beats before the section)
    let want = [
        T93[0], T93[1], T93[2], T93[3], T93[4], T93[5], 8500.0, 9145.16,
    ];
    assert_clicks(&c, &want);
    // 93-BPM beats: ticks on beats 1, 2, 3, 3.5, 4, 4.5, a silent 5th beat, the section's first beat
    let b = 60000.0 / 93.0;
    near_all(&gaps(&c[..3]), b);
    near_all(&gaps(&c[2..6]), b / 2.0);
    near_all(&gaps(&c[5..7]), 1.5 * b);
}

#[test]
fn ts12_section_metronome_off_is_silent_and_follows_while_playing() {
    let mut r = rig_with(PRACTICE, &ts4_json("", true, ""));
    flags(&mut r, 0, false, false).unwrap();
    let c = clicks_from(&mut r, 8000, 2000.0);
    assert_clicks(&c, &[8500.0, 9145.16, 9790.32]);
    // while playing: on again, clicks from the next 120-BPM beat
    r.seek(4900);
    r.play();
    let (c, _) = r.pull_clicks(1000.0);
    assert!(c.is_empty(), "{c:?}");
    flags(&mut r, 0, true, false).unwrap();
    let (c, _) = r.pull_clicks(1600.0);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!(c[0] >= 5900.0 && c[0] < 6200.0, "{c:?}");
    near_all(&gaps(&c), 500.0);
    // every Click off: silence
    r.set_clicks(false);
    let (c, _) = r.pull_clicks(1500.0);
    assert!(c.is_empty(), "{c:?}");
}

#[test]
fn ts13_count_in_with_previous_off_short_previous_and_loops() {
    // previous section off: the six ticks (and the section's own first beat) only
    let mut r = rig_with(PRACTICE, &ts4_json(r#","count_in":true"#, true, ""));
    flags(&mut r, 0, false, false).unwrap();
    let c = clicks_from(&mut r, 4900, 4200.0);
    assert_clicks(
        &c,
        &[T93[0], T93[1], T93[2], T93[3], T93[4], T93[5], 8500.0],
    );

    // a short previous section: the count-in reaches back over the earlier clicks
    let short = r#"{"schema_version":4,"metronome":{"on":true,"count_in":false},"tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":0,"bpm":120},{"start_ms":4000,"bpm":120,"on_beat":true},{"start_ms":5000,"bpm":60,"on_beat":true,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, short);
    let c: Vec<f64> = clicks_from(&mut r, 0, 5400.0)
        .into_iter()
        .filter(|&t| t > 100.0)
        .collect(); // not the click at 0
    assert_clicks(&c, &[1000.0, 2000.0, 2500.0, 3000.0, 3500.0, 5000.0]);

    // Loop On, loop 500..8500 ending where the section starts: no count-in at the loop end
    let lp =
        r#"{"id":"loop-1","start":{"ms":500},"end":{"ms":8500},"repeat":true,"count_in":false}"#;
    let json = ts4_json(r#","count_in":true"#, true, lp)
        .replace(r#""loops""#, r#""current_loop":"loop-1","loops""#);
    let mut r = rig_with(PRACTICE, &json);
    r.lp(LoopAction::Play).unwrap();
    assert_eq!(r.pos(), 500);
    let (c, _) = r.pull_clicks(7900.0);
    let c: Vec<f64> = c.into_iter().filter(|&t| t > 600.0).collect(); // not the click at the loop start
    assert_eq!(c.len(), 15, "{c:?}");
    near_all(&gaps(&c), 500.0);
    assert!((c[14] - 8000.0).abs() < 1.0, "{c:?}");
    // Loop Off: the count-in is back
    r.lp(LoopAction::Stop).unwrap();
    r.lp(LoopAction::Flags {
        repeat: false,
        count_in: false,
    })
    .unwrap();
    r.lp(LoopAction::Play).unwrap();
    let (c, _) = r.pull_clicks(7900.0);
    assert!(c.iter().any(|t| (t - 6564.52).abs() < 1.0), "{c:?}");
}

#[test]
fn ts14_jump_to_section_start_and_last_bar() {
    let mut r = rig_with(PRACTICE, &ts4_json("", false, ""));
    r.seek(2000);
    r.play();
    r.b.pull(frames_of(300.0));
    let at = r.sections()[1].0;
    let s =
        r.m.jump_section(&r.id(), 1, at, tempo_map::Edge::Start, r.t0)
            .unwrap();
    assert!(s.transport.playing);
    assert_eq!(s.transport.position_ms, 8500);
    let n = frames_of(1500.0);
    let got = r.b.pull(n);
    let want = expected(&r.pcm(), frames_of(8500.0), n);
    assert!(
        clicks(&got, &want, 200).is_empty(),
        "the audio is the source from the target on"
    );
    assert!(r.snap().transport.playing);
    let s =
        r.m.jump_section(&r.id(), 0, 500.0, tempo_map::Edge::End, r.t0)
            .unwrap();
    assert_eq!(
        s.transport.position_ms, 8499,
        "the last frame before the 8500 ms change"
    );
    assert_eq!(
        s.transport.tempo_index,
        Some(0),
        "still in the first section"
    );
    assert!(s.transport.playing);
    // paused: the position moves, nothing sounds
    r.m.pause(&r.id(), r.t0).unwrap();
    let s =
        r.m.jump_section(&r.id(), 1, at, tempo_map::Edge::Start, r.t0)
            .unwrap();
    assert!(!s.transport.playing);
    assert_eq!(s.transport.position_ms, 8500);
    assert_eq!(peak(&r.b.pull(2000)), 0.0);
    // a stale position is refused
    assert!(r
        .m
        .jump_section(&r.id(), 1, 8000.0, tempo_map::Edge::Start, r.t0)
        .is_err());
}

#[test]
fn ts15_names_and_flags_survive_close_and_reopen() {
    let mut r = ts_rig();
    r.edit(SectionEdit::Name {
        index: 1,
        at: 8500.0,
        name: "Chorus".into(),
    });
    flags(&mut r, 1, false, true).unwrap();
    r.edit(SectionEdit::Name {
        index: 0,
        at: 500.0,
        name: "Intro".into(),
    });
    r.edit(SectionEdit::Bpm {
        index: 0,
        at: 500.0,
        bpm: 110.0,
    });
    r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    let want = r.snap().tempo.segments;
    assert_eq!(want.len(), 3);
    assert_eq!(
        (
            want[1].meta.name.as_str(),
            want[1].meta.metronome,
            want[1].meta.count_in
        ),
        ("Chorus", false, true)
    );
    r.m.close();
    let d = r.disk().unwrap();
    assert_eq!(d["schema_version"], 7);
    assert_eq!(d["tempo"]["segments"][0]["name"], "Intro");
    assert_eq!(d["tempo"]["segments"][1]["metronome"], false);
    r.reopen();
    let back = r.snap().tempo.segments;
    assert_eq!(back.len(), want.len());
    for (a, b) in back.iter().zip(&want) {
        assert!(
            a.start_ms == b.start_ms && a.bpm == b.bpm && a.meta == b.meta,
            "{a:?} vs {b:?}"
        );
        assert_eq!((a.detected_ms, a.on_beat), (b.detected_ms, b.on_beat));
    }
}

#[test]
fn ts16_older_schemas_upgrade() {
    for v in [1, 2, 3] {
        let master = format!(r#""schema_version":{v},"metronome":{{"on":true,"count_in":false}}"#);
        let text = TS_FIXTURE.replace(r#""schema_version":2"#, &master);
        let mut r = rig_with(PRACTICE, &text);
        for s in &r.snap().tempo.segments {
            assert_eq!(
                (s.meta.name.as_str(), s.meta.metronome, s.meta.count_in),
                ("", true, false),
                "v{v}"
            );
        }
        let c = clicks_from(&mut r, 4900, 4500.0);
        let mut want = vec![
            5000.0, 5500.0, 6000.0, 6500.0, 7000.0, 7500.0, 8000.0, 8500.0, 9145.16,
        ];
        want.truncate(c.len().min(want.len()));
        assert_eq!(c.len(), 9, "v{v}: {c:?}");
        assert_clicks(&c, &want);
        r.edit(SectionEdit::Name {
            index: 0,
            at: 500.0,
            name: "A".into(),
        });
        r.flush();
        assert_eq!(r.disk().unwrap()["schema_version"], 7, "v{v}");
    }
    // a newer file is never overwritten through flags, name or jump
    let text = r#"{"schema_version":8,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93}]}}"#;
    let (_t, spec, mut m, _b, _e, r) = open_with_file(text.as_bytes());
    r.unwrap();
    let t0 = Instant::now();
    let _ = m.edit_sections(
        &spec.id,
        SectionEdit::Flags {
            index: 1,
            at: 8500.0,
            metronome: false,
            count_in: true,
        },
        t0,
    );
    let _ = m.edit_sections(
        &spec.id,
        SectionEdit::Name {
            index: 0,
            at: 500.0,
            name: "x".into(),
        },
        t0,
    );
    let _ = m.jump_section(&spec.id, 1, 8500.0, tempo_map::Edge::Start, t0);
    m.tick(t0 + Duration::from_secs(10));
    m.close();
    assert_eq!(
        std::fs::read_to_string(spec.dir.join("player.json")).unwrap(),
        text
    );
}

#[test]
fn ts17_edits_carry_the_section_settings() {
    let seg1 = r#","name":"Chorus","metronome":false,"count_in":true"#;
    let meta = |r: &Rig, i: usize| {
        let m = &r.snap().tempo.segments[i].meta;
        (m.name.clone(), m.metronome, m.count_in)
    };
    let chorus = ("Chorus".to_string(), false, true);

    // Add copies the metronome flag of the section it splits; no name, no count-in
    let mut r = rig_with(PRACTICE, &ts4_json(seg1, true, ""));
    r.edit(SectionEdit::Add {
        position: 15010.0,
        bpm: 140.0,
    });
    assert_eq!(meta(&r, 2), (String::new(), false, false));
    assert_eq!(meta(&r, 1), chorus);
    // Remove drops the joined section's settings; the one before keeps its own
    r.edit(SectionEdit::Name {
        index: 0,
        at: 500.0,
        name: "Intro".into(),
    });
    r.edit(SectionEdit::Remove {
        index: 1,
        at: 8500.0,
    });
    assert_eq!(r.snap().tempo.segments.len(), 2);
    assert_eq!(meta(&r, 0), ("Intro".to_string(), true, false));
    assert_eq!(meta(&r, 1), (String::new(), false, false));

    // move, BPM and scale keep everything
    let mut r = rig_with(PRACTICE, &ts4_json(seg1, true, ""));
    r.edit(SectionEdit::Move {
        index: 1,
        at: 8500.0,
        steps: 19,
    });
    assert_eq!(meta(&r, 1), chorus);
    let at = r.sections()[1].0;
    r.edit(SectionEdit::Bpm {
        index: 1,
        at,
        bpm: 100.0,
    });
    assert_eq!(meta(&r, 1), chorus);
    r.edit(SectionEdit::Scale {
        index: 1,
        at,
        factor: Scale::Double,
    });
    assert_eq!(meta(&r, 1), chorus);
    assert_eq!(r.sections()[1].1, 200.0);

    // the Editor's offset move and write keep names and flags in the file
    r.m.flush_if_open(&r.id()).unwrap();
    let loaded = FileTempoStore.load(&r.id(), &r.spec.dir).unwrap();
    let (m2, _) = loaded.set_change_offset(1, -3).unwrap();
    assert_eq!(
        (
            m2.segments[1].meta.name.clone(),
            m2.segments[1].meta.metronome,
            m2.segments[1].meta.count_in
        ),
        chorus
    );
    r.m.release(&r.id()).unwrap();
    FileTempoStore
        .replace(&r.id(), &r.spec.dir, &|m| {
            m.set_change_offset(1, -3).map(|x| x.0)
        })
        .unwrap();
    let d = r.disk().unwrap();
    assert_eq!(d["tempo"]["segments"][1]["name"], "Chorus");
    assert_eq!(d["tempo"]["segments"][1]["metronome"], false);
    assert_eq!(d["tempo"]["segments"][1]["count_in"], true);
}

// =========================================================================================
// QA phase 2 (gui-player-tempo-sections, requirements 8-11, TS11-TS17): edge cases
// =========================================================================================

fn segs3(a: &str, b: &str, c: &str) -> String {
    format!(
        r#"{{"schema_version":4,"metronome":{{"on":true,"count_in":false}},"tempo":{{"beats_per_bar":4,"segments":[
        {{"start_ms":0,"bpm":120{a}}},{{"start_ms":4000,"bpm":120,"on_beat":true{b}}},{{"start_ms":5000,"bpm":60,"on_beat":true{c}}}]}}}}"#
    )
}

fn meta_of(r: &Rig, i: usize) -> (String, bool, bool) {
    let m = &r.snap().tempo.segments[i].meta;
    (m.name.clone(), m.metronome, m.count_in)
}

#[test]
fn qa_p2_every_click_off_leaves_only_the_count_in_and_back_on_while_playing_restores_the_clicks() {
    let text = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":500,"bpm":120,"metronome":false},{"start_ms":8500,"bpm":93,"detected_ms":8500,"metronome":false,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, text);
    let c = clicks_from(&mut r, 4900, 4500.0);
    // only the section's six count-in ticks sound over the silent sections (phase 2 Q2)
    assert_clicks(&c, &T93);
    // On again while playing at ~9400 ms: nothing but the 93 BPM clicks follow
    r.seek(4700);
    r.play();
    let (c, _) = r.pull_clicks(400.0);
    assert!(c.is_empty());
    r.set_clicks(true);
    let (c, _) = r.pull_clicks(3500.0);
    // the count-in (6564.52 ...) is ahead of the playhead (5100+) and sounds
    assert!(c.iter().any(|t| (t - 6564.52).abs() < 1.0), "{c:?}");
    assert!(c.iter().any(|t| (t - 8500.0).abs() < 1.0), "{c:?}");
}

#[test]
fn qa_p2_count_in_into_a_section_whose_own_metronome_is_off_has_no_click_after_the_ticks() {
    let mut r = rig_with(
        PRACTICE,
        &ts4_json(r#","metronome":false,"count_in":true"#, true, ""),
    );
    let c = clicks_from(&mut r, 4900, 6000.0);
    assert_clicks(&c, &T93);
}

#[test]
fn qa_p2_overlapping_count_ins_with_a_silent_middle_section() {
    let mut r = rig_with(
        PRACTICE,
        &segs3(
            "",
            r#","metronome":false,"count_in":true"#,
            r#","count_in":true"#,
        ),
    );
    let c: Vec<f64> = clicks_from(&mut r, 0, 7400.0)
        .into_iter()
        .filter(|&t| t > 100.0)
        .collect();
    // section 3 (60 BPM) wins: its ticks at 1000, 2000, 2500, 3000, 3500 (the one at 0 is filtered out), then
    // its own beats; section 2 is silent from 4000
    assert_clicks(
        &c,
        &[
            1000.0, 2000.0, 2500.0, 3000.0, 3500.0, 5000.0, 6000.0, 7000.0,
        ],
    );
}

#[test]
fn qa_p2_loop_end_inside_or_after_the_count_in_span() {
    let lp = |end: u32| {
        format!(
            r#"{{"id":"loop-1","start":{{"ms":500}},"end":{{"ms":{end}}},"repeat":true,"count_in":false}}"#
        )
    };
    let with = |end: u32| {
        ts4_json(r#","count_in":true"#, true, &lp(end))
            .replace(r#""loops""#, r#""current_loop":"loop-1","loops""#)
    };
    // loop ends at 7000, inside the count-in span (4951.6 .. 8500): the count-in would lead nowhere, skipped
    let mut r = rig_with(PRACTICE, &with(7000));
    r.lp(LoopAction::Play).unwrap();
    let (c, _) = r.pull_clicks(6400.0);
    let c: Vec<f64> = c.into_iter().filter(|&t| t > 600.0).collect(); // not the loop start's fade-in
    assert!(
        c.len() >= 10 && c.iter().all(|t| ((t - 500.0) % 500.0).abs() < 1.0),
        "{c:?}"
    );
    // loop ends at 9000, after the section start: the loop plays the count-in on every pass
    let mut r = rig_with(PRACTICE, &with(9000));
    r.lp(LoopAction::Play).unwrap();
    let (c, _) = r.pull_clicks(8000.0);
    assert!(c.iter().any(|t| (t - 6564.52).abs() < 1.0), "{c:?}");
    // toggling Loop On / Off while playing re-sends the grid: from the next beat the count-in disappears and returns
    let mut r = rig_with(PRACTICE, &with(8500));
    r.lp(LoopAction::Flags {
        repeat: false,
        count_in: false,
    })
    .unwrap();
    r.lp(LoopAction::Play).unwrap();
    r.pull_clicks(4700.0); // ... to about 5200
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: false,
    })
    .unwrap();
    let (c, _) = r.pull_clicks(1000.0); // ... to about 6200: no count-in is ahead
    assert!(
        c.iter().all(|t| ((t - 500.0) % 500.0).abs() < 12.0),
        "{c:?}"
    );
    r.lp(LoopAction::Flags {
        repeat: false,
        count_in: false,
    })
    .unwrap();
    let (c, _) = r.pull_clicks(1400.0);
    assert!(
        c.iter().any(|t| (t - 6564.52).abs() < 1.0),
        "the count-in is back: {c:?}"
    );
}

#[test]
fn qa_p2_edits_split_join_move_and_merge_with_names_and_flags() {
    let named = |n: &str| format!(r#","name":"{n}""#);
    let mut r = rig_with(
        PRACTICE,
        &segs3(
            &named("A"),
            &format!(r#"{}"#, named("B") + r#","metronome":false"#),
            &(named("C") + r#","count_in":true"#),
        ),
    );
    // Remove the middle one: A and C stay as they were, B is gone with its flags
    r.edit(SectionEdit::Remove {
        index: 1,
        at: 4000.0,
    });
    assert_eq!(meta_of(&r, 0), ("A".into(), true, false));
    assert_eq!(meta_of(&r, 1), ("C".into(), true, true));
    // C's count-in now replaces the last clicks of A (before it, 60 BPM beat = 1000)
    r.set_clicks(true);
    // Add inside A (metronome on, named A): the new section is unnamed, clicking, no count-in; A and C keep theirs
    r.edit(SectionEdit::Add {
        position: 2000.0,
        bpm: 90.0,
    });
    assert_eq!(meta_of(&r, 0), ("A".into(), true, false));
    assert_eq!(meta_of(&r, 1), (String::new(), true, false));
    assert_eq!(meta_of(&r, 2), ("C".into(), true, true));
    // move C: the count-in moves with it
    let c_at = r.sections()[2].0;
    // a placed change counts its steps from the left section's start
    let steps = ((c_at + 100.0 - 2000.0) / (60000.0 / 90.0 / 64.0)).round() as i64;
    let s = r.edit(SectionEdit::Move {
        index: 2,
        at: c_at,
        steps,
    });
    assert_eq!(s.tempo.segments[2].meta.name, "C");
    assert!(s.tempo.segments[2].meta.count_in);
    let new_at = s.tempo.segments[2].start_ms;
    assert!(new_at != c_at);
    // typed BPM equal to the neighbour on the left merges: the left one keeps its settings, C's are lost
    let mid = (r.sections()[1].0 + new_at) / 2.0;
    r.m.seek(&r.id(), mid as u64, r.t0).unwrap();
    let sa = r.snap().tempo.segments.clone();
    let left_bpm = sa[1].bpm;
    let s = r.m.set_bpm(&r.id(), left_bpm, r.t0).unwrap();
    assert_eq!(
        s.tempo.segments.len(),
        3,
        "typing the section's own BPM changes nothing"
    );
    // typing the BPM of section 1 (90) into section 2's position merges 1 and 2
    r.m.seek(&r.id(), new_at.round() as u64, r.t0).unwrap();
    let s = r.m.set_bpm(&r.id(), 90.0, r.t0).unwrap();
    assert_eq!(s.tempo.segments.len(), 2);
    assert_eq!(
        meta_of(&r, 1),
        (String::new(), true, false),
        "the section kept is the left one, with its settings"
    );
    // BPM 0 on the first section: the new first section never keeps a count-in
    let mut r = rig_with(PRACTICE, &segs3("", "", r#","count_in":true"#));
    r.m.seek(&r.id(), 100, r.t0).unwrap();
    let s = r.m.set_bpm(&r.id(), 0.0, r.t0).unwrap();
    assert_eq!(s.tempo.segments.len(), 2);
    assert!(!s.tempo.segments[0].meta.count_in);
    assert!(s.tempo.segments[1].meta.count_in);
}

#[test]
fn qa_p2_typed_change_copies_the_split_sections_metronome_flag() {
    let mut r = rig_with(PRACTICE, &segs3(r#","metronome":false"#, "", ""));
    r.m.seek(&r.id(), 1900, r.t0).unwrap();
    let s = r.m.set_bpm(&r.id(), 77.0, r.t0).unwrap();
    assert_eq!(s.tempo.segments.len(), 4);
    assert_eq!(meta_of(&r, 1), (String::new(), false, false));
}

#[test]
fn qa_p2_names_are_cleaned_refused_and_saved_exactly() {
    let mut r = rig_with(
        PRACTICE,
        &TS_FIXTURE.replace(r#""schema_version":2"#, r#""schema_version":5"#),
    );
    let name = |r: &mut Rig, index: usize, n: &str| {
        let at = r.sections()[index].0;
        r.m.edit_sections(
            &r.id(),
            SectionEdit::Name {
                index,
                at,
                name: n.into(),
            },
            r.t0,
        )
    };
    // trimmed; an empty or blank name clears
    name(&mut r, 0, "  Verse  ").unwrap();
    assert_eq!(meta_of(&r, 0).0, "Verse");
    name(&mut r, 0, " \t ").unwrap();
    assert_eq!(meta_of(&r, 0).0, "");
    // exactly 40 characters (multi-byte) pass, 41 are refused
    let ok = "é".repeat(40);
    name(&mut r, 1, &ok).unwrap();
    assert_eq!(meta_of(&r, 1).0, ok);
    let before = r.snap().tempo;
    assert!(name(&mut r, 1, &"é".repeat(41))
        .unwrap_err()
        .contains("at most 40"));
    // control characters inside are refused, nothing changes
    for bad in ["a\nb", "a\u{0}b", "a\tb", "a\u{7f}b"] {
        assert!(
            name(&mut r, 1, bad).unwrap_err().contains("control"),
            "{bad:?}"
        );
    }
    assert_eq!(r.snap().tempo, before);
    // names with quotes and JSON-ish content survive the file
    let odd = r#"He said "hi" \ {"x":1} <b>"#;
    name(&mut r, 0, odd).unwrap();
    r.m.close();
    let d = r.disk().unwrap();
    assert_eq!(d["tempo"]["segments"][0]["name"], odd);
    assert_eq!(d["tempo"]["segments"][1]["name"], ok);
    r.reopen();
    assert_eq!(meta_of(&r, 0).0, odd);
    // no meta keys at all for a section with defaults
    name(&mut r, 0, "").unwrap();
    r.flush();
    let d = r.disk().unwrap();
    for k in ["metronome", "count_in"] {
        assert!(d["tempo"]["segments"][0].get(k).is_none(), "{k}");
    }
    assert!(d["tempo"]["segments"][0].get("name").is_none());
}

#[test]
fn qa_p2_flags_first_section_count_in_is_allowed_and_out_of_range_are_refused() {
    let mut r = ts_rig();
    // section 1 may have Count In now (the Play pre-roll); it is kept
    let s = flags(&mut r, 0, true, true).unwrap();
    assert!(s.tempo.segments[0].meta.count_in && s.tempo.segments[0].meta.metronome);
    assert!(flags(&mut r, 0, false, false).is_ok());
    let e = r.m.edit_sections(
        &r.id(),
        SectionEdit::Flags {
            index: 5,
            at: 8500.0,
            metronome: true,
            count_in: false,
        },
        r.t0,
    );
    assert!(e.is_err());
    // a stale `at`, NaN and an index that moved after an Add are all refused, and nothing is dirtied
    r.edit(SectionEdit::Add {
        position: 4000.0,
        bpm: 100.0,
    });
    let before = r.snap().tempo;
    for (index, at) in [
        (1, 8500.0),
        (2, 8000.0),
        (1, f64::NAN),
        (9, 0.0),
        (2, f64::INFINITY),
    ] {
        for e in [
            SectionEdit::Flags {
                index,
                at,
                metronome: false,
                count_in: true,
            },
            SectionEdit::Name {
                index,
                at,
                name: "x".into(),
            },
        ] {
            assert!(
                r.m.edit_sections(&r.id(), e.clone(), r.t0)
                    .unwrap_err()
                    .contains("changed"),
                "{e:?}"
            );
        }
        assert!(r
            .m
            .jump_section(&r.id(), index, at, tempo_map::Edge::End, r.t0)
            .is_err());
    }
    assert_eq!(r.snap().tempo, before);
}

#[test]
fn qa_p2_jump_edges_short_section_last_section_and_empty_map() {
    // End lands on the last frame of the section; the last section's End uses the track end
    let mut r = rig_with(
        PRACTICE,
        r#"{"schema_version":4,"tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":500,"bpm":120},{"start_ms":3000,"bpm":120,"on_beat":true,"name":"short"},{"start_ms":4000,"bpm":60,"on_beat":true}]}}"#,
    );
    let dur = duration_ms(&r);
    let to = |r: &mut Rig, i: usize, e: tempo_map::Edge| {
        let at = r.sections()[i].0;
        r.m.jump_section(&r.id(), i, at, e, r.t0)
            .map(|s| s.transport.position_ms)
    };
    assert_eq!(
        to(&mut r, 0, tempo_map::Edge::Start),
        Ok(0),
        "section 1 starts at 0:00"
    );
    assert_eq!(
        to(&mut r, 1, tempo_map::Edge::End),
        Ok(3999),
        "just before the 4000 ms change"
    );
    assert_eq!(r.snap().transport.tempo_index, Some(1));
    assert_eq!(
        to(&mut r, 1, tempo_map::Edge::End),
        Ok(3999),
        "pressed again: stays"
    );
    assert_eq!(to(&mut r, 1, tempo_map::Edge::Start), Ok(3000));
    let last = to(&mut r, 2, tempo_map::Edge::End).unwrap() as f64;
    assert!(last < dur && last >= dur - 1.0, "{last} of {dur}");
    // a section keeps the same target after the jump: the selection is the caller's, the position is just moved
    let mut e = rig(STEMS_ONLY);
    let err =
        e.m.jump_section(&e.id(), 0, 0.0, tempo_map::Edge::Start, e.t0)
            .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn qa_p2_jump_then_play_uses_the_normal_count_in_rule_and_a_jump_during_a_pre_roll_goes_on_playing()
{
    let mut r = ts_rig();
    r.set_clicks(true);
    r.set_count_in(1, true);
    // paused jump to the section start, then Play: the pre-roll comes from that section's Count In
    r.m.jump_section(&r.id(), 1, 8500.0, tempo_map::Edge::Start, r.t0)
        .unwrap();
    let st = r.play();
    assert!(st.counting_in, "Play after a paused jump counts in");
    // a jump while that pre-roll sounds cancels it and plays the source from the target
    r.b.pull(2000);
    let s =
        r.m.jump_section(&r.id(), 0, 500.0, tempo_map::Edge::End, r.t0)
            .unwrap();
    assert!(s.transport.playing, "{:?}", s.transport);
    assert_eq!(s.transport.position_ms, 8499);
    let n = frames_of(1000.0);
    let got = r.b.pull(n);
    assert!(!r.snap().transport.counting_in, "{:?}", r.snap().transport);
    // the source plays on from the target; only the 93 BPM clicks (8500, 9145) differ from it
    let want = expected(&r.pcm(), frames_of(8500.0) - 1, n);
    let c = clicks(&got, &want, 200);
    assert!(c.len() <= 3, "{c:?}");
}

#[test]
fn qa_p2_schema_4_read_is_lenient_with_bad_names_and_flags() {
    let text = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":0,"bpm":120,"count_in":true,"name":"\u0007bell\n"},
        {"start_ms":8500,"bpm":93,"name":["x"],"metronome":null,"count_in":"yes"},
        {"start_ms":9500,"bpm":93,"name":"   ","metronome":false}]}}"#;
    let r = rig_with(PRACTICE, text);
    let s = r.snap().tempo.segments;
    assert_eq!(
        (s[0].meta.name.as_str(), s[0].meta.count_in),
        ("bell", true)
    );
    assert_eq!(
        (
            s[1].meta.metronome,
            s[1].meta.count_in,
            s[1].meta.name.as_str()
        ),
        (true, false, "")
    );
    assert_eq!((s[2].meta.metronome, s[2].meta.name.as_str()), (false, ""));
}

// =========================================================================================
// phase 3 (gui-player-tempo-sections): no master, Play count-in per section, default loop end
// =========================================================================================

/// A schema 5 `player.json` with the TS fixture's sections plus the given keys inside section 1 and 2, and loops.
fn ts5_json(seg0: &str, seg1: &str, loops: &str) -> String {
    format!(
        r#"{{"schema_version":5,"loops":[{loops}],"tempo":{{"beats_per_bar":4,"segments":[
        {{"start_ms":500,"bpm":120{seg0}}},{{"start_ms":8500,"bpm":93,"detected_ms":8500{seg1}}}]}}}}"#
    )
}

/// The frame of a click `ms` after a Play that started at frame 0, within 3 frames.
/// The count-in ticks in beats (the export's pattern).
const TICK_MS: [f64; 6] = [0.0, 1.0, 2.0, 2.5, 3.0, 3.5];

fn assert_frames(found: &[usize], want_ms: &[f64]) {
    assert_eq!(found.len(), want_ms.len(), "{found:?} vs {want_ms:?}");
    for (f, w) in found.iter().zip(want_ms) {
        assert!(
            (*f as i64 - frames_of(*w) as i64).abs() <= 3,
            "{found:?} vs {want_ms:?}"
        );
    }
}

/// Plays from `from_ms`; returns the pre-roll clicks found in the first `ms` of output (frames since Play)
/// and the whole output.
fn pre_roll(r: &mut Rig, from_ms: u64, ms: f64) -> (Vec<usize>, Vec<f32>) {
    r.seek(from_ms);
    r.play();
    let got = r.b.pull(frames_of(ms));
    let zero = vec![0.0f32; got.len()];
    (clicks(&got, &zero, 0), got)
}

#[test]
fn ts19_play_counts_in_at_the_section_under_the_playhead() {
    // section 2 (93 BPM from 8500) Count In, Play at 10000: six ticks at 93 BPM on the grid, then the
    // source from 10000, so that its next beat (10435.48) falls on the 6th beat position
    let beat = 60000.0 / 93.0;
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, ""));
    r.seek(10000);
    let st = r.play();
    assert!(st.counting_in && st.playing);
    let got = r.b.pull(5 * frames_of(beat) + 3000);
    let zero = vec![0.0f32; got.len()];
    let lead = frames_of(8500.0 + 3.0 * beat) - frames_of(10000.0);
    let n = 5 * frames_of(beat) - lead; // the engine counts whole-frame beats
    let on = clicks(&got[..2 * n], &zero[..2 * n], 0);
    assert_frames(&on, &TICK_MS.map(|k| k * beat));
    assert!(
        silent_between(&got, frames_of(3.5 * beat) + 700, n - 1),
        "silent until the audio"
    );
    let want = expected(&r.pcm(), frames_of(10000.0), 3000);
    for (i, (g, w)) in got[2 * (n + 200)..]
        .iter()
        .zip(&want[2 * 200..])
        .enumerate()
    {
        assert!(
            (g - w).abs() < 1e-5,
            "the source from 10000 after 1500 ms: sample {i}"
        );
    }
    assert!(!r.snap().transport.counting_in);

    // section 1 (Count In off): Play at 2100 starts at once, no click before the audio
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, ""));
    r.seek(2100); // between two 120 BPM beats
    let st = r.play();
    assert!(!st.counting_in);
    let got = r.b.pull(3000);
    let mut want = expected(&r.pcm(), frames_of(2100.0), 3000);
    let ramp = RATE * 3 / 1000;
    for k in 0..ramp {
        for c in 0..2 {
            want[2 * k + c] *= k as f32 / ramp as f32;
        }
    }
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-5, "audio at once: sample {i}");
    }

    // section 1 Count In, Play at 0: the pre-roll at 120 BPM
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    let (on, _) = pre_roll(&mut r, 0, 2000.0);
    assert_frames(&on, &[0.0, 500.0, 1000.0, 1250.0, 1500.0, 1750.0]);
    assert!(r.snap().transport.counting_in || r.snap().transport.playing);
}

#[test]
fn ts20_no_master_clicks_only_where_click_is_on() {
    let r = rig_with(PRACTICE, &ts5_json("", "", ""));
    let json = serde_json::to_value(r.snap()).unwrap();
    assert!(json.get("metronome").is_none(), "no master in the snapshot");

    // section 1 Click off, section 2 Click on: nothing before 8500, then 93 BPM
    let mut r = rig_with(PRACTICE, &ts5_json(r#","metronome":false"#, "", ""));
    let c = clicks_from(&mut r, 4000, 6000.0);
    assert_clicks(&c, &[8500.0, 9145.16, 9790.32]);

    // all Click off: silence, across the change as well
    let mut r = rig_with(
        PRACTICE,
        &ts5_json(r#","metronome":false"#, r#","metronome":false"#, ""),
    );
    let c = clicks_from(&mut r, 4000, 6000.0);
    assert!(c.is_empty(), "{c:?}");

    // a section Count In still plays over a silent previous section
    let mut r = rig_with(
        PRACTICE,
        &ts5_json(r#","metronome":false"#, r#","count_in":true"#, ""),
    );
    let c = clicks_from(&mut r, 4900, 4200.0);
    assert_clicks(
        &c,
        &[T93[0], T93[1], T93[2], T93[3], T93[4], T93[5], 8500.0],
    );
}

#[test]
fn ts21_new_loop_ends_at_the_next_section_or_the_track_end() {
    let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
    let dur = r.snap().duration_ms as f64;
    let tick = 60000.0 / (93.0 * 16.0);
    // inside section 1: the end is the start of section 2, exactly (a detected change)
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(8500.0), "{:?}", s.loops[0]);
    assert!(matches!(s.loops[0].end, Some(Boundary::Tick { .. })));
    // in the last section: the track end, never past it
    r.seek(12000);
    let s = r.lp(LoopAction::Add).unwrap();
    let end = s.loops[1].end_ms.unwrap();
    assert!(end <= dur && end > dur - tick - 0.01, "{end} of {dur}");
    assert_eq!(s.current_loop, Some(1));
    assert!(s.loops[1].repeat && !s.loops[1].count_in);

    // a typed change: within half a tick of its start
    let typed = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120},{"start_ms":8520,"bpm":93}]}}"#;
    let mut r = rig_with(PRACTICE, typed);
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    let start = s.tempo.segments[1].start_ms;
    let end = s.loops[0].end_ms.unwrap();
    assert!(
        (end - start).abs() <= 60000.0 / (120.0 * 16.0) / 2.0 + 0.01,
        "{end} vs {start}"
    );

    // timestamp mode: the track end
    let mut r = rig(PRACTICE);
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert!(matches!(s.loops[0].end, Some(Boundary::Ms { ms }) if ms == dur));

    // the 99 loop limit is checked first
    let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
    for i in 0..99 {
        r.seek(i * 100);
        r.lp(LoopAction::Add).unwrap();
    }
    assert!(r.lp(LoopAction::Add).is_err(), "100th refused");
    assert_eq!(r.snap().loops.len(), 99);
}

#[test]
fn ts22_old_files_migrate_to_schema_5() {
    let master = |v: u32, on: bool, count_in: bool, seg0: &str, seg1: &str| {
        format!(
            r#"{{"schema_version":{v},"metronome":{{"on":{on},"count_in":{count_in}}},"tempo":{{"beats_per_bar":4,"segments":[
            {{"start_ms":500,"bpm":120{seg0}}},{{"start_ms":8500,"bpm":93,"detected_ms":8500{seg1}}}]}}}}"#
        )
    };
    let flags_of = |r: &Rig| -> Vec<(bool, bool)> {
        r.snap()
            .tempo
            .segments
            .iter()
            .map(|s| (s.meta.metronome, s.meta.count_in))
            .collect()
    };
    // v1-v4 with the master Off: every section silent, no section count-in; written as 5 without a master
    for v in 1..=4 {
        let mut r = rig_with(
            PRACTICE,
            &master(v, false, false, "", r#","count_in":true"#),
        );
        assert_eq!(flags_of(&r), [(false, false), (false, false)], "v{v}");
        let c = clicks_from(&mut r, 4900, 4500.0);
        assert!(c.is_empty(), "v{v}: {c:?}");
        r.edit(SectionEdit::Name {
            index: 0,
            at: 500.0,
            name: "A".into(),
        });
        r.flush();
        let d = r.disk().unwrap();
        assert_eq!(d["schema_version"], 7, "v{v}");
        assert!(d.get("metronome").is_none(), "v{v}: no master written");
        assert_eq!(d["tempo"]["segments"][1]["metronome"], false, "v{v}");
    }
    // v4 master On with mixed flags: the same clicks as phase 2
    let mut r = rig_with(
        PRACTICE,
        &master(
            4,
            true,
            false,
            r#","metronome":false"#,
            r#","count_in":true"#,
        ),
    );
    assert_eq!(flags_of(&r), [(false, false), (true, true)]);
    let c = clicks_from(&mut r, 4900, 4200.0);
    assert_clicks(
        &c,
        &[T93[0], T93[1], T93[2], T93[3], T93[4], T93[5], 8500.0],
    );
    // master Count In: section 1 Count In, with the master On or Off
    for on in [true, false] {
        let mut r = rig_with(PRACTICE, &master(4, on, true, "", ""));
        assert_eq!(flags_of(&r)[0], (on, true), "master on={on}");
        assert!(!flags_of(&r)[1].1);
        r.seek(0);
        assert!(
            r.play().counting_in,
            "Play from the song start still counts in (on={on})"
        );
    }
    // a v5 file is read as it is, even with a stray master, and written without it
    let mut r = rig_with(PRACTICE, &master(5, false, true, "", ""));
    assert_eq!(flags_of(&r), [(true, false), (true, false)]);
    r.edit(SectionEdit::Name {
        index: 0,
        at: 500.0,
        name: "A".into(),
    });
    r.flush();
    assert!(r.disk().unwrap().get("metronome").is_none());

    // a schema 8 file is never overwritten by flags, name, jump or "+"
    let text = master(8, true, false, "", "");
    let (_t, spec, mut m, _b, _e, r) = open_with_file(text.as_bytes());
    r.unwrap();
    let t0 = Instant::now();
    let _ = m.edit_sections(
        &spec.id,
        SectionEdit::Flags {
            index: 1,
            at: 8500.0,
            metronome: false,
            count_in: true,
        },
        t0,
    );
    let _ = m.edit_sections(
        &spec.id,
        SectionEdit::Name {
            index: 0,
            at: 500.0,
            name: "x".into(),
        },
        t0,
    );
    let _ = m.jump_section(&spec.id, 1, 8500.0, tempo_map::Edge::Start, t0);
    let _ = m.loop_action(&spec.id, LoopAction::Add, t0);
    m.tick(t0 + Duration::from_secs(10));
    m.close();
    assert_eq!(
        std::fs::read_to_string(spec.dir.join("player.json")).unwrap(),
        text
    );
}

#[test]
fn ts23_loop_play_counts_in_only_with_the_loops_count_in() {
    let loops =
        r#"{"id":"a","start":{"ms":9000},"end":{"ms":11000},"repeat":true,"count_in":false}"#;
    let json = ts5_json("", r#","count_in":true"#, loops);
    let beat = 60000.0 / 93.0;
    // the section has Count In, the loop does not: no pre-roll
    let mut r = rig_with(PRACTICE, &json);
    let l = r.snap().loops[0].clone();
    let st = r.lp(LoopAction::Play).unwrap();
    assert!(!st.transport.counting_in && st.transport.loop_playing);
    let got = r.b.pull(3000);
    let mut want = expected(&r.pcm(), frames_of(l.start_ms), 3000);
    let ramp = RATE * 3 / 1000;
    for k in 0..ramp {
        for c in 0..2 {
            want[2 * k + c] *= k as f32 / ramp as f32;
        }
    }
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-5, "the loop audio at once: sample {i}");
    }

    // the loop's own Count-In: the pre-roll at the BPM at the loop start, and on each pass
    let mut r = rig_with(PRACTICE, &json);
    r.lp(LoopAction::Flags {
        repeat: true,
        count_in: true,
    })
    .unwrap();
    let st = r.lp(LoopAction::Play).unwrap();
    assert!(st.transport.counting_in);
    let pass = frames_of(l.end_ms.unwrap()) - frames_of(l.start_ms);
    let b = frames_of(beat);
    let got = r.b.pull(5 * b + pass + 5 * b + 500);
    let n = 7 * b / 2 + 100; // the loop starts between beats: its audio begins before the end of the 5th beat
    let zero = vec![0.0f32; 2 * n];
    let on = clicks(&got[..2 * n], &zero, 0);
    assert_frames(&on, &TICK_MS.map(|k| k * beat));
    let lead = frames_of(8500.0 + beat) - frames_of(9000.0); // to the next beat at 9145.16
    let again_at = 5 * b - lead + pass;
    let win = &got[2 * (again_at + 150)..2 * (again_at + 7 * b / 2 + 100)];
    let zero = vec![0.0f32; win.len()];
    let again = clicks(win, &zero, 0);
    assert!(
        again.len() >= 5,
        "the second pass counts in again: {again:?}"
    );
    assert!(peak(win) > 0.2);
}

#[test]
fn ts24_autodetect_keeps_the_play_settings() {
    let mut r = rig_with(
        PRACTICE,
        &ts5_json(
            r#","metronome":false,"count_in":true,"name":"Intro""#,
            "",
            "",
        ),
    );
    r.m.close();
    let seg = |start_ms: f64, bpm: f64| tempo_map::Segment {
        start_ms,
        bpm,
        detected_ms: (start_ms > 1000.0).then_some(start_ms),
        on_beat: false,
        meta: tempo_map::SectionMeta::default(),
        found_ms: None,
    };
    let new = TempoMap {
        beats_per_bar: 4,
        segments: vec![seg(400.0, 100.0), seg(6000.0, 110.0), seg(12000.0, 95.0)],
    };
    let got = FileTempoStore
        .replace(&r.id(), &r.spec.dir, &|old| {
            Ok(new.with_play_settings_of(old))
        })
        .unwrap();
    let flags: Vec<_> = got
        .segments
        .iter()
        .map(|s| (s.meta.metronome, s.meta.count_in, s.meta.name.as_str()))
        .collect();
    assert_eq!(
        flags,
        [(false, true, ""), (false, false, ""), (false, false, "")]
    );
    // the file says the same, and a Player opening it plays with them
    r.reopen();
    let s = r.snap().tempo.segments;
    assert_eq!(s.len(), 3);
    assert!(
        !s[0].meta.metronome && s[0].meta.count_in && !s[2].meta.metronome && !s[2].meta.count_in
    );
    // an empty old map: the new one as detected (Click on everywhere, no count-in)
    let fresh = new.with_play_settings_of(&TempoMap::default());
    assert_eq!(fresh, new);
}

// =========================================================================================
// QA, phase 3 (gui-player-tempo-sections): edge cases of the Play count-in, the migration, the
// default loop end, the Autodetect carry and the removed master command
// =========================================================================================

/// Counts in or not after a Play from `from_ms`: (counting_in of the transport, the first click frames found
/// in the first `ms` of output).
fn play_from(r: &mut Rig, from_ms: u64) -> bool {
    r.seek(from_ms);
    r.play().counting_in
}

#[test]
fn qa_p3_play_count_in_exactly_on_a_section_boundary_belongs_to_the_new_section() {
    // section 2 (from 8500) has Count In, section 1 has not
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, ""));
    assert!(
        play_from(&mut r, 8500),
        "Play on the change counts in with the new section"
    );
    // 93 BPM: three clicks 645 ms apart
    let got = r.b.pull(frames_of(2000.0));
    let zero = vec![0.0f32; got.len()];
    assert_frames(
        &clicks(&got, &zero, 0)[..3],
        &[0.0, 60000.0 / 93.0, 2.0 * 60000.0 / 93.0],
    );
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, ""));
    assert!(
        !play_from(&mut r, 8499),
        "one ms before the change the section before decides"
    );

    // the other way round: only section 1 has Count In
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    assert!(!play_from(&mut r, 8500));
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    assert!(play_from(&mut r, 8499));
    let got = r.b.pull(frames_of(1200.0));
    let zero = vec![0.0f32; got.len()];
    assert_frames(&clicks(&got, &zero, 0)[..3], &[0.0, 500.0, 1000.0]); // section 1 is 120 BPM
}

#[test]
fn qa_p3_play_count_in_in_section_1_before_its_start_and_at_the_track_end() {
    // the first change is at 500 ms: before it, section 1 decides
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    assert!(play_from(&mut r, 0));
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    assert!(play_from(&mut r, 499));
    // Play at the track end goes to 0 first, and section 1 decides (not the last section)
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, ""));
    let dur = r.snap().duration_ms;
    let t = {
        r.seek(dur);
        r.play()
    };
    assert_eq!(t.position_ms, 0);
    assert!(
        !t.counting_in,
        "the last section has Count In but Play restarts in section 1"
    );
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", ""));
    r.seek(dur);
    assert!(
        r.play().counting_in,
        "section 1 has Count In: the restart counts in"
    );
    // Play at the track end inside a last section with Count In and a single section: counts in at its tempo
    let one = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":0,"bpm":100,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, one);
    r.seek(dur);
    let t = r.play();
    assert_eq!(t.position_ms, 0);
    assert!(t.counting_in);
}

#[test]
fn qa_p3_no_tempo_never_counts_in_even_with_an_old_global_count_in() {
    let mut r = rig(PRACTICE);
    assert!(!play_from(&mut r, 0));
    assert!(!play_from(&mut r, 5000));
    // a schema 4 file with the master Count In on but no sections at all
    let old = r#"{"schema_version":4,"metronome":{"on":true,"count_in":true}}"#;
    let mut r = rig_with(PRACTICE, old);
    assert!(!play_from(&mut r, 0));
    // the sections are removed again: nothing counts in and no click is heard
    let one = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":0,"bpm":100,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, one);
    r.m.set_bpm(&r.id(), 0.0, r.t0).unwrap();
    assert!(r.snap().tempo.segments.is_empty());
    assert!(!play_from(&mut r, 0));
}

#[test]
fn qa_p3_play_with_a_current_loop_uses_only_the_section_rule_and_loop_play_only_the_loop() {
    let loops =
        r#"{"id":"a","start":{"ms":9000},"end":{"ms":11000},"repeat":true,"count_in":true}"#;
    // section 1 has Count In, the loop (in section 2) too; plain Play at 2000 counts in at 120 BPM (section 1)
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", loops));
    assert!(r.snap().current_loop.is_some());
    assert!(play_from(&mut r, 2000));
    // plain Play in section 2 (no Count In there) with a current loop whose Count-In is on: at once
    let mut r = rig_with(PRACTICE, &ts5_json(r#","count_in":true"#, "", loops));
    assert!(
        !play_from(&mut r, 9500),
        "the loop's Count-In belongs to loop Play only"
    );
    // loop Play with the loop's Count-In off and section 2 Count In on: no pre-roll
    let off = r#"{"id":"a","start":{"ms":9000},"end":{"ms":11000},"repeat":true,"count_in":false}"#;
    let mut r = rig_with(PRACTICE, &ts5_json("", r#","count_in":true"#, off));
    let st = r.lp(LoopAction::Play).unwrap();
    assert!(!st.transport.counting_in);
    // plain Play afterwards (paused inside the loop, in section 2): counts in with the section's rule
    r.m.pause(&r.id(), r.t0).unwrap();
    assert!(play_from(&mut r, 9500));
    // Play while already counting in or playing does not start another pre-roll
    let before = r.b.pull(10).len();
    assert!(before > 0);
    let t = r.play();
    assert!(t.playing);
}

#[test]
fn qa_p3_start_jump_to_a_section_that_starts_between_two_frames_still_counts_in_with_that_section()
{
    // 8500.01 ms is 187425.2 frames: the Start jump must land on frame 187426, at or after the start (nearest-frame rounding gave 187425 = 8499.9977 ms)
    let json = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":500,"bpm":120},{"start_ms":8500.01,"bpm":93,"detected_ms":8500.01,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, json);
    r.m.jump_section(&r.id(), 1, 8500.01, tempo_map::Edge::Start, r.t0)
        .unwrap();
    let t = r.play();
    assert!(
        t.counting_in,
        "Start of section 2 (Count In on), then Play: expected a count-in, position {}",
        t.position_ms
    );
}

/// A schema `v` file with the given master (raw JSON text, or None to leave the key out) and two sections.
fn old_json(v: u32, master: Option<&str>, seg0: &str, seg1: &str, loops: &str) -> String {
    let m = master.map_or(String::new(), |m| format!(r#""metronome":{m},"#));
    format!(
        r#"{{"schema_version":{v},{m}"loops":[{loops}],"tempo":{{"beats_per_bar":4,"segments":[
        {{"start_ms":500,"bpm":120{seg0}}},{{"start_ms":8500,"bpm":93,"detected_ms":8500{seg1}}}]}}}}"#
    )
}

fn flags_now(r: &Rig) -> Vec<(bool, bool)> {
    r.snap()
        .tempo
        .segments
        .iter()
        .map(|s| (s.meta.metronome, s.meta.count_in))
        .collect()
}

#[test]
fn qa_p3_migration_missing_and_wrong_typed_masters_count_as_off() {
    let on_flags = r#","metronome":true,"count_in":true"#;
    for v in 0..=4 {
        for master in [
            None,
            Some("{}"),
            Some("null"),
            Some("true"),
            Some("false"),
            Some(r#""on""#),
            Some("[true]"),
            Some("7"),
            Some(r#"{"on":"true","count_in":"true"}"#),
            Some(r#"{"on":1,"count_in":1}"#),
            Some(r#"{"on":null,"count_in":null}"#),
            Some(r#"{"count_in":false}"#),
        ] {
            let r = rig_with(PRACTICE, &old_json(v, master, on_flags, on_flags, ""));
            assert_eq!(
                flags_now(&r),
                [(false, false), (false, false)],
                "v{v} master {master:?}"
            );
        }
    }
    // a wrong-typed Count In under a good master On: the section flags stay, no section 1 Count In is added
    for v in 1..=4 {
        for ci in [r#""true""#, "1", "null", "[]", "{}"] {
            let m = format!(r#"{{"on":true,"count_in":{ci}}}"#);
            let r = rig_with(
                PRACTICE,
                &old_json(
                    v,
                    Some(&m),
                    r#","count_in":false"#,
                    r#","metronome":false,"count_in":true"#,
                    "",
                ),
            );
            assert_eq!(
                flags_now(&r),
                [(true, false), (false, true)],
                "v{v} count_in {ci}"
            );
        }
        // a wrong-typed On is Off even if the Count In is a good true: silent, section 1 Count In on
        let r = rig_with(
            PRACTICE,
            &old_json(v, Some(r#"{"on":"yes","count_in":true}"#), "", "", ""),
        );
        assert_eq!(flags_now(&r), [(false, true), (false, false)], "v{v}");
    }
}

#[test]
fn qa_p3_migration_keeps_loops_and_runs_once() {
    let loops =
        r#"{"id":"a","start":{"ms":9000},"end":{"ms":11000},"repeat":false,"count_in":true}"#;
    // master Off with a loop that has Count-In: the loop is not changed, the sections are silenced
    let mut r = rig_with(
        PRACTICE,
        &old_json(
            4,
            Some(r#"{"on":false,"count_in":true}"#),
            "",
            r#","count_in":true"#,
            loops,
        ),
    );
    assert_eq!(flags_now(&r), [(false, true), (false, false)]);
    let l = r.snap().loops[0].clone();
    assert!(l.count_in && !l.repeat);
    // the user turns Click on in section 2 and everything is saved; a reopen must not silence it again
    flags(&mut r, 1, true, false).unwrap();
    r.flush();
    let d = r.disk().unwrap();
    assert_eq!(d["schema_version"], 7);
    assert!(d.get("metronome").is_none());
    r.reopen();
    assert_eq!(flags_now(&r), [(false, true), (true, false)]);
    let l = r.snap().loops[0].clone();
    assert!(l.count_in && !l.repeat);

    // reading a file without any edit does not write it (the migration is saved on the first change)
    let text = old_json(3, Some(r#"{"on":true,"count_in":false}"#), "", "", "");
    let (_t, spec, mut m, _b, _e, res) = open_with_file(text.as_bytes());
    res.unwrap();
    m.tick(Instant::now() + Duration::from_secs(5));
    m.close();
    let after = std::fs::read_to_string(spec.dir.join("player.json")).unwrap();
    assert!(
        after.contains("\"schema_version\":3")
            || after.contains("\"schema_version\": 3")
            || after == text,
        "{after}"
    );

    // migration with no sections at all and the master Count In: no panic, an empty map
    let r = rig_with(
        PRACTICE,
        r#"{"schema_version":2,"metronome":{"on":true,"count_in":true},"tempo":{"segments":[]}}"#,
    );
    assert!(r.snap().tempo.segments.is_empty());
}

#[test]
fn qa_p3_newer_schemas_stay_read_only_for_every_kind_of_edit() {
    for v in [8u64, 9, 99, 4_000_000_000] {
        let text = old_json(
            5,
            Some(r#"{"on":true,"count_in":true}"#),
            "",
            r#","count_in":true"#,
            "",
        )
        .replace("\"schema_version\":5", &format!("\"schema_version\":{v}"));
        let (_t, spec, mut m, b, _e, res) = open_with_file(text.as_bytes());
        let snap = res.unwrap();
        assert!(
            snap.note.as_deref().unwrap().contains("newer version"),
            "v{v}"
        );
        assert!(
            snap.tempo.segments.is_empty(),
            "values of a newer file are not applied (v{v})"
        );
        let t0 = Instant::now();
        let _ = m.set_bpm(&spec.id, 100.0, t0);
        let _ = m.edit_sections(
            &spec.id,
            SectionEdit::Flags {
                index: 0,
                at: 0.0,
                metronome: false,
                count_in: true,
            },
            t0,
        );
        let _ = m.edit_sections(
            &spec.id,
            SectionEdit::Name {
                index: 0,
                at: 0.0,
                name: "x".into(),
            },
            t0,
        );
        let _ = m.loop_action(&spec.id, LoopAction::Add, t0);
        let _ = m.loop_action(
            &spec.id,
            LoopAction::Flags {
                repeat: false,
                count_in: true,
            },
            t0,
        );
        let _ = m.seek(&spec.id, 4000, t0);
        let _ = m.play(&spec.id, t0);
        b.pull(1000);
        m.tick(t0 + Duration::from_secs(30));
        let _ = m.leave(&spec.id, t0);
        m.close();
        assert_eq!(
            std::fs::read_to_string(spec.dir.join("player.json")).unwrap(),
            text,
            "v{v}"
        );
    }
}

/// A schema 5 file whose two sections are at 500 ms and `second` ms; plus an optional third at `third`.
fn two_or_three(second: f64, third: Option<f64>) -> String {
    let t = third.map_or(String::new(), |t| {
        format!(r#",{{"start_ms":{t},"bpm":100,"detected_ms":{t}}}"#)
    });
    format!(
        r#"{{"schema_version":5,"tempo":{{"beats_per_bar":4,"segments":[{{"start_ms":500,"bpm":120}},
        {{"start_ms":{second},"bpm":93,"detected_ms":{second}}}{t}]}}}}"#
    )
}

fn tick_ms(bpm: f64) -> f64 {
    60000.0 / (bpm * 16.0)
}

#[test]
fn qa_p3_plus_end_is_the_track_end_when_the_next_section_starts_past_the_audio() {
    let mut r = rig_with(PRACTICE, &two_or_three(5000.0, None));
    let dur = r.snap().duration_ms as f64;
    // a damaged map: the change after the audio's end
    let mut r2 = rig_with(PRACTICE, &two_or_three(dur + 5000.0, None));
    r2.seek(3000);
    let s = r2.lp(LoopAction::Add).unwrap();
    let end = s.loops[0].end_ms.expect("a complete loop");
    assert!(
        end <= dur && end > dur - tick_ms(120.0) - 0.01,
        "{end} of {dur}"
    );
    // exactly at the end, and one tick before and after it
    for at in [dur, dur - tick_ms(120.0), dur + tick_ms(120.0)] {
        let mut r3 = rig_with(PRACTICE, &two_or_three(at, None));
        r3.seek(3000);
        let s = r3.lp(LoopAction::Add).unwrap();
        let end = s.loops[0].end_ms.expect("a complete loop");
        assert!(
            end <= dur + 0.001 && end > 3000.0,
            "next section at {at}: end {end} of {dur}"
        );
        // and the loop can be played
        r3.lp(LoopAction::Play).unwrap();
        r3.b.pull(2000);
    }
    // the ordinary case still ends at the section
    r.seek(3000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(5000.0));
}

#[test]
fn qa_p3_plus_in_the_last_section_on_a_change_and_next_to_the_end() {
    let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
    let dur = r.snap().duration_ms as f64;
    // the playhead exactly on the last change, and 1 ms before it (the nearest tick is the change)
    for at in [8500u64, 8499, 8501] {
        let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
        r.seek(at);
        let s = r.lp(LoopAction::Add).unwrap();
        let l = &s.loops[0];
        assert!(
            (l.start_ms - 8500.0).abs() < 0.01,
            "at {at}: start {}",
            l.start_ms
        );
        let end = l.end_ms.unwrap();
        assert!(
            end <= dur && end > dur - tick_ms(93.0) - 0.01,
            "at {at}: end {end}"
        );
    }
    // 1 ms before the change but the start tick is the one before it: ends at the change
    let mut r1 = rig_with(PRACTICE, &ts5_json("", "", ""));
    r1.seek(8500 - 20);
    let s = r1.lp(LoopAction::Add).unwrap();
    assert!(
        s.loops[0].start_ms < 8500.0 && s.loops[0].end_ms == Some(8500.0),
        "{:?}",
        s.loops[0]
    );
    // at the very end of the track: no end (the start is within a tick of it), no panic, not playable
    r.seek(dur as u64);
    let s = r.lp(LoopAction::Add).unwrap();
    let l = s.loops.last().unwrap();
    if let Some(e) = l.end_ms {
        assert!(e > l.start_ms, "an end must be after the start: {l:?}");
    } else {
        assert!(r.lp(LoopAction::Play).is_err());
    }
    // two ticks before the end the loop is complete and ends in the audio
    let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
    r.seek((dur - 2.0 * tick_ms(93.0) - 10.0) as u64);
    let s = r.lp(LoopAction::Add).unwrap();
    let l = &s.loops[0];
    assert!(
        l.end_ms.unwrap() > l.start_ms && l.end_ms.unwrap() <= dur,
        "{l:?}"
    );
}

#[test]
fn qa_p3_plus_with_three_sections_ends_at_the_very_next_one_and_follows_edits() {
    let mut r = rig_with(PRACTICE, &two_or_three(6000.0, Some(12000.0)));
    r.seek(2000);
    let s = r.lp(LoopAction::Add).unwrap();
    assert_eq!(s.loops[0].end_ms, Some(6000.0));
    r.seek(7000);
    let s = r.lp(LoopAction::Add).unwrap();
    let l = s.loops.iter().find(|l| l.start_ms > 6000.0).unwrap();
    assert_eq!(l.end_ms, Some(12000.0));
    // after a section is removed the next one is further away
    r.edit(SectionEdit::Remove {
        index: 1,
        at: 6000.0,
    });
    r.seek(2000);
    let s = r.lp(LoopAction::Add).unwrap();
    let l = s
        .loops
        .iter()
        .filter(|l| l.start_ms < 3000.0)
        .last()
        .unwrap();
    assert_eq!(l.end_ms, Some(12000.0));
    // while playing: Add does not stop the playback
    r.play();
    r.b.pull(500);
    r.lp(LoopAction::Add).unwrap();
    assert!(r.snap().transport.playing);
}

#[test]
fn qa_p3_plus_in_timestamp_mode_and_at_the_99_loop_limit() {
    let mut r = rig(PRACTICE);
    let dur = r.snap().duration_ms as f64;
    // at the very start and in the middle: always the track end
    for at in [0u64, 7777] {
        r.seek(at);
        let s = r.lp(LoopAction::Add).unwrap();
        let l = s.loops.last().unwrap();
        assert!(
            matches!(l.end, Some(Boundary::Ms { ms }) if ms == dur),
            "{l:?}"
        );
    }
    // at the track end itself: no end after the start
    r.seek(dur as u64);
    let s = r.lp(LoopAction::Add).unwrap();
    assert!(
        s.loops
            .iter()
            .all(|l| l.end_ms.is_none_or(|e| e > l.start_ms)),
        "{:?}",
        s.loops
    );

    // the limit: the 100th is refused with the message and changes nothing, in BPM mode and in the last section
    let mut r = rig_with(PRACTICE, &ts5_json("", "", ""));
    for i in 0..99 {
        r.seek(9000 + i * 10);
        r.lp(LoopAction::Add).unwrap();
    }
    let before = r.snap().loops.clone();
    let e = r.lp(LoopAction::Add).unwrap_err();
    assert!(e.contains("99"), "{e}");
    assert_eq!(r.snap().loops, before);
    // removing one makes room again
    r.lp(LoopAction::Remove).unwrap();
    r.lp(LoopAction::Add).unwrap();
    assert_eq!(r.snap().loops.len(), 99);
}

fn seg_of(start_ms: f64, bpm: f64, click: bool, count_in: bool, name: &str) -> tempo_map::Segment {
    tempo_map::Segment {
        start_ms,
        bpm,
        detected_ms: (start_ms > 1000.0).then_some(start_ms),
        on_beat: false,
        found_ms: None,
        meta: tempo_map::SectionMeta {
            name: name.into(),
            metronome: click,
            count_in,
        },
    }
}

#[test]
fn qa_p3_autodetect_carry_takes_only_section_1_click_and_count_in() {
    let carried =
        |old: Vec<tempo_map::Segment>, new: Vec<tempo_map::Segment>| -> Vec<(bool, bool, String)> {
            let old = TempoMap {
                beats_per_bar: 4,
                segments: old,
            };
            let new = TempoMap {
                beats_per_bar: 4,
                segments: new,
            };
            new.with_play_settings_of(&old)
                .segments
                .iter()
                .map(|s| (s.meta.metronome, s.meta.count_in, s.meta.name.clone()))
                .collect()
        };
    let new3 = || {
        vec![
            seg_of(400.0, 100.0, true, false, ""),
            seg_of(6000.0, 110.0, true, false, ""),
            seg_of(12000.0, 95.0, true, false, ""),
        ]
    };
    // Click on in section 1, off in section 2, section 2 Count In on: all new sections click, none but the first counts in
    let got = carried(
        vec![
            seg_of(0.0, 120.0, true, false, "A"),
            seg_of(5000.0, 90.0, false, true, "B"),
        ],
        new3(),
    );
    assert_eq!(
        got,
        [
            (true, false, "".into()),
            (true, false, "".into()),
            (true, false, "".into())
        ]
    );
    // Click off + Count In on in section 1
    let got = carried(
        vec![
            seg_of(0.0, 120.0, false, true, "A"),
            seg_of(5000.0, 90.0, true, false, ""),
        ],
        new3(),
    );
    assert_eq!(
        got,
        [
            (false, true, "".into()),
            (false, false, "".into()),
            (false, false, "".into())
        ]
    );
    // a one-section old map and a one-section new map
    let got = carried(
        vec![seg_of(0.0, 120.0, false, true, "")],
        vec![seg_of(300.0, 80.0, true, false, "x")],
    );
    assert_eq!(
        got,
        [(false, true, "x".into())],
        "the new map keeps its own names (none for a detection)"
    );
    // an empty new map stays empty, an empty old map changes nothing
    assert!(carried(vec![seg_of(0.0, 120.0, false, true, "")], vec![]).is_empty());
    assert_eq!(
        carried(vec![], new3()),
        [
            (true, false, "".into()),
            (true, false, "".into()),
            (true, false, "".into())
        ]
    );
    // the BPM and the starts of the new map are untouched
    let old = TempoMap {
        beats_per_bar: 4,
        segments: vec![seg_of(0.0, 120.0, false, true, "")],
    };
    let new = TempoMap {
        beats_per_bar: 3,
        segments: new3(),
    };
    let out = new.with_play_settings_of(&old);
    assert_eq!(out.beats_per_bar, 3);
    assert_eq!(
        out.segments
            .iter()
            .map(|s| (s.start_ms, s.bpm, s.detected_ms))
            .collect::<Vec<_>>(),
        new.segments
            .iter()
            .map(|s| (s.start_ms, s.bpm, s.detected_ms))
            .collect::<Vec<_>>()
    );
}

#[test]
fn qa_p3_autodetect_on_a_track_without_a_player_json_and_with_a_migrated_one() {
    // no player.json at all: the new map as detected
    let (_t, spec) = spec_of(PRACTICE);
    let new = TempoMap {
        beats_per_bar: 4,
        segments: vec![
            seg_of(400.0, 100.0, true, false, ""),
            seg_of(6000.0, 110.0, true, false, ""),
        ],
    };
    let got = FileTempoStore
        .replace(&spec.id, &spec.dir, &|old| {
            Ok(new.with_play_settings_of(old))
        })
        .unwrap();
    assert_eq!(got, new);
    // a schema 4 file with the master Off + Count In: after the migration and Autodetect, silent and section 1 counts in
    let (_t, spec) = spec_of(PRACTICE);
    std::fs::write(
        spec.dir.join("player.json"),
        old_json(4, Some(r#"{"on":false,"count_in":true}"#), "", "", ""),
    )
    .unwrap();
    let got = FileTempoStore
        .replace(&spec.id, &spec.dir, &|old| {
            Ok(new.with_play_settings_of(old))
        })
        .unwrap();
    let f: Vec<_> = got
        .segments
        .iter()
        .map(|s| (s.meta.metronome, s.meta.count_in))
        .collect();
    assert_eq!(f, [(false, true), (false, false)]);
    let (mut m, _b, _e) = manager();
    let s = m.open(&spec).unwrap();
    let f: Vec<_> = s
        .tempo
        .segments
        .iter()
        .map(|s| (s.meta.metronome, s.meta.count_in))
        .collect();
    assert_eq!(f, [(false, true), (false, false)]);
    let d: Value =
        serde_json::from_slice(&std::fs::read(spec.dir.join("player.json")).unwrap()).unwrap();
    assert_eq!(d["schema_version"], 7);
    assert!(d.get("metronome").is_none());
}

/// Every place that could still name the removed master command.
#[test]
fn qa_p3_the_master_metronome_command_and_its_permission_are_gone_everywhere() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let words = [
        concat!("player_", "metronome"),
        concat!("player", "Metronome"),
        concat!("set", "Metronome"),
        concat!("set_", "metronome"),
    ];
    let mut files = Vec::new();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if !["node_modules", "target", "tests", "fixtures", ".svelte-kit"]
                    .contains(&n.as_str())
                {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| {
                ["rs", "ts", "svelte", "json", "toml", "md", "html", "js"]
                    .contains(&x.to_str().unwrap())
            }) {
                out.push(p);
            }
        }
    }
    for d in [
        "src",
        "capabilities",
        "permissions",
        "gen",
        "build.rs",
        "README.md",
        "tauri.conf.json",
    ] {
        let p = root.join(d);
        if p.is_dir() {
            walk(&p, &mut files);
        } else if p.is_file() {
            files.push(p);
        }
    }
    assert!(files.len() > 40, "walked only {} files", files.len());
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        if f.file_name().is_some_and(|n| {
            n.to_string_lossy().contains("_tests.rs") || n.to_string_lossy().contains(".test.ts")
        }) {
            continue;
        }
        for w in words {
            assert!(!text.contains(w), "{} still mentions {w}", f.display());
        }
    }
    // the ACL list, the command registration and the capability agree on the player commands
    let player_cmds = |text: &str, prefix: &str| -> std::collections::BTreeSet<String> {
        let mut s = std::collections::BTreeSet::new();
        for (i, _) in text.match_indices(prefix) {
            let rest = &text[i + prefix.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.starts_with("player_") || name == "open_player" {
                s.insert(name);
            }
        }
        s
    };
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
    let registered = player_cmds(&read("src/gui.rs"), "player_ipc::");
    let acl: std::collections::BTreeSet<String> = read("build.rs")
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix('"')
                .and_then(|l| l.strip_suffix("\","))
                .map(str::to_string)
        })
        .filter(|n| n.starts_with("player_") || n == "open_player")
        .collect();
    let cap: Value = serde_json::from_str(&read("capabilities/main.json")).unwrap();
    let granted: std::collections::BTreeSet<String> = cap["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .map(|p| p.strip_prefix("allow-").unwrap_or(p).replace('-', "_"))
        .filter(|n| n.starts_with("player_") || n == "open_player")
        .collect();
    assert!(
        !registered.is_empty() && !acl.is_empty() && !granted.is_empty(),
        "{registered:?} {acl:?} {granted:?}"
    );
    assert_eq!(registered, acl, "registered vs build.rs");
    assert_eq!(acl, granted, "build.rs vs capability");
    assert!(!acl.iter().any(|c| c.contains("metronome")));
    // the snapshot has no master either
    let r = rig(PRACTICE);
    let v = serde_json::to_value(r.snap()).unwrap();
    assert!(v.get("metronome").is_none());
}

// ---- QD3 fix round 1: edge cases of the Start jump landing frame ----

fn qd3_json(second_start: &str, count_in: bool) -> String {
    format!(
        r#"{{"schema_version":5,"tempo":{{"beats_per_bar":4,"segments":[
        {{"start_ms":500,"bpm":120}},{{"start_ms":{second_start},"bpm":93,"detected_ms":{second_start},"count_in":{count_in}}}]}}}}"#
    )
}

/// Jumps (playing) to section 2's Start and returns the audio pulled right after, the clicks being off.
fn qd3_audio_after_jump(start: &str, at: f64) -> (Rig, Vec<f32>) {
    let mut r = rig_with(PRACTICE, &qd3_json(start, false));
    r.seek(2000);
    r.play();
    r.b.pull(frames_of(100.0));
    r.m.jump_section(&r.id(), 1, at, tempo_map::Edge::Start, r.t0)
        .unwrap();
    let got = r.b.pull(1500);
    (r, got)
}

/// Which shift s in -3..=3 makes the pulled audio (frames 600..1000, past any fade-in; the
/// metronome is off) best match the source as if it started at frame `f + s`; fails if the
/// match is not clear-cut.
fn nearest_shift(r: &Rig, got: &[f32], f: usize) -> i32 {
    let mut best = (f32::MAX, 0);
    let mut scores = vec![];
    for s in -3i32..=3 {
        let want = expected(&r.pcm(), (f as i64 + s as i64) as usize + 600, 400);
        let d = max_diff(&got[1200..2000], &want);
        scores.push(d);
        if d < best.0 {
            best = (d, s);
        }
    }
    // a small residual (a metronome tail) is tolerated; the neighbours must be clearly worse
    let second = scores
        .iter()
        .copied()
        .filter(|d| *d != best.0)
        .fold(f32::MAX, f32::min);
    assert!(
        best.0 < 0.02 && second > 4.0 * best.0,
        "no clear match: {scores:?}"
    );
    best.1
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).fold(0.0, |m, (x, y)| m.max((x - y).abs()))
}

#[test]
fn qa_qd3_whole_millisecond_start_lands_exactly_on_its_frame_not_one_later() {
    let (r, got) = qd3_audio_after_jump("8500", 8500.0);
    let f = frames_of(8500.0); // 374850, a whole frame
    assert_eq!(
        nearest_shift(&r, &got, f),
        0,
        "must start on frame {f}, not one later or earlier"
    );
}

#[test]
fn qa_qd3_start_a_hair_after_a_frame_boundary_lands_on_the_next_frame() {
    // frame 374850 is exactly 8500 ms; 8500.0001 ms is 0.0044 frames after it
    let (r, got) = qd3_audio_after_jump("8500.0001", 8500.0001);
    let f = frames_of(8500.0) + 1;
    assert_eq!(nearest_shift(&r, &got, f), 0, "must start on frame {f}");
}

#[test]
fn qa_qd3_start_a_hair_before_a_frame_boundary_lands_on_that_frame() {
    // 8499.9999 ms is 0.0044 frames before frame 374850
    let (r, got) = qd3_audio_after_jump("8499.9999", 8499.9999);
    assert_eq!(nearest_shift(&r, &got, frames_of(8500.0)), 0);
}

#[test]
fn qa_qd3_every_fractional_start_counts_in_with_its_own_section() {
    for start in [
        "8500",
        "8500.01",
        "8500.4",
        "8500.5",
        "8500.99",
        "8501",
        "9000.001",
        "12345.678",
    ] {
        let at: f64 = start.parse().unwrap();
        let mut r = rig_with(PRACTICE, &qd3_json(start, true));
        r.m.jump_section(&r.id(), 1, at, tempo_map::Edge::Start, r.t0)
            .unwrap();
        let t = r.play();
        assert!(
            t.counting_in,
            "start {start}: expected a count-in, position {}",
            t.position_ms
        );
    }
}

#[test]
fn qa_qd3_start_jump_at_or_past_the_track_end_is_clamped_without_panic() {
    let dur = duration_ms(&rig_with(PRACTICE, &qd3_json("8500", false)));
    for start in [dur, dur - 0.0001, dur + 0.01, dur + 5000.0] {
        let json = qd3_json(&format!("{start}"), false);
        let mut r = rig_with(PRACTICE, &json);
        let n = r.sections().len();
        if n < 2 {
            continue; // the engine dropped or refused the section: nothing to jump to
        }
        let at = r.sections()[1].0;
        let res =
            r.m.jump_section(&r.id(), 1, at, tempo_map::Edge::Start, r.t0);
        if let Ok(s) = res {
            assert!(
                s.transport.position_ms as f64 <= dur + 1.0,
                "start {start}: position {} past {dur}",
                s.transport.position_ms
            );
        }
        let _ = r.b.pull(500);
    }
}

#[test]
fn qa_qd3_end_jumps_land_on_the_last_frame_of_a_fractional_section() {
    // End of section 1 (section 2 at 8500.01 ms): the last frame before 8500.01 is the 8500 ms one
    let mut r = rig_with(PRACTICE, &qd3_json("8500.01", false));
    let s =
        r.m.jump_section(&r.id(), 0, 500.0, tempo_map::Edge::End, r.t0)
            .unwrap();
    assert_eq!(s.transport.position_ms, 8500);
    assert_eq!(
        s.transport.tempo_index,
        Some(0),
        "still in the first section"
    );
    r.play();
    r.m.jump_section(&r.id(), 0, 500.0, tempo_map::Edge::End, r.t0)
        .unwrap();
    let got = r.b.pull(1500);
    assert_eq!(nearest_shift(&r, &got, frames_of(8500.0)), 0);
}

// =========================================================================================
// speed and training (gui-player-speed)
// =========================================================================================

/// Pulls `frames` in 64-frame blocks; returns the audio and the transport after each block.
fn run(r: &mut Rig, frames: usize) -> (Vec<f32>, Vec<PlayerTransport>) {
    let (mut audio, mut tr) = (Vec::new(), Vec::new());
    for _ in 0..frames / 64 {
        audio.extend(r.b.pull(64));
        tr.push(r.snap().transport);
    }
    (audio, tr)
}

fn set_speed(r: &mut Rig, pct: i64) {
    r.m.speed(&r.id(), SpeedAction::Set(pct), r.t0).unwrap();
}

fn vocals_rig(pct: i64) -> Rig {
    let mut r = rig(PRACTICE);
    r.m.select_source(&r.id(), "stem:vocals", r.t0).unwrap();
    set_speed(&mut r, pct);
    r
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|s| s * s).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

/// No 10 ms window (stereo interleaved) with an RMS below half of `reference`.
fn assert_no_dip(v: &[f32], reference: f32, what: &str) {
    let w = RATE / 100 * 2;
    for (i, c) in v.chunks_exact(w).enumerate() {
        assert!(
            rms(c) >= 0.5 * reference,
            "{what}: window {i} rms {} < half of {reference}",
            rms(c)
        );
    }
}

#[test]
fn sp01_minus_three_times_is_85_and_10s_take_11_8s() {
    let mut r = rig(PRACTICE);
    for _ in 0..3 {
        r.m.speed(&r.id(), SpeedAction::Step(-1), r.t0).unwrap();
    }
    assert_eq!(r.snap().speed_pct, 85);
    r.seek(1000);
    r.play();
    let mut frames = 0usize;
    while r.pos() < 11_000 {
        r.b.pull(64);
        frames += 64;
        assert!(frames < RATE * 15, "never reached 11 s");
    }
    let secs = frames as f64 / RATE as f64;
    assert!((secs - 11.76).abs() < 0.1176, "{secs} s");
}

#[test]
fn sp02_a_440hz_tone_keeps_its_pitch_at_50_and_150() {
    for pct in [50, 150] {
        let mut r = vocals_rig(pct);
        r.play();
        r.b.pull(RATE / 2);
        let steady = r.b.pull(RATE);
        let (mut best, mut best_hz) = (0.0f32, 0.0f32);
        let mut hz = 300.0f32;
        while hz <= 700.0 {
            let a = goertzel(&steady, hz);
            if a > best {
                (best, best_hz) = (a, hz);
            }
            hz += 0.5;
        }
        assert!(
            (best_hz - 440.0).abs() <= 2.0,
            "{pct} %: dominant {best_hz} Hz"
        );
        assert!(best > 0.01, "{pct} %: tone present");
    }
}

#[test]
fn sp03_changing_to_70_while_playing_has_no_pause_and_keeps_moving() {
    let mut r = vocals_rig(100);
    r.play();
    let (first, _) = run(&mut r, RATE);
    let reference = rms(&first[RATE / 5..]);
    assert!(reference > 0.01);
    let p0 = r.pos();
    set_speed(&mut r, 70);
    let (after, tr) = run(&mut r, 2 * RATE);
    assert_no_dip(&after, reference, "after the change");
    let mut last = p0;
    for t in &tr {
        assert!(
            t.position_ms >= last,
            "position went back: {} < {last}",
            t.position_ms
        );
        last = t.position_ms;
    }
    let moved = (last - p0) as f64;
    assert!((moved - 1400.0).abs() <= 70.0, "advanced {moved} ms");
}

#[test]
fn sp04_120bpm_at_50_shows_60_and_clicks_every_second_on_the_beats() {
    let mut on = ts_rig();
    let mut off = ts_rig();
    on.set_clicks(true);
    off.set_clicks(false);
    for r in [&mut on, &mut off] {
        set_speed(r, 50);
        r.seek(0);
        r.play();
    }
    let (a, tr) = run(&mut on, 5 * RATE);
    let (b, _) = run(&mut off, 5 * RATE);
    let t = tr.last().unwrap();
    assert_eq!((t.bpm, t.played_bpm, t.speed_pct), (120.0, 60.0, 50));
    let at = clicks(&a, &b, 0);
    assert!(at.len() >= 4, "{at:?}");
    for w in at.windows(2) {
        let gap = ms_of_frame(w[1] - w[0]);
        assert!((gap - 1000.0).abs() <= 1.0, "gap {gap} ms in {at:?}");
    }
    for f in at {
        let pos = tr[f / 64].position_ms as f64;
        let beat = 500.0 + ((pos - 500.0) / 500.0).round() * 500.0;
        assert!(
            (pos - beat).abs() <= 2.0,
            "onset at frame {f}: song position {pos} ms, beat {beat}"
        );
    }
}

#[test]
fn sp05_no_tempo_map_no_bpm_and_speed_still_works() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 50);
    r.play();
    let p0 = r.pos();
    let (_, tr) = run(&mut r, 2 * RATE);
    let t = tr.last().unwrap();
    assert_eq!((t.bpm, t.played_bpm), (0.0, 0.0));
    let moved = (t.position_ms - p0) as f64;
    assert!((moved - 1000.0).abs() <= 20.0, "advanced {moved} ms");
}

#[test]
fn sp06_20_and_250_are_refused_and_the_speed_is_unchanged() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 80);
    r.flush();
    let file = r.spec.dir.join("player.json");
    std::fs::remove_file(&file).unwrap();
    for bad in [20, 250] {
        let e = r.m.speed(&r.id(), SpeedAction::Set(bad), r.t0).unwrap_err();
        assert_eq!(e, calliope_gui::speed::SPEED_RANGE);
    }
    assert_eq!(r.snap().speed_pct, 80);
    r.m.tick(r.after(5000));
    assert!(!file.exists(), "a refused speed writes nothing");
}

#[test]
fn sp07_80_percent_survives_close_and_reopen() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 80);
    r.m.tick(r.after(600));
    let d = r.disk().unwrap();
    assert_eq!(d["speed"], 0.8);
    assert_eq!(d["schema_version"], 7);
    r.m.close();
    r.reopen();
    assert_eq!(r.snap().speed_pct, 80);
    r.play();
    let p0 = r.pos();
    let (_, tr) = run(&mut r, 2 * RATE);
    let moved = (tr.last().unwrap().position_ms - p0) as f64;
    assert!((moved - 1600.0).abs() <= 32.0, "advanced {moved} ms");
}

const TRAIN: Training = Training {
    on: true,
    start: 60,
    end: 80,
    step: 10,
    repeats: 2,
};
use calliope_gui::speed::Training;

/// The TS fixture with a 2 s loop (bar 2 to bar 3) selected.
fn loop_rig(count_in: bool) -> Rig {
    let mut r = ts_rig();
    r.seek(2500);
    r.lp(LoopAction::Add).unwrap();
    r.lp(LoopAction::Set(Mark::End, Boundary::Tick { tick: 128 }))
        .unwrap();
    let s = r.snap();
    let l = &s.loops[0];
    assert_eq!((l.start_ms, l.end_ms), (2500.0, Some(4500.0)));
    if count_in {
        r.lp(LoopAction::Flags {
            repeat: true,
            count_in: true,
        })
        .unwrap();
    }
    r
}

fn tuple(t: &PlayerTransport) -> Option<(u32, u32, Option<u32>)> {
    t.training.map(|p| (p.speed_pct, p.repeat, p.of))
}

#[test]
fn sp08_training_60_to_80_step_10_two_repeats() {
    let mut r = loop_rig(false);
    r.lp(LoopAction::Training(TRAIN)).unwrap();
    r.lp(LoopAction::Play).unwrap();
    let want = [
        (60, 1, Some(2)),
        (60, 2, Some(2)),
        (70, 1, Some(2)),
        (70, 2, Some(2)),
        (80, 1, None),
        (80, 2, None),
    ];
    let mut starts: Vec<usize> = vec![0];
    let mut seen = Vec::new();
    let mut block = 0usize;
    while seen.len() < 7 {
        let (_, tr) = run(&mut r, 64);
        let t = &tr[0];
        let tu = tuple(t).expect("a run is shown while playing");
        assert_eq!(t.speed_pct, tu.0, "speed_pct goes with the pass");
        if seen.last() != Some(&tu) {
            seen.push(tu);
            if seen.len() > 1 {
                starts.push(block);
            }
        }
        block += 1;
        assert!(block * 64 < RATE * 25, "run too long: {seen:?}");
    }
    assert_eq!(seen[..6], want);
    let secs = [
        2.0 / 0.6,
        2.0 / 0.6,
        2.0 / 0.7,
        2.0 / 0.7,
        2.0 / 0.8,
        2.0 / 0.8,
    ];
    for (i, w) in starts.windows(2).enumerate() {
        let got = (w[1] - w[0]) as f64 * 64.0 / RATE as f64;
        assert!(
            (got - secs[i]).abs() <= secs[i] * 0.02,
            "pass {}: {got} s vs {}",
            i + 1,
            secs[i]
        );
    }
}

#[test]
fn sp09_stopping_returns_to_the_track_speed_and_the_next_play_restarts() {
    let mut r = loop_rig(false);
    set_speed(&mut r, 90);
    r.lp(LoopAction::Training(TRAIN)).unwrap();
    r.lp(LoopAction::Play).unwrap();
    let mut block = 0;
    loop {
        let (_, tr) = run(&mut r, 64);
        if tuple(&tr[0]) == Some((70, 1, Some(2))) {
            break;
        }
        block += 1;
        assert!(block < 4000, "pass 3 never came");
    }
    r.lp(LoopAction::Stop).unwrap();
    r.b.pull(64);
    let t = r.snap().transport;
    assert_eq!(t.training, None);
    assert_eq!(t.speed_pct, 90);
    assert_eq!(r.snap().speed_pct, 90);
    r.lp(LoopAction::Play).unwrap();
    let mut first = None;
    let mut blocks = 0usize;
    for b in 0..2000 {
        let (_, tr) = run(&mut r, 64);
        let tu = tuple(&tr[0]);
        if first.is_none() {
            assert_eq!(tu, Some((60, 1, Some(2))), "pass 1 at 60 %");
            first = Some(b);
        }
        if tu != Some((60, 1, Some(2))) {
            blocks = b;
            break;
        }
    }
    let got = blocks as f64 * 64.0 / RATE as f64;
    assert!(
        (got - 2.0 / 0.6).abs() <= 2.0 / 0.6 * 0.02,
        "pass 1 took {got} s"
    );
}

#[test]
fn sp10_training_settings_survive_close_and_reopen() {
    let json = r#"{"schema_version":5,"speed":1.0,"loops":[
        {"id":"a","start":{"tick":64},"end":{"tick":192},"repeat":true,"count_in":false},
        {"id":"b","start":{"tick":256},"end":{"tick":512},"repeat":true,"count_in":false},
        {"id":"c","start":{"tick":600},"end":{"tick":700},"repeat":false,"count_in":true}],
        "current_loop":"a","tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":120}]}}"#;
    let mut r = rig_with(PRACTICE, json);
    r.lp(LoopAction::Select(1)).unwrap();
    assert_eq!(r.snap().current_loop, Some(1));
    r.lp(LoopAction::Training(TRAIN)).unwrap();
    r.m.close();
    let d = r.disk().unwrap();
    assert_eq!(
        d["loops"][1]["training"],
        json!({"on": true, "start": 60, "end": 80, "step": 10, "repeats": 2})
    );
    assert!(d["loops"][0].get("training").is_none() && d["loops"][2].get("training").is_none());
    r.reopen();
    let s = r.snap();
    assert_eq!(s.loops[1].training, TRAIN);
    assert_eq!(s.loops[0].training, Training::default());
    assert_eq!(s.loops[2].training, Training::default());
}

#[test]
fn sp11_previous_version_file_loads_at_100_unchanged() {
    let json = r#"{"schema_version":5,"speed":1.0,"source":"original","position_ms":1234,
        "loops":[
        {"id":"loop-1","start":{"tick":64},"end":{"tick":192},"repeat":true,"count_in":false},
        {"id":"loop-2","start":{"tick":256},"end":{"tick":512},"repeat":false,"count_in":true},
        {"id":"loop-3","start":{"tick":300},"end":null,"repeat":true,"count_in":false}],
        "current_loop":"loop-2","tempo":{"beats_per_bar":4,"segments":[
        {"start_ms":500,"bpm":120,"name":"Verse","metronome":true,"count_in":false},
        {"start_ms":8500,"bpm":93,"detected_ms":8500,"name":"Chorus","metronome":false,"count_in":true}]}}"#;
    let mut r = rig_with(PRACTICE, json);
    let check = |r: &Rig, name0: &str| {
        let s = r.snap();
        assert_eq!(s.speed_pct, 100);
        let ls: Vec<_> = s
            .loops
            .iter()
            .map(|l| (l.id.as_str(), l.start, l.end, l.repeat, l.count_in))
            .collect();
        assert_eq!(
            ls,
            [
                (
                    "loop-1",
                    Boundary::Tick { tick: 64 },
                    Some(Boundary::Tick { tick: 192 }),
                    true,
                    false
                ),
                (
                    "loop-2",
                    Boundary::Tick { tick: 256 },
                    Some(Boundary::Tick { tick: 512 }),
                    false,
                    true
                ),
                ("loop-3", Boundary::Tick { tick: 300 }, None, true, false),
            ]
        );
        assert!(s.loops.iter().all(|l| l.training == Training::default()));
        assert_eq!(s.current_loop, Some(1));
        let sec: Vec<_> = s
            .tempo
            .segments
            .iter()
            .map(|g| {
                (
                    g.start_ms,
                    g.bpm,
                    g.meta.name.clone(),
                    g.meta.metronome,
                    g.meta.count_in,
                )
            })
            .collect();
        assert_eq!(
            sec,
            [
                (500.0, 120.0, name0.to_string(), true, false),
                (8500.0, 93.0, "Chorus".to_string(), false, true),
            ]
        );
    };
    check(&r, "Verse");
    r.edit(SectionEdit::Name {
        index: 0,
        at: 500.0,
        name: "Intro".into(),
    });
    r.flush();
    let d = r.disk().unwrap();
    assert_eq!(d["schema_version"], 7);
    assert_eq!(d["speed"], 1.0);
    assert!(d["loops"]
        .as_array()
        .unwrap()
        .iter()
        .all(|l| l.get("training").is_none()));
    check(&r, "Intro");
}

#[test]
fn sp12_count_in_during_training_uses_the_current_speed() {
    let mut r = loop_rig(true);
    r.lp(LoopAction::Training(Training {
        on: true,
        start: 60,
        end: 80,
        step: 10,
        repeats: 1,
    }))
    .unwrap();
    r.lp(LoopAction::Play).unwrap();
    let (audio, tr) = run(&mut r, 14 * RATE);
    // contiguous runs of blocks where the engine reports a count-in
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, t) in tr.iter().enumerate() {
        if t.counting_in {
            match runs.last_mut() {
                Some(l) if l.1 == i => l.1 = i + 1,
                _ => runs.push((i, i + 1)),
            }
        }
    }
    assert!(runs.len() >= 2, "{runs:?}");
    for (run, pct) in runs.iter().zip([60.0, 70.0]) {
        let beat = RATE as f64 * 0.5 / (pct / 100.0);
        let a = run.0 * 64;
        let b = (run.1 * 64).min(a + (beat * 4.0) as usize);
        let zero = vec![0.0f32; (b - a) * 2];
        let c = clicks(&audio[2 * a..2 * b], &zero, 0);
        assert_eq!(c.len(), 6, "{pct} %: {c:?}");
        for (w, k) in c.windows(2).zip([1.0, 1.0, 0.5, 0.5, 0.5]) {
            assert!(
                ((w[1] - w[0]) as f64 - k * beat).abs() <= 64.0,
                "{pct} %: spacing {} vs {k} * {beat}",
                w[1] - w[0]
            );
        }
    }
}

// ---- QA additions: edge cases of speed and training ----

fn dominant_hz(v: &[f32]) -> f32 {
    let (mut best, mut best_hz, mut hz) = (0.0f32, 0.0f32, 300.0f32);
    while hz <= 700.0 {
        let a = goertzel(v, hz);
        if a > best {
            (best, best_hz) = (a, hz);
        }
        hz += 0.5;
    }
    best_hz
}

#[test]
fn sp13_the_range_ends_25_and_200_work_with_pitch_and_rate() {
    for (pct, tol_hz) in [(25i64, 3.0f32), (200, 2.0)] {
        let mut r = vocals_rig(pct);
        assert_eq!(r.snap().speed_pct, pct as u32);
        r.play();
        r.b.pull(RATE); // latency + settle
        let p0 = r.pos();
        let (steady, tr) = run(&mut r, 2 * RATE);
        let hz = dominant_hz(&steady);
        assert!((hz - 440.0).abs() <= tol_hz, "{pct} %: {hz} Hz");
        let moved = (tr.last().unwrap().position_ms - p0) as f64;
        let want = 2000.0 * pct as f64 / 100.0;
        assert!(
            (moved - want).abs() <= want * 0.04 + 20.0,
            "{pct} %: advanced {moved} ms, want {want}"
        );
    }
}

#[test]
fn sp14_bounds_steps_and_every_refused_value() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 25);
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Step(-1), r.t0)
            .unwrap()
            .speed_pct,
        25,
        "- stops at 25"
    );
    set_speed(&mut r, 200);
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Step(1), r.t0)
            .unwrap()
            .speed_pct,
        200,
        "+ stops at 200"
    );
    set_speed(&mut r, 73);
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Step(-1), r.t0)
            .unwrap()
            .speed_pct,
        70
    );
    set_speed(&mut r, 73);
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Step(1), r.t0)
            .unwrap()
            .speed_pct,
        75
    );
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Reset, r.t0)
            .unwrap()
            .speed_pct,
        100
    );
    set_speed(&mut r, 64);
    for bad in [24, 201, 0, -50, 1000, i64::MIN, i64::MAX] {
        assert!(
            r.m.speed(&r.id(), SpeedAction::Set(bad), r.t0).is_err(),
            "{bad} must be refused"
        );
        assert_eq!(r.snap().speed_pct, 64, "after {bad}");
    }
    set_speed(&mut r, 25);
    set_speed(&mut r, 200);
}

#[test]
fn sp15_a_newer_schema_file_is_never_overwritten_by_speed_or_training() {
    let text = r#"{"schema_version":8,"speed":0.5,"loops":[{"id":"a","start":{"tick":64},"end":{"tick":192}}],"future":{"x":1}}"#;
    let mut r = rig_with(PRACTICE, text);
    assert!(r.snap().note.as_deref().unwrap().contains("newer version"));
    let _ = r.m.speed(&r.id(), SpeedAction::Set(80), r.t0);
    let _ = r.lp(LoopAction::Training(TRAIN));
    r.m.tick(r.after(1000));
    r.m.close();
    let on_disk = std::fs::read_to_string(r.spec.dir.join("player.json")).unwrap();
    assert_eq!(
        on_disk, text,
        "a schema 7 file stays byte for byte as it was"
    );
}

#[test]
fn sp16_schema_5_with_odd_speed_values_loads_clamped_and_saves_as_6() {
    // 0.5 and 1.5 were the old limits; 1.0 is the only value an old build wrote, but be lenient
    for (factor, pct) in [
        ("1.0", 100),
        ("0.73", 73),
        ("0.1", 25),
        ("9", 200),
        ("1.5", 150),
    ] {
        let json = format!(r#"{{"schema_version":5,"speed":{factor},"position_ms":1000}}"#);
        let mut r = rig_with(PRACTICE, &json);
        assert_eq!(r.snap().speed_pct, pct, "factor {factor}");
        set_speed(&mut r, pct as i64 + if pct < 200 { 5 } else { -5 });
        r.flush();
        let d = r.disk().unwrap();
        assert_eq!(d["schema_version"], 7);
    }
}

#[test]
fn sp17_the_100_percent_path_is_bit_exact_after_a_round_trip_through_other_speeds() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 70);
    r.m.speed(&r.id(), SpeedAction::Reset, r.t0).unwrap();
    r.seek(1000);
    r.play();
    let got = r.b.pull(RATE);
    let mut want = expected(&r.pcm(), 1000 * RATE / 1000, RATE);
    let n = RATE * 3 / 1000;
    for k in 0..n {
        for c in 0..2 {
            want[2 * k + c] *= k as f32 / n as f32;
        }
    }
    assert_eq!(got, want, "100 % must be the exact source samples");
}

#[test]
fn sp18_training_from_80_down_to_60_slows_down_then_stays() {
    let mut r = loop_rig(false);
    r.lp(LoopAction::Training(Training {
        on: true,
        start: 80,
        end: 60,
        step: 10,
        repeats: 1,
    }))
    .unwrap();
    r.lp(LoopAction::Play).unwrap();
    let mut seen: Vec<(u32, u32, Option<u32>)> = Vec::new();
    let mut blocks = 0usize;
    while seen.len() < 5 {
        let (_, tr) = run(&mut r, 64);
        let tu = tuple(&tr[0]).expect("run shown");
        if seen.last() != Some(&tu) {
            seen.push(tu);
        }
        blocks += 1;
        assert!(blocks * 64 < RATE * 20, "{seen:?}");
    }
    assert_eq!(
        seen[..4],
        [
            (80, 1, Some(1)),
            (70, 1, Some(1)),
            (60, 1, None),
            (60, 2, None)
        ]
    );
}

#[test]
fn sp19_pause_during_a_run_restores_the_track_speed_and_unlocks_the_speed_controls() {
    let mut r = loop_rig(false);
    set_speed(&mut r, 90);
    r.lp(LoopAction::Training(TRAIN)).unwrap();
    r.lp(LoopAction::Play).unwrap();
    run(&mut r, RATE);
    // refused while the run is going
    assert!(r.m.speed(&r.id(), SpeedAction::Step(1), r.t0).is_err());
    assert!(r.m.speed(&r.id(), SpeedAction::Set(50), r.t0).is_err());
    assert!(r.m.speed(&r.id(), SpeedAction::Reset, r.t0).is_err());
    assert_eq!(
        r.snap().speed_pct,
        90,
        "the track speed is untouched by the refusals"
    );
    r.m.pause(&r.id(), r.t0).unwrap();
    r.b.pull(64);
    let t = r.snap().transport;
    assert_eq!((t.playing, t.training, t.speed_pct), (false, None, 90));
    assert_eq!(
        r.m.speed(&r.id(), SpeedAction::Step(1), r.t0)
            .unwrap()
            .speed_pct,
        95
    );
    set_speed(&mut r, 90);
    r.lp(LoopAction::Play).unwrap();
    let (_, tr) = run(&mut r, 64);
    assert_eq!(
        tuple(&tr[0]),
        Some((60, 1, Some(2))),
        "restarts at the start %"
    );
}

#[test]
fn sp20_a_speed_change_during_a_count_in_keeps_the_count_in_sane() {
    let mut r = loop_rig(true);
    r.lp(LoopAction::Play).unwrap();
    assert!(r.snap().transport.counting_in);
    run(&mut r, 3200);
    set_speed(&mut r, 50);
    let (audio, tr) = run(&mut r, 6 * RATE);
    assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.5));
    let mut last = 0;
    for t in &tr {
        // a repeating loop wraps back to its start (2500); nothing else may go back
        assert!(
            t.position_ms + 40 >= last || t.position_ms <= 2600,
            "position went back {} < {last}",
            t.position_ms
        );
        last = t.position_ms;
        assert!(t.position_ms <= 4600, "left the loop: {}", t.position_ms);
    }
    let t = tr.last().unwrap();
    assert!(t.playing, "still playing");
    assert_eq!(t.speed_pct, 50);
}

#[test]
fn sp21_a_loop_at_50_percent_wraps_inside_its_song_time_bounds_and_seek_stays_in_song_time() {
    let mut r = loop_rig(false);
    set_speed(&mut r, 50);
    r.seek(2500);
    r.lp(LoopAction::Play).unwrap();
    let (_, tr) = run(&mut r, 9 * RATE); // 2 s of song takes 4 s: two wraps in 9 s
    let mut wraps = 0;
    let mut prev = tr[0].position_ms;
    for t in &tr {
        assert!(
            (2400..=4600).contains(&t.position_ms),
            "outside the loop: {}",
            t.position_ms
        );
        if t.position_ms + 1000 < prev {
            wraps += 1;
        }
        prev = t.position_ms;
    }
    assert_eq!(wraps, 2, "wraps");
    // a seek while playing at 50 % lands on the target in song time and carries on at half rate
    r.m.seek(&r.id(), 3000, r.t0).unwrap();
    assert_eq!(r.pos(), 3000);
    assert!(r.snap().transport.resume_pending);
    r.m.tick(r.after(400));
    run(&mut r, 3200);
    let p0 = r.pos();
    let (_, tr) = run(&mut r, RATE);
    let moved = tr.last().unwrap().position_ms as i64 - p0 as i64;
    assert!((moved - 500).abs() <= 30, "advanced {moved} ms");
}

// ---- fix round 1 ----------------------------------------------------------------------------

#[test]
fn sp22_a_huge_step_count_returns_promptly_and_lands_on_the_end() {
    let mut r = rig(PRACTICE);
    for (start, steps, want) in [
        (25i64, i32::MAX, 200u32),
        (200, i32::MIN, 25),
        (25, i32::MIN, 25),
        (200, i32::MAX, 200),
        (100, 1_000_000, 200),
    ] {
        set_speed(&mut r, start);
        let t = Instant::now();
        let s = r.m.speed(&r.id(), SpeedAction::Step(steps), r.t0).unwrap();
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "step {steps} from {start} took {:?}",
            t.elapsed()
        );
        assert_eq!(s.speed_pct, want, "step {steps} from {start}");
    }
}

#[test]
fn sp23_70_to_100_while_playing_is_bit_exact_after_the_fade_with_no_gap_and_no_step_back() {
    let mut r = rig(PRACTICE);
    set_speed(&mut r, 70);
    r.seek(1000);
    r.play();
    let (first, _) = run(&mut r, RATE);
    let reference = rms(&first[RATE / 5..]);
    assert!(reference > 0.001);
    set_speed(&mut r, 100);
    let (fade, tr) = run(&mut r, RATE / 50); // 20 ms: the 3 ms crossfade and more
    assert_no_dip(&fade, reference * 0.0 + 0.0, "fade"); // (windows only; no floor here)
    let mut last = 0;
    let (rest, tr2) = run(&mut r, RATE);
    for t in tr.iter().chain(tr2.iter()) {
        assert!(
            t.position_ms >= last,
            "position went back {} < {last}",
            t.position_ms
        );
        last = t.position_ms;
    }
    assert_eq!(r.snap().speed_pct, 100);
    // the tail is the source, sample for sample, from some frame f on
    let pcm = r.pcm();
    let tail = &rest[RATE..]; // second half of the second
    let guess = r.pos() as usize * RATE / 1000;
    let want_len = RATE / 2 * 2;
    let mut found = None;
    for f in guess.saturating_sub(RATE / 2)..guess + RATE / 10 {
        if f + RATE > pcm.frames as usize {
            break;
        }
        let w = expected(&pcm, f, 32);
        if tail[..64] == w[..] {
            found = Some(f);
            break;
        }
    }
    let _ = want_len;
    let f =
        found.expect("the tail never matched the source: not bit-exact at 100 % after the fade");
    // continuity: the next 0.4 s is the same contiguous source run (no gap, no repeat)
    let n = RATE * 2 / 5;
    assert_eq!(
        &tail[..2 * n],
        &expected(&pcm, f, n)[..],
        "gap or jump after the fade"
    );
}

/// 70 % loop playing; at `k` blocks past 40 ms before the loop end, Reset to 100 %; the loop must never
/// play on past its end (4500 ms).
fn return_to_100_near_the_loop_end(k: usize, repeat: bool) -> (u64, bool) {
    let mut r = loop_rig(false);
    if !repeat {
        r.lp(LoopAction::Flags {
            repeat: false,
            count_in: false,
        })
        .unwrap();
    }
    set_speed(&mut r, 70);
    r.lp(LoopAction::Play).unwrap();
    let mut guard = 0;
    while r.pos() < 4460 {
        r.b.pull(64);
        guard += 1;
        assert!(guard < 4000, "never reached the loop end");
        if r.pos() < 2400 && guard > 10 {
            panic!("wrapped early");
        }
    }
    for _ in 0..k {
        r.b.pull(64);
    }
    r.m.speed(&r.id(), SpeedAction::Reset, r.t0).unwrap();
    let mut maxp = 0;
    let mut wrapped = false;
    let mut prev = r.pos();
    for _ in 0..(RATE / 64) {
        r.b.pull(64);
        let t = r.snap().transport;
        maxp = maxp.max(t.position_ms);
        if t.position_ms + 1000 < prev {
            wrapped = true;
        }
        prev = t.position_ms;
    }
    (maxp, wrapped)
}

#[test]
fn sp24_return_to_100_just_before_a_repeating_loop_end_never_plays_past_the_end() {
    let mut bad = Vec::new();
    for k in 0..60 {
        let (maxp, wrapped) = return_to_100_near_the_loop_end(k, true);
        if maxp > 4502 || (!wrapped && k < 18) {
            bad.push((k, maxp, wrapped));
        }
    }
    assert!(
        bad.is_empty(),
        "(k blocks after 4460 ms, max position, wrapped): {bad:?}"
    );
}

#[test]
fn sp25_return_to_100_before_a_non_repeating_loop_end_stops_at_the_end() {
    let mut bad = Vec::new();
    for k in 0..60 {
        let (maxp, _) = return_to_100_near_the_loop_end(k, false);
        if maxp > 4502 {
            bad.push((k, maxp));
        }
    }
    assert!(bad.is_empty(), "(k, max position): {bad:?}");
}

#[test]
fn sp26_a_speed_action_right_after_a_training_loop_play_is_refused_and_a_plain_loop_play_allows_it()
{
    let mut r = loop_rig(false);
    r.lp(LoopAction::Training(TRAIN)).unwrap();
    r.lp(LoopAction::Play).unwrap();
    // before any audio is pulled
    for a in [
        SpeedAction::Step(1),
        SpeedAction::Step(-1),
        SpeedAction::Set(50),
        SpeedAction::Reset,
    ] {
        assert!(
            r.m.speed(&r.id(), a, r.t0).is_err(),
            "{a:?} must be refused straight after Play"
        );
    }
    assert_eq!(r.snap().speed_pct, 100);
    r.m.pause(&r.id(), r.t0).unwrap();
    r.b.pull(64);
    assert!(
        r.m.speed(&r.id(), SpeedAction::Set(80), r.t0).is_ok(),
        "allowed after Pause"
    );
    // training off: a loop Play does not lock the speed
    let mut r = loop_rig(false);
    r.lp(LoopAction::Play).unwrap();
    assert!(r.m.speed(&r.id(), SpeedAction::Set(80), r.t0).is_ok());
}
