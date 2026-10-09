//! Editor GUI end-to-end tests. They need an X display and are run with:
//! `DISPLAY=:1 npm run test:gui` (or
//! `cargo test --features e2e-hooks --test gui_editor_e2e -- --ignored --test-threads=1`).
//!
//! Every test works on a copy of `tests/fixtures/library-editor` under `target/gui-e2e/<test>/`.
//! The audio goes to the capturing fake backend (a float32 WAV that the tests analyse); the
//! real sound device is never opened.

mod common;

use common::*;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg") && have("ffmpeg")
}

// Positions in the floated 1280x800 window (Editor, four lanes).
const LANE_Y: [i32; 4] = [102, 194, 286, 378]; // vocals, drums, bass, guitar
const CHECK_X: i32 = 903;
const LANE_PLAY_X: i32 = 819;
const SLIDER_END_X: i32 = 1123;
const MIX_PLAY: (i32, i32) = (856, 554);
const TIME: (i32, i32) = (1170, 625);
const SAVE: (i32, i32) = (712, 691);
// After a save the "Saved." line makes the Mix lane taller and the lane moves up by 45 px.
const MIX_PLAY_MSG: (i32, i32) = (856, 510);
const SAVE_MSG: (i32, i32) = (712, 646);

fn select(app: &mut App, wid: &str, query: &str, id: &str) {
    app.key(wid, "ctrl+f ctrl+a");
    app.typ(wid, query);
    settle();
    app.key(wid, "Down Down Down Return");
    app.wait_line(|l| l.trim_end().ends_with(&format!("select id={id}")), Duration::from_secs(10));
    settle();
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn finish(app: &App) {
    app.no_csp_violation();
    app.assert_no_real_audio();
}

// ---- audio analysis ----

/// Interleaved stereo f32 samples of the capture WAV from byte `from` of the file (the header
/// is 44 bytes; its sizes are only patched when the stream closes, so they are ignored).
fn capture_from(path: &Path, from: usize) -> Vec<f32> {
    let b = std::fs::read(path).unwrap();
    let from = from.max(44) & !7;
    b[from.min(b.len())..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn capture_len(path: &Path) -> usize {
    std::fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0)
}

/// Mean power of `freq` in the left channel (Goertzel).
fn tone_power(stereo: &[f32], rate: f64, freq: f64) -> f64 {
    let left: Vec<f64> = stereo.chunks_exact(2).map(|c| f64::from(c[0])).collect();
    let n = left.len();
    assert!(n > 1000, "too little audio: {n} frames");
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for x in &left {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    p / (n as f64 * n as f64)
}

fn db(a: f64, b: f64) -> f64 {
    10.0 * ((a + 1e-30) / (b + 1e-30)).log10()
}

/// Decodes any audio file to interleaved stereo f32 with ffmpeg.
fn decode(path: &Path) -> Vec<f32> {
    let o = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "2", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "ffmpeg decode of {}", path.display());
    o.stdout.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn rate_of(app: &App) -> f64 {
    let l = app.all_lines().into_iter().rev().find(|l| l.contains("audio backend=")).expect("audio backend line");
    field(&l, "rate").parse().unwrap()
}

// ---- tests ----

#[test]
#[ignore]
fn states_and_lanes() {
    if !ready() {
        return;
    }
    let dirs = editor_dirs("editor_states_and_lanes");
    let (mut app, wid) = start_editor(&dirs);
    // The Editor with nothing selected is inactive (logged while starting).
    assert!(app.count("editor inactive id=none") >= 1);

    // A plain backing track has no stems.
    select(&mut app, &wid, "plain backing", ED_PLAIN);
    app.wait_contains(&format!("editor inactive id={ED_PLAIN}"));
    settle();
    shot("editor-inactive");

    // Four lanes: opened by Rust, active in the UI; Mix Play is disabled.
    select(&mut app, &wid, "four lanes", ED_FOUR);
    let l = app.wait_contains("calliope: editor open");
    assert_eq!(field(&l, "duration_ms"), "12000", "{l}");
    assert_eq!(field(&l, "stems"), "4", "{l}");
    let l = app.wait_contains("editor active");
    assert_eq!(field(&l, "stems"), "4", "{l}");
    settle();
    shot("editor-four-lanes");
    app.key(&wid, "space");
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(app.count("editor play"), 0, "Mix Play must be disabled with nothing checked");

    // Six lanes.
    select(&mut app, &wid, "six lanes", ED_SIX);
    let l = app.wait_contains("editor active");
    assert_eq!(field(&l, "stems"), "6", "{l}");
    settle();
    shot("editor-six-lanes");

    // A stem file is missing: inactive, with the message on screen.
    select(&mut app, &wid, "missing stem", ED_MISSING);
    app.wait_contains(&format!("editor inactive id={ED_MISSING}"));
    settle();
    shot("editor-missing-stem");
    assert_eq!(app.count("audio backend="), 0, "nothing was played");
    finish(&app);
}

#[test]
#[ignore]
fn play_solo_nudge_save_clip() {
    if !ready() {
        return;
    }
    let dirs = editor_dirs("editor_play_solo_save");
    let (mut app, wid) = start_editor(&dirs);
    select(&mut app, &wid, "four lanes", ED_FOUR);
    app.wait_contains("editor active");
    settle();

    // Check vocals and drums.
    for (i, name) in ["vocals", "drums"].iter().enumerate() {
        app.click(&wid, CHECK_X, LANE_Y[i]);
        let l = app.wait_line(|l| l.contains(&format!("editor stem name={name}")), secs(10));
        assert!(l.contains("unmuted=true"), "{l}");
    }

    // Mix Play: the capture backend opens and the transport runs for 1.5 s.
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    app.wait_contains("audio backend=capture");
    let l = app.wait_contains("editor play");
    assert!(l.contains("calliope-ui"), "{l}");
    let rate = rate_of(&app);
    let cap = dirs.capture();
    let start = capture_len(&cap);
    std::thread::sleep(Duration::from_millis(1500));
    settle();
    shot("editor-playing");
    let mix = capture_from(&cap, start);
    let (p440, p220, p660) = (tone_power(&mix, rate, 440.0), tone_power(&mix, rate, 220.0), tone_power(&mix, rate, 660.0));
    assert!(db(p440, p660) > 40.0 && db(p220, p660) > 40.0, "mix of vocals and drums: 440={p440} 220={p220} 660={p660}");

    // The moving slider must not seek by itself (it used to, on every transport event).
    assert_eq!(app.count("editor seek"), 0, "playing caused seeks");

    // Pause: the position stops.
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    let l = app.wait_line(|l| l.contains("editor pause position="), secs(10));
    let pos: i64 = field(&l, "position").parse().unwrap();
    assert!((1500..4500).contains(&pos), "paused position {pos} ms");
    assert!(app.count("calliope: editor transport playing=false") >= 1);
    let n = app.count("editor transport");
    std::thread::sleep(secs(1));
    assert_eq!(app.count("editor transport"), n, "the position moved while paused");

    // Lane Play on guitar: solo, and the guitar checkbox becomes checked.
    app.click(&wid, LANE_PLAY_X, LANE_Y[3]);
    app.wait_line(|l| l.contains("editor solo name=guitar"), secs(10));
    settle();
    shot("editor-solo");
    let start = capture_len(&cap);
    std::thread::sleep(Duration::from_millis(1200));
    let solo = capture_from(&cap, start);
    let (p440, p660) = (tone_power(&solo, rate, 440.0), tone_power(&solo, rate, 660.0));
    assert!(db(p660, p440) > 40.0, "solo guitar: 660={p660} 440={p440}");

    // Mix Pause keeps the solo; the following Mix Play ends it (the mix returns).
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    let l = app.wait_line(|l| l.contains("calliope: editor transport playing=false"), secs(10));
    assert!(l.contains("solo=guitar"), "{l}");
    app.wait_line(|l| l.contains("editor pause"), secs(10));
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    let l = app.wait_line(|l| l.contains("calliope: editor transport playing=true"), secs(10));
    assert!(l.contains("solo=none"), "{l}");

    // Wheel up over the time field: three nudges of -100 ms, a pause, then the sound resumes
    // about 0.3 s after the last one.
    app.wheel(&wid, TIME.0, TIME.1, true, 3);
    for _ in 0..3 {
        app.wait_line(|l| l.contains("editor nudge delta=-100"), secs(10));
    }
    let last = Instant::now();
    app.wait_line(|l| l.contains("calliope: editor transport") && l.contains("audible=true"), secs(5));
    let gap = last.elapsed();
    assert!(
        gap > Duration::from_millis(150) && gap < Duration::from_millis(900),
        "audible again after {gap:?}"
    );
    let lines = app.all_lines();
    assert!(
        lines.iter().any(|l| l.contains("calliope: editor transport") && l.contains("audible=false")),
        "no audible=false line during the nudges"
    );
    assert_eq!(app.count("editor nudge delta=-100"), 3);

    // Leaving the Editor while playing stops the playback.
    app.key(&wid, "alt+1");
    app.wait_contains("editor leave stop");
    app.wait_line(|l| l.contains("calliope: editor transport playing=false"), secs(5));

    // Back in the Editor: uncheck guitar (it was checked by the solo) and Save.
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    settle();
    app.click(&wid, CHECK_X, LANE_Y[3]);
    let l = app.wait_line(|l| l.contains("editor stem name=guitar"), secs(10));
    assert!(l.contains("unmuted=false"), "guitar was not checked by the solo: {l}");
    app.click(&wid, SAVE.0, SAVE.1);
    let l = app.wait_line(|l| l.contains("editor saved"), secs(60));
    assert_eq!(field(&l, "clipped"), "0", "{l}");
    settle();
    shot("editor-saved");

    let flac = dirs.track_dir(ED_FOUR).join("backings/backing.flac");
    assert!(flac.is_file());
    let v = dirs.json(ED_FOUR);
    let b = &v["backings"][0];
    assert_eq!(b["file"], "backings/backing.flac", "{v}");
    let stems = b["mix"]["stems"].as_array().unwrap();
    let on: Vec<&str> = stems.iter().filter(|s| s["unmuted"] == true).map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(on, ["vocals", "drums"], "{b}");
    let samples = decode(&flac);
    let brate = b["sample_rate"].as_f64().unwrap();
    let (p440, p220, p660) = (tone_power(&samples, brate, 440.0), tone_power(&samples, brate, 220.0), tone_power(&samples, brate, 660.0));
    assert!(db(p660, p440) < -40.0, "saved file: 660={p660} 440={p440}");
    assert!(db(p220, p440) > -20.0, "drums missing from the saved file: 220={p220} 440={p440}");
    assert!(dirs.trash().iter().all(|t| !t.ends_with("-backing")), "first save trashed something");

    // Second Save: vocals, drums and bass pushed to +12 dB. The CLIP badge lights and the old
    // file goes to the trash.
    app.click(&wid, CHECK_X, LANE_Y[2]);
    app.wait_line(|l| l.contains("editor stem name=bass"), secs(10));
    for y in [LANE_Y[0], LANE_Y[1], LANE_Y[2]] {
        app.click(&wid, SLIDER_END_X, y);
        app.key(&wid, "End");
    }
    for name in ["vocals", "drums", "bass"] {
        app.wait_line(|l| l.contains(&format!("editor stem name={name} gain=12")), secs(10));
    }
    app.click(&wid, MIX_PLAY_MSG.0, MIX_PLAY_MSG.1);
    app.wait_line(|l| l.contains("editor play"), secs(10));
    app.wait_line(|l| l.contains("calliope: editor transport") && l.contains("clipping=true"), secs(10));
    settle();
    shot("editor-clip");
    app.click(&wid, MIX_PLAY_MSG.0, MIX_PLAY_MSG.1);
    app.wait_line(|l| l.contains("editor pause"), secs(10));
    let old = std::fs::read(&flac).unwrap();
    app.click(&wid, SAVE_MSG.0, SAVE_MSG.1);
    let l = app.wait_line(|l| l.contains("editor saved"), secs(60));
    let clipped: u64 = field(&l, "clipped").parse().unwrap();
    assert!(clipped > 0, "{l}");
    settle();
    shot("editor-saved-clipped");
    let trashed: Vec<String> = dirs.trash().into_iter().filter(|t| t.ends_with("-backing")).collect();
    assert_eq!(trashed.len(), 1, "{:?}", dirs.trash());
    let moved = dirs.repo().join("trash").join(&trashed[0]).join("backing.flac");
    assert_eq!(std::fs::read(moved).unwrap(), old, "the old backing is not in the trash");
    assert_ne!(std::fs::read(&flac).unwrap(), old);
    finish(&app);
}
