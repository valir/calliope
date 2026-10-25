//! Editor GUI end-to-end tests. They need an X display and are run with:
//! `npm run test:gui` (or
//! `cargo test --features e2e-hooks --test gui_editor_e2e -- --ignored --test-threads=1`).
//!
//! Every test works on a copy of `tests/fixtures/library-editor` under `target/gui-e2e/<test>/`.
//! The audio goes to the capturing fake backend (a float32 WAV that the tests analyse); the
//! real sound device is never opened. Without the `e2e-hooks` feature the app would open the
//! real device, so the whole file is compiled out then (and `start_editor` asserts it too).
#![cfg(feature = "e2e-hooks")]

mod common;

use common::*;
use std::time::{Duration, Instant};

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg") && have("ffmpeg")
}

// Positions in the floated 1280x800 window (Editor, four lanes). The layout does not shift
// (the message slot, the solo name and the CLIP badge have reserved space), so one set serves
// the whole test.
const LANE_Y: [i32; 4] = [100, 183, 266, 349]; // vocals, drums, bass, guitar (all four fully visible at 1280x800)
const CHECK_X: i32 = 845;
const LANE_PLAY_X: i32 = 775;
const SLIDER_END_X: i32 = 1120;
const MIX_PLAY: (i32, i32) = (834, 520);
const TIME: (i32, i32) = (1178, 580);
const SAVE: (i32, i32) = (704, 640);

fn select(app: &mut App, wid: &str, query: &str, id: &str) {
    app.key(wid, "ctrl+f ctrl+a");
    app.typ(wid, query);
    settle();
    app.key(wid, "Down Down Down Return");
    app.wait_line(
        |l| l.trim_end().ends_with(&format!("select id={id}")),
        Duration::from_secs(10),
    );
    settle();
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn finish(app: &App) {
    app.no_csp_violation();
    app.assert_no_real_audio();
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
    assert_eq!(
        app.count("editor play"),
        0,
        "Mix Play must be disabled with nothing checked"
    );

    // Six lanes.
    select(&mut app, &wid, "six lanes", ED_SIX);
    let l = app.wait_contains("editor active");
    assert_eq!(field(&l, "stems"), "6", "{l}");
    settle();
    shot("editor-six-lanes");
    // The smallest supported window: the lane list scrolls, the Mix lane stays whole.
    size(&wid, 1024, 640);
    settle();
    shot("editor-six-lanes-1024x640");
    size(&wid, 1280, 800);

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
        let l = app.wait_line(
            |l| l.contains(&format!("editor stem name={name}")),
            secs(10),
        );
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
    let (p440, p220, p660) = (
        tone_power(&mix, rate, 440.0),
        tone_power(&mix, rate, 220.0),
        tone_power(&mix, rate, 660.0),
    );
    assert!(
        db(p440, p660) > 40.0 && db(p220, p660) > 40.0,
        "mix of vocals and drums: 440={p440} 220={p220} 660={p660}"
    );

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
    assert_eq!(
        app.count("editor transport"),
        n,
        "the position moved while paused"
    );

    // Lane Play on guitar: solo, and the guitar checkbox becomes checked.
    app.click(&wid, LANE_PLAY_X, LANE_Y[3]);
    app.wait_line(|l| l.contains("editor solo name=guitar"), secs(10));
    settle();
    shot("editor-solo");
    let start = capture_len(&cap);
    std::thread::sleep(Duration::from_millis(1200));
    let solo = capture_from(&cap, start);
    let (p440, p660) = (
        tone_power(&solo, rate, 440.0),
        tone_power(&solo, rate, 660.0),
    );
    assert!(db(p660, p440) > 40.0, "solo guitar: 660={p660} 440={p440}");

    // Mix Pause keeps the solo; the following Mix Play ends it (the mix returns).
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    let l = app.wait_line(
        |l| l.contains("calliope: editor transport playing=false"),
        secs(10),
    );
    assert!(l.contains("solo=guitar"), "{l}");
    app.wait_line(|l| l.contains("editor pause"), secs(10));
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    let l = app.wait_line(
        |l| l.contains("calliope: editor transport playing=true"),
        secs(10),
    );
    assert!(l.contains("solo=none"), "{l}");

    // Wheel up over the time field: three nudges of -100 ms, a pause, then the sound resumes
    // about 0.3 s after the last one.
    app.wheel(&wid, TIME.0, TIME.1, true, 3);
    for _ in 0..3 {
        app.wait_line(|l| l.contains("editor nudge delta=-100"), secs(10));
    }
    let last = Instant::now();
    app.wait_line(
        |l| l.contains("calliope: editor transport") && l.contains("audible=true"),
        secs(5),
    );
    let gap = last.elapsed();
    assert!(
        gap > Duration::from_millis(150) && gap < Duration::from_millis(900),
        "audible again after {gap:?}"
    );
    let lines = app.all_lines();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("calliope: editor transport") && l.contains("audible=false")),
        "no audible=false line during the nudges"
    );
    assert_eq!(app.count("editor nudge delta=-100"), 3);

    // Leaving the Editor while playing stops the playback.
    app.key(&wid, "alt+1");
    app.wait_contains("editor leave stop");
    app.wait_line(
        |l| l.contains("calliope: editor transport playing=false"),
        secs(5),
    );

    // Back in the Editor: uncheck guitar (it was checked by the solo) and Save.
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    settle();
    app.click(&wid, CHECK_X, LANE_Y[3]);
    let l = app.wait_line(|l| l.contains("editor stem name=guitar"), secs(10));
    assert!(
        l.contains("unmuted=false"),
        "guitar was not checked by the solo: {l}"
    );
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
    let on: Vec<&str> = stems
        .iter()
        .filter(|s| s["unmuted"] == true)
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(on, ["vocals", "drums"], "{b}");
    let samples = decode(&flac);
    let brate = b["sample_rate"].as_f64().unwrap();
    let (p440, p220, p660) = (
        tone_power(&samples, brate, 440.0),
        tone_power(&samples, brate, 220.0),
        tone_power(&samples, brate, 660.0),
    );
    assert!(db(p660, p440) < -40.0, "saved file: 660={p660} 440={p440}");
    assert!(
        db(p220, p440) > -20.0,
        "drums missing from the saved file: 220={p220} 440={p440}"
    );
    assert!(
        dirs.trash().iter().all(|t| !t.ends_with("-backing")),
        "first save trashed something"
    );

    // Second Save: vocals, drums and bass pushed to +12 dB. The CLIP badge lights and the old
    // file goes to the trash.
    app.click(&wid, CHECK_X, LANE_Y[2]);
    app.wait_line(|l| l.contains("editor stem name=bass"), secs(10));
    for y in [LANE_Y[0], LANE_Y[1], LANE_Y[2]] {
        app.click(&wid, SLIDER_END_X, y);
        app.key(&wid, "End");
    }
    for name in ["vocals", "drums", "bass"] {
        app.wait_line(
            |l| l.contains(&format!("editor stem name={name} gain=12")),
            secs(10),
        );
    }
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    // The backend's clipping line can come before or after the frontend's "editor play" line, and
    // wait_line consumes lines in order, so only the clipping line is waited for.
    app.wait_line(
        |l| l.contains("calliope: editor transport") && l.contains("clipping=true"),
        secs(10),
    );
    settle();
    shot("editor-clip");
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    app.wait_line(|l| l.contains("editor pause"), secs(10));
    let old = std::fs::read(&flac).unwrap();
    app.click(&wid, SAVE.0, SAVE.1);
    let l = app.wait_line(|l| l.contains("editor saved"), secs(60));
    let clipped: u64 = field(&l, "clipped").parse().unwrap();
    assert!(clipped > 0, "{l}");
    settle();
    shot("editor-saved-clipped");
    let trashed: Vec<String> = dirs
        .trash()
        .into_iter()
        .filter(|t| t.ends_with("-backing"))
        .collect();
    assert_eq!(trashed.len(), 1, "{:?}", dirs.trash());
    let moved = dirs
        .repo()
        .join("trash")
        .join(&trashed[0])
        .join("backing.flac");
    assert_eq!(
        std::fs::read(moved).unwrap(),
        old,
        "the old backing is not in the trash"
    );
    assert_ne!(std::fs::read(&flac).unwrap(), old);
    finish(&app);
}

// ---- Mix waveform and tempo detection (the Editor with a calliope-stems server) ----

// The Mix lane in the floated 1280x800 window (it sits at the bottom, so it does not depend on
// the number of lanes). The waveform canvas spans x = WAVE_X0 .. WAVE_X0 + WAVE_W.
const WAVE_X0: i32 = 641;
const WAVE_W: i32 = 610;
const WAVE_Y: i32 = 453;
const AUTODETECT: (i32, i32) = (1150, 520);
const TRACK_MS: f64 = 12000.0; // length of the Four Lanes stems
const CHANGE_MS: f64 = 4500.0; // where the stub's `ok` mode changes from 120 to 90 BPM there
const FINE_MS: f64 = 7.8125; // 1/64 beat at 120 BPM

fn wave_x(ms: f64) -> i32 {
    WAVE_X0 + (ms / TRACK_MS * (WAVE_W - 1) as f64).round() as i32
}

/// The tempo changes of `player.json` (`tempo.segments`), waiting up to 4 s for `ok`.
fn wait_segments(
    dirs: &Dirs,
    id: &str,
    ok: impl Fn(&[serde_json::Value]) -> bool,
) -> Vec<serde_json::Value> {
    let end = Instant::now() + secs(4);
    loop {
        if let Ok(b) = std::fs::read(dirs.player_json(id)) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                let segs = v["tempo"]["segments"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if ok(&segs) {
                    return segs;
                }
            }
        }
        assert!(
            Instant::now() < end,
            "player.json of {id} did not reach the expected state"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn start_ms(seg: &serde_json::Value) -> f64 {
    seg["start_ms"].as_f64().unwrap()
}

fn peaks_max(line: &str) -> f64 {
    field(line, "max").parse().unwrap()
}

#[test]
#[ignore]
fn mix_waveform_and_autodetect() {
    if !ready() {
        return;
    }
    let dirs = editor_dirs("editor_mix_waveform_autodetect");
    let (mut app, wid) = start_detect_app(&dirs, "ok");
    select(&mut app, &wid, "four lanes", ED_FOUR);
    app.wait_contains("editor active");
    settle();

    // A checked stem: the Mix waveform shows its peaks soon after.
    app.click(&wid, CHECK_X, LANE_Y[0]);
    app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) > 0.0,
        secs(3),
    );

    // Three quick toggles: one recomputation, no earlier than 300 ms after the last change,
    // while a Play is not held up by it.
    let before = app.count("editor peaks id=");
    app.click(&wid, CHECK_X, LANE_Y[1]);
    app.click(&wid, CHECK_X, LANE_Y[2]);
    let last = Instant::now();
    app.click(&wid, CHECK_X, LANE_Y[1]);
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    app.wait_line(|l| l.contains("editor play"), secs(10));
    let played = Instant::now();
    app.wait_line(|l| l.contains("editor waveform peaks="), secs(5));
    let gap = last.elapsed();
    assert!(
        gap >= Duration::from_millis(300),
        "the peaks came {gap:?} after the last change"
    );
    assert!(played < last + gap, "Play waited for the peaks");
    std::thread::sleep(secs(1));
    assert_eq!(
        app.count("editor peaks id="),
        before + 1,
        "three toggles must give one recomputation"
    );
    app.click(&wid, MIX_PLAY.0, MIX_PLAY.1);
    app.wait_line(|l| l.contains("editor pause"), secs(10));

    // Nothing checked: a flat waveform (Autodetect does not depend on the checked stems).
    for y in [LANE_Y[0], LANE_Y[2]] {
        app.click(&wid, CHECK_X, y);
    }
    let l = app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) == 0.0,
        secs(5),
    );
    assert_eq!(field(&l, "max"), "0.00", "{l}");
    settle();
    shot("editor-mix-nothing-checked");

    // Autodetect with the stems checked: phases, two tempo sections, player.json schema 3.
    for y in [LANE_Y[0], LANE_Y[1]] {
        app.click(&wid, CHECK_X, y);
    }
    app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) > 0.0,
        secs(5),
    );
    settle();
    app.click(&wid, AUTODETECT.0, AUTODETECT.1);
    app.wait_contains("editor detect start");
    let l = app.wait_line(|l| l.contains("editor detected segments="), secs(30));
    assert_eq!(field(&l, "segments"), "2", "{l}");
    assert_eq!(field(&l, "bpm"), "120,90", "{l}");
    let phases: Vec<String> = app
        .all_lines()
        .iter()
        .filter(|l| l.contains("editor detect phase="))
        .map(|l| field(l, "phase"))
        .collect();
    assert!(
        phases.len() >= 2 && phases.contains(&"detecting".to_string()),
        "phases {phases:?}"
    );
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dirs.player_json(ED_FOUR)).unwrap()).unwrap();
    assert_eq!(v["schema_version"], 7, "{v}");
    let segs = wait_segments(&dirs, ED_FOUR, |s| s.len() == 2);
    assert_eq!(start_ms(&segs[1]), CHANGE_MS, "{segs:?}");
    assert_eq!(segs[1]["detected_ms"].as_f64(), Some(CHANGE_MS), "{segs:?}");
    settle();
    shot("editor-mix-detected-dark");

    // The light theme (Settings, Alt+7, is not needed: the theme is a settings.json value).
    // Keys on the waveform: Shift+Right moves by a beat, Right by 1/64 beat.
    app.click(&wid, wave_x(2000.0), WAVE_Y); // focuses the canvas, away from the bar
    app.key(&wid, "shift+Right");
    let l = app.wait_line(|l| l.contains("editor tempo move"), secs(10));
    assert_eq!(field(&l, "applied"), "64", "{l}");
    app.key(&wid, "Right");
    let l = app.wait_line(|l| l.contains("editor tempo move"), secs(10));
    assert_eq!(field(&l, "applied"), "65", "{l}");
    let segs = wait_segments(&dirs, ED_FOUR, |s| {
        s.len() == 2 && (start_ms(&s[1]) - (CHANGE_MS + 65.0 * FINE_MS)).abs() < 0.01
    });
    assert_eq!(segs[1]["detected_ms"].as_f64(), Some(CHANGE_MS), "{segs:?}");
    settle();
    shot("editor-mix-moved");

    // Four more beats hit the limit of 256 steps (4 beats) and stay there.
    for _ in 0..4 {
        app.key(&wid, "shift+Right");
    }
    app.wait_line(
        |l| l.contains("editor tempo move") && field(l, "applied") == "256",
        secs(10),
    );
    let segs = wait_segments(&dirs, ED_FOUR, |s| {
        s.len() == 2 && (start_ms(&s[1]) - (CHANGE_MS + 256.0 * FINE_MS)).abs() < 0.01
    });
    assert_eq!(segs[1]["detected_ms"].as_f64(), Some(CHANGE_MS), "{segs:?}");

    // Dragging the bar 40 px back.
    let bar = wave_x(CHANGE_MS + 256.0 * FINE_MS) + 1;
    app.skip_seen();
    app.drag(&wid, bar, bar - 40, WAVE_Y, || {
        std::thread::sleep(Duration::from_millis(300));
        shot("editor-mix-drag-readout");
    });
    let l = app.wait_line(|l| l.contains("editor tempo move"), secs(10));
    let applied: i64 = field(&l, "applied").parse().unwrap();
    assert!(applied.abs() <= 256 && applied < 256, "{l}");
    settle();
    shot("editor-mix-dragged");

    // The Player takes the tempo: BPM 120, the bar in its waveform, no Detect button.
    app.key(&wid, "alt+5");
    app.wait_view("player");
    app.wait_line(|l| l.contains("player active"), secs(20));
    settle();
    shot("player-detected-sections");
    finish(&app);

    // The same track in the light theme, after a restart: the tempo comes back from player.json.
    drop(app);
    std::fs::write(
        dirs.app_config().join("settings.json"),
        r#"{"theme":"light"}"#,
    )
    .unwrap();
    let (mut app, wid) = start_detect_app(&dirs, "ok");
    select(&mut app, &wid, "four lanes", ED_FOUR);
    app.wait_contains("editor active");
    app.click(&wid, CHECK_X, LANE_Y[0]);
    app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) > 0.0,
        secs(5),
    );
    settle();
    shot("editor-mix-detected-light");
    finish(&app);
}

const TAB_TABLATURE: (i32, i32) = (737, 36);
const TAB_STEMS: (i32, i32) = (664, 36);
const PLAYER_LOOP_ADD: (i32, i32) = (1009, 368); // 1280x800, track tree collapsed

/// Waits until no process of the test runs the stub beat tracker (up to 5 s).
fn assert_no_tracker(app: &App) {
    let end = Instant::now() + secs(5);
    loop {
        let left: Vec<_> = app
            .leftovers()
            .into_iter()
            .filter(|(_, cmd)| {
                runs_program(cmd, "stub-beat-tracker") && !runs_program(cmd, "calliope-stems")
            })
            .collect();
        if left.is_empty() {
            return;
        }
        assert!(
            Instant::now() < end,
            "the stub beat tracker is still running: {left:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore]
fn autodetect_errors_cancel_and_player_reload() {
    if !ready() {
        return;
    }
    let dirs = editor_dirs("editor_autodetect_errors_cancel");
    let (mut app, wid) = start_detect_app(&dirs, "steady");
    select(&mut app, &wid, "four lanes", ED_FOUR);
    app.wait_contains("editor active");

    // A loop in the Player first.
    app.key(&wid, "alt+5");
    app.wait_view("player");
    app.wait_line(|l| l.contains("player active"), secs(20));
    app.key(&wid, "ctrl+l");
    app.wait_contains("player tree collapsed=true");
    settle();
    app.click(&wid, PLAYER_LOOP_ADD.0, PLAYER_LOOP_ADD.1);
    app.wait_contains("player loop add");
    let loops = |dirs: &Dirs| -> usize {
        let end = Instant::now() + secs(4);
        loop {
            if let Ok(b) = std::fs::read(dirs.player_json(ED_FOUR)) {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                    if let Some(l) = v["loops"].as_array().filter(|l| !l.is_empty()) {
                        return l.len();
                    }
                }
            }
            assert!(Instant::now() < end, "the loop was not saved");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert_eq!(loops(&dirs), 1);

    // Autodetect in the Editor releases the Player; coming back reopens it with the loop.
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    settle();
    app.click(&wid, CHECK_X, LANE_Y[0]);
    app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) > 0.0,
        secs(5),
    );
    app.click(&wid, AUTODETECT.0, AUTODETECT.1);
    app.wait_contains("player released");
    let l = app.wait_line(|l| l.contains("editor detected segments="), secs(30));
    assert_eq!(field(&l, "segments"), "1", "{l}");
    assert_eq!(field(&l, "bpm"), "120", "{l}");
    app.key(&wid, "alt+5");
    app.wait_view("player");
    app.wait_line(|l| l.contains("player active"), secs(20));
    settle();
    shot("player-reopened-with-loop");
    assert_eq!(loops(&dirs), 1, "the loop is gone after the reopen");
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dirs.player_json(ED_FOUR)).unwrap()).unwrap();
    assert_eq!(
        v["tempo"]["segments"][0]["bpm"].as_f64(),
        Some(120.0),
        "{v}"
    );

    // A failing tracker: the message appears in the Editor.
    set_tracker_mode(&dirs, "fail");
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    settle();
    app.click(&wid, AUTODETECT.0, AUTODETECT.1);
    let l = app.wait_line(|l| l.contains("editor detect error message="), secs(30));
    assert!(l.contains("tracker") || l.contains("beat"), "{l}");
    settle();
    shot("editor-detect-error");

    // A hanging tracker: the phase text and Cancel, then Cancel stops it.
    set_tracker_mode(&dirs, "hang");
    app.click(&wid, AUTODETECT.0, AUTODETECT.1);
    app.wait_line(|l| l.contains("editor detect phase=detecting"), secs(20));
    settle();
    shot("editor-detect-running");
    app.click(&wid, AUTODETECT.0, AUTODETECT.1); // the button is Cancel now
    app.wait_contains("editor detect cancelled");
    assert_no_tracker(&app);
    settle();
    shot("editor-detect-cancelled");

    // The other Editor tabs are placeholders.
    app.click(&wid, TAB_TABLATURE.0, TAB_TABLATURE.1);
    settle();
    shot("editor-tab-tablature");
    app.click(&wid, TAB_STEMS.0, TAB_STEMS.1);
    finish(&app);
    drop(app);

    // Without an edge-AI server in the settings the button is disabled and says why.
    let dirs = editor_dirs("editor_autodetect_no_server");
    let (mut app, wid) = start_editor(&dirs);
    select(&mut app, &wid, "four lanes", ED_FOUR);
    app.wait_contains("editor active");
    app.click(&wid, CHECK_X, LANE_Y[0]);
    app.wait_line(
        |l| l.contains("editor waveform peaks=") && peaks_max(l) > 0.0,
        secs(5),
    );
    settle();
    shot("editor-autodetect-no-server");
    app.click(&wid, AUTODETECT.0, AUTODETECT.1);
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        app.count("editor detect start"),
        0,
        "Autodetect must be disabled without a server"
    );
    finish(&app);
}
