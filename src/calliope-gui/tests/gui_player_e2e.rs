//! Player GUI end-to-end tests. They need an X display and are run with:
//! `npm run test:gui` (or
//! `cargo test --features e2e-hooks --test gui_player_e2e -- --ignored --test-threads=1`).
//!
//! Every test works on a copy of `tests/fixtures/library-player` under `target/gui-e2e/<test>/`.
//! The audio goes to the capturing fake backend (a float32 WAV that the tests analyse); the
//! real sound device is never opened. Without the `e2e-hooks` feature the whole file is
//! compiled out (and `start_player` asserts it too).
//!
//! The window is floated at 1280x800 and the track tree is collapsed (Ctrl+L) after a track is
//! picked, which gives the control bar a fixed layout; the coordinates below were read from
//! screenshots of that layout.
#![cfg(feature = "e2e-hooks")]

mod common;

use common::*;
use serde_json::Value;
use std::time::{Duration, Instant};

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg")
}

// Positions in the floated 1280x800 window with the tree collapsed.
const BLANK: (i32, i32) = (356, 47); // the "Play:" label: a click there only removes the focus
const TRIGGER: (i32, i32) = (555, 47);
const ITEM_X: i32 = 500;
const ITEM_Y: [i32; 4] = [113, 167, 221, 275]; // Backing, Original mix, Vocals, Guitar
const BPM: (i32, i32) = (445, 272);
const METRO: (i32, i32) = (761, 383);
const METRO_COUNT_IN: (i32, i32) = (867, 383);
const PLAY: (i32, i32) = (477, 495);
const TIME: (i32, i32) = (669, 494);
const LOOP_ADD: (i32, i32) = (457, 634);
const MARK_START: (i32, i32) = (798, 634);
const MARK_END: (i32, i32) = (1203, 634);
const LOOP_PLAY: (i32, i32) = (402, 701);
const LOOP_ON: (i32, i32) = (517, 701);
const LOOP_COUNT_IN: (i32, i32) = (633, 701);
// The Sections group (name box, buttons, the Metronome sub-group and the "This section" toggles), the
// waveform (section 0 | the change bar at 8500 ms | section 1) and its labels.
const SEC_NAME: (i32, i32) = (1043, 266);
const SEC_ADD: (i32, i32) = (643, 320);
const SEC_REMOVE: (i32, i32) = (734, 320);
const SEC_DOUBLE: (i32, i32) = (820, 320);
const SEC_START: (i32, i32) = (965, 320);
const SEC_END: (i32, i32) = (1072, 320);
const SEC_CLICK: (i32, i32) = (1082, 383);
const SEC_COUNT_IN: (i32, i32) = (1179, 383);
const WAVE_BAR: (i32, i32) = (729, 180);
const WAVE_SECTION0: (i32, i32) = (450, 180);
const WAVE_SECTION1: (i32, i32) = (1000, 180);
const WAVE_LABEL0: (i32, i32) = (370, 215);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ms(m: u64) {
    std::thread::sleep(Duration::from_millis(m));
}

fn finish(app: &App) {
    app.no_csp_violation();
    app.assert_no_real_audio();
}

fn at(app: &App, wid: &str, p: (i32, i32)) {
    app.click(wid, p.0, p.1);
}

/// Types `text` into the field at `p` (replacing its content) and presses Enter.
fn enter(app: &App, wid: &str, p: (i32, i32), text: &str) {
    at(app, wid, p);
    app.key(wid, "ctrl+a");
    app.typ(wid, text);
    app.key(wid, "Return");
}

/// Picks a track through the search box (the tree is expanded for that), waits for the Player to
/// become active or inactive, and collapses the tree again. Returns that log line.
fn pick(app: &mut App, wid: &str, collapsed: &mut bool, query: &str, id: &str) -> String {
    if *collapsed {
        app.key(wid, "ctrl+l");
        app.wait_contains("player tree collapsed=false");
        *collapsed = false;
    }
    app.key(wid, "ctrl+f ctrl+a");
    app.typ(wid, query);
    settle();
    app.key(wid, "Down Down Down Return");
    app.wait_line(|l| l.trim_end().ends_with(&format!("select id={id}")), secs(10));
    let l = app.wait_line(|l| l.contains("player active") || l.contains("player inactive"), secs(20));
    app.key(wid, "ctrl+l");
    app.wait_contains("player tree collapsed=true");
    *collapsed = true;
    settle();
    l
}

fn choose_source(app: &App, wid: &str, index: usize) {
    at(app, wid, TRIGGER);
    ms(500);
    app.click(wid, ITEM_X, ITEM_Y[index]);
}

/// Waits (up to 4 s) until `player.json` of `id` satisfies `ok`; returns it.
fn wait_json(dirs: &Dirs, id: &str, ok: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + secs(4);
    loop {
        if let Ok(b) = std::fs::read(dirs.player_json(id)) {
            if let Ok(v) = serde_json::from_slice::<Value>(&b) {
                if ok(&v) {
                    return v;
                }
            }
        }
        assert!(Instant::now() < end, "player.json of {id} did not reach the expected state");
        ms(100);
    }
}

/// A place in the capture: its length and how many streams were opened so far (a new stream
/// starts a new file).
fn mark(app: &App, cap: &std::path::Path) -> (usize, usize) {
    (capture_len(cap), app.count("audio backend=capture"))
}

/// What was captured since `m`.
fn since(app: &App, cap: &std::path::Path, m: (usize, usize)) -> Vec<f32> {
    capture_from(cap, if app.count("audio backend=capture") == m.1 { m.0 } else { 0 })
}

fn position_of(line: &str) -> i64 {
    field(line, "position").parse().unwrap()
}

fn transport(app: &mut App, playing: bool) -> String {
    app.wait_line(|l| l.contains(&format!("calliope: player transport playing={playing}")), secs(15))
}

#[test]
#[ignore]
fn source_and_tree() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_source_and_tree");
    let (mut app, wid) = start_player(&dirs);
    assert!(app.count("player inactive id=none") >= 1, "nothing selected: inactive");
    shot("player-inactive");

    // A track with stems only starts on its first stem at position 0.
    app.key(&wid, "ctrl+f ctrl+a");
    app.typ(&wid, "stems only");
    settle();
    app.key(&wid, "Down Down Down Return");
    let l = app.wait_line(|l| l.contains("player active"), secs(20));
    assert_eq!(field(&l, "source"), "stem:bass", "{l}");
    assert_eq!(field(&l, "position"), "0", "{l}");
    assert_eq!(field(&l, "id"), PL_STEMS, "{l}");

    app.key(&wid, "ctrl+l");
    app.wait_contains("player tree collapsed=true");
    settle();
    shot("player-tree-collapsed");
    app.key(&wid, "ctrl+l");
    app.wait_contains("player tree collapsed=false");

    // A track with a backing opens on it, with the backing, the original and two stems listed.
    app.key(&wid, "ctrl+f ctrl+a");
    app.typ(&wid, "practice song");
    settle();
    app.key(&wid, "Down Down Down Return");
    let l = app.wait_line(|l| l.contains("player active"), secs(20));
    assert_eq!(field(&l, "source"), "backing:backing", "{l}");
    assert_eq!(field(&l, "sources"), "4", "{l}");
    settle();
    shot("player-tree-shown");
    finish(&app);
}

#[test]
#[ignore]
fn play_and_change_source() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_play_source");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let cap = dirs.capture();

    // The backing (330 Hz) plays.
    let start = mark(&app, &cap);
    at(&app, &wid, PLAY);
    app.wait_contains("audio backend=capture");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(2000);
    let heard = since(&app, &cap, start);
    let (p330, p660) = (tone_power(&heard, rate, 330.0), tone_power(&heard, rate, 660.0));
    assert!(db(p330, p660) > 40.0, "backing: 330={p330} 660={p660}");
    shot("player-playing");

    // Choosing Guitar while playing stops the playback and keeps the position.
    app.skip_seen();
    choose_source(&app, &wid, 3);
    transport(&mut app, false);
    app.wait_contains("player source key=stem:guitar");
    let v = wait_json(&dirs, PL_PRACTICE, |v| v["source"] == "stem:guitar");
    let pos = v["position_ms"].as_i64().unwrap();
    assert!((1500..4000).contains(&pos), "saved position {pos}");

    // Playing again (Space, with the focus off the controls) plays the guitar (660 Hz).
    ms(800);
    at(&app, &wid, BLANK);
    ms(300);
    let start = mark(&app, &cap);
    app.skip_seen();
    app.key(&wid, "space");
    let l = app.wait_contains("player play position=");
    assert!((position_of(&l) - pos).abs() < 300, "{l} vs {pos}");
    ms(1500);
    let heard = since(&app, &cap, start);
    let (p330, p660) = (tone_power(&heard, rate, 330.0), tone_power(&heard, rate, 660.0));
    assert!(db(p660, p330) > 40.0, "guitar: 330={p330} 660={p660}");

    // Pause: the position is stable.
    app.key(&wid, "space");
    let l = app.wait_contains("player pause position=");
    let paused = position_of(&l);
    assert!(paused > pos + 1000, "{paused} vs {pos}");
    let n = app.count("player transport");
    ms(1200);
    assert_eq!(app.count("player transport"), n, "the position moved while paused");
    assert_eq!(app.count("player seek"), 0, "playing or pausing caused a seek");

    // Clicking the position slider seeks (its middle is about 10 s of the 20 s).
    app.click(&wid, 798, 546);
    let l = app.wait_contains("calliope-ui: player seek position=");
    assert!((8500..11500).contains(&position_of(&l)), "{l}");
    finish(&app);
}

#[test]
#[ignore]
fn tempo_metronome_count_in() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_tempo_metronome");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let cap = dirs.capture();

    // Position 1.0 s, then 120 BPM there: one tempo segment starting at 1000 ms.
    enter(&app, &wid, TIME, "0:01.0");
    app.wait_contains("player seek position=1000");
    enter(&app, &wid, BPM, "120");
    let l = app.wait_contains("calliope-ui: player bpm value=120");
    assert_eq!(position_of(&l), 1000, "{l}");
    let v = wait_json(&dirs, PL_PRACTICE, |v| v["tempo"]["segments"].as_array().is_some_and(|s| s.len() == 1));
    assert_eq!(v["tempo"]["segments"][0]["start_ms"].as_f64(), Some(1000.0), "{v}");
    assert_eq!(v["tempo"]["segments"][0]["bpm"].as_f64(), Some(120.0), "{v}");

    // Metronome on: clicks every 500 ms.
    at(&app, &wid, METRO);
    app.wait_contains("player metronome on=true");
    let start = mark(&app, &cap);
    at(&app, &wid, PLAY);
    app.wait_contains("audio backend=capture");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(3000);
    shot("player-metronome");
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let on = click_onsets_ms(&heard, rate);
    assert!(on.len() >= 5, "click onsets: {on:?}");
    for w in on.windows(2) {
        assert!((w[1] - w[0] - 500.0).abs() <= 5.0, "clicks are not 500 ms apart: {on:?}");
    }
    at(&app, &wid, METRO);
    app.wait_contains("player metronome on=false");

    // Count-in with the guitar: three clicks 500 ms apart, then the tone after 1500 ms.
    choose_source(&app, &wid, 3);
    app.wait_contains("player source key=stem:guitar");
    at(&app, &wid, METRO_COUNT_IN);
    app.wait_contains("player metronome on=false count_in=true");
    enter(&app, &wid, TIME, "0:01.0");
    app.wait_line(|l| l.contains("player seek position=1000"), secs(10));
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(300);
    shot("player-count-in");
    ms(2200);
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let clicks = click_onsets_ms(&heard, rate);
    assert_eq!(clicks.len(), 3, "count-in clicks: {clicks:?}");
    for w in clicks.windows(2) {
        assert!((w[1] - w[0] - 500.0).abs() <= 10.0, "{clicks:?}");
    }
    let tone = tone_start_ms(&heard, rate, 660.0).expect("the guitar starts");
    assert!(
        (tone - clicks[0] - 1500.0).abs() <= 10.0,
        "the tone starts at {tone} ms, the first click at {} ms",
        clicks[0]
    );
    finish(&app);
}

const SECTIONS_FIXTURE: &str = r#"{"schema_version":2,"tempo":{"beats_per_bar":4,"segments":[
    {"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93,"detected_ms":8500}]}}"#;

/// The backend's line for a moved change (the frontend logs its own after the answer).
fn moved(app: &mut App) -> String {
    app.wait_line(|l| l.contains("calliope: player section move") && l.contains("applied="), secs(10))
}

/// The segment starts of a `player sections ...` line.
fn starts_of(line: &str) -> Vec<f64> {
    field(line, "starts").split(',').map(|s| s.parse().unwrap()).collect()
}

fn change_start(v: &Value, i: usize) -> f64 {
    v["tempo"]["segments"][i]["start_ms"].as_f64().unwrap()
}

/// Editing the sections of a detected tempo map in the Player: drag and key a change, retype a
/// BPM, double one while it plays (the clicks follow, nothing stops), add and remove, restart.
#[test]
#[ignore]
fn player_tempo_sections() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_tempo_sections");
    std::fs::write(dirs.player_json(PL_PRACTICE), SECTIONS_FIXTURE).unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let cap = dirs.capture();

    // 1. The detected map is shown.
    assert_eq!(app.count("player sections segments=2 bpm=120,93 starts=500,8500"), 1);
    shot("player-sections");

    // 2. Drag the change bar far right: it stops 4 beats (256 steps) from where it was detected.
    app.drag(&wid, WAVE_BAR.0, 1250, WAVE_BAR.1, || ());
    let l = moved(&mut app);
    assert_eq!(field(&l, "applied"), "256", "{l}");
    let v = wait_json(&dirs, PL_PRACTICE, |v| change_start(v, 1) == 10500.0);
    assert_eq!(v["schema_version"], 5, "{v}");
    assert_eq!(v["tempo"]["segments"][1]["detected_ms"].as_f64(), Some(8500.0), "{v}");

    // 3. The waveform has the focus: Left moves it one step back.
    app.key(&wid, "Left");
    let l = moved(&mut app);
    assert_eq!(field(&l, "applied"), "255", "{l}");

    // 4. Click the first label, type 110; the overlay is shown before Enter.
    at(&app, &wid, WAVE_LABEL0);
    ms(300);
    app.key(&wid, "ctrl+a");
    app.typ(&wid, "110");
    ms(200);
    shot("player-section-bpm");
    app.key(&wid, "Return");
    app.wait_contains("player sections segments=2 bpm=110,93");

    // 5. Section 1 is the target; double it while the metronome plays.
    at(&app, &wid, WAVE_SECTION1);
    enter(&app, &wid, TIME, "0:11.0");
    app.wait_contains("player seek position=11000");
    at(&app, &wid, METRO);
    app.wait_contains("player metronome on=true");
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("audio backend=capture");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(1000);
    at(&app, &wid, SEC_DOUBLE);
    let l = app.wait_contains("player sections segments=2 bpm=110,186");
    let doubled = starts_of(&l);
    ms(1500);
    let between: Vec<String> =
        app.all_lines().into_iter().filter(|l| l.contains("player pause") || l.contains("player play")).collect();
    assert_eq!(between.len(), 1, "playback stopped or restarted: {between:?}");
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let on = click_onsets_ms(&heard, rate);
    let gaps: Vec<f64> = on.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() >= 4, "click onsets: {on:?}");
    assert!((gaps[0] - 645.16).abs() <= 5.0, "before: {gaps:?}");
    assert!((gaps[gaps.len() - 1] - 322.58).abs() <= 5.0, "after: {gaps:?}");

    // 6. Add at 15.0 s with 140 BPM: the new change is on the 186-BPM fine grid (5.04 ms).
    enter(&app, &wid, TIME, "0:15.0");
    app.wait_contains("player seek position=15000");
    at(&app, &wid, SEC_ADD);
    ms(300);
    app.key(&wid, "ctrl+a");
    app.typ(&wid, "140");
    app.key(&wid, "Return");
    let l = app.wait_contains("player sections segments=3 bpm=110,186,140");
    let added = starts_of(&l);
    assert_eq!(&added[..2], &doubled[..], "{l}");
    assert!((added[2] - 15000.0).abs() <= 60000.0 / 186.0 / 64.0 / 2.0 + 0.01, "{l}");

    // 7. Remove the new section (it is selected).
    at(&app, &wid, SEC_REMOVE);
    let l = app.wait_contains("player sections segments=2 bpm=110,186");
    assert_eq!(starts_of(&l), doubled, "{l}");

    // 8. Restart: the same sections, schema 3.
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let v = serde_json::from_slice::<Value>(&std::fs::read(dirs.player_json(PL_PRACTICE)).unwrap()).unwrap();
    assert_eq!(v["schema_version"], 5, "{v}");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let l = app.all_lines().into_iter().rev().find(|l| l.contains("player sections segments=")).unwrap();
    assert!(l.contains("segments=2 bpm=110,186"), "{l}");
    assert_eq!(starts_of(&l), doubled, "{l}");
    finish(&app);
}

/// Per-section metronome settings, names and jumps: a count-in into section 1 replaces the last
/// clicks of section 0, a section without clicks is silent, End and Home jump, the name is typed
/// in the box, and everything is back after a restart.
#[test]
#[ignore]
fn player_section_metronome() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_section_metronome");
    std::fs::write(dirs.player_json(PL_PRACTICE), SECTIONS_FIXTURE).unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let cap = dirs.capture();

    // 1. Every section clicks, no count-in, no names.
    assert_eq!(app.count("player sections segments=2 bpm=120,93 starts=500,8500 metronome=1,1 count_in=0,0 named=0"), 1);
    shot("player-sections-p2");

    // 2. Count-in into section 1.
    at(&app, &wid, WAVE_SECTION1);
    at(&app, &wid, SEC_COUNT_IN);
    app.wait_contains("player section flags index=1 metronome=true count_in=true");

    // 3. Played from 5.0 s: the 120-BPM clicks of the last bar (6500 .. 8000) become three clicks at 93 BPM.
    at(&app, &wid, METRO);
    app.wait_contains("player metronome on=true");
    let play_from = |app: &mut App, wid: &str, play_ms: u64| -> Vec<f64> {
        enter(app, wid, TIME, "0:05.0");
        app.wait_line(|l| l.contains("player seek position=5000"), secs(10));
        let start = mark(app, &cap);
        app.skip_seen();
        at(app, wid, PLAY);
        app.wait_contains("player play");
        let rate = rate_of(app);
        ms(play_ms);
        at(app, wid, PLAY);
        app.wait_contains("player pause");
        click_onsets_ms(&since(app, &cap, start), rate)
    };
    let on = play_from(&mut app, &wid, 4500);
    // The first click is the one at 5000 ms (a beat of section 0); the rest are relative to it.
    let rel: Vec<f64> = on.iter().map(|t| t - on[0] + 5000.0).collect();
    const WANT3: [f64; 7] = [5000.0, 5500.0, 6000.0, 6564.5, 7209.7, 7854.8, 8500.0];
    let want = WANT3;
    assert!(rel.len() >= want.len(), "click onsets: {rel:?}");
    // The onsets are found to a few ms: the gaps within 5 ms, the places within 10 ms of the first.
    for (i, (got, want)) in rel.iter().zip(want).enumerate() {
        assert!((got - want).abs() <= 10.0, "click onsets: {rel:?}, wanted {want:?}");
        if i > 0 {
            assert!((got - rel[i - 1] - (want - WANT3[i - 1])).abs() <= 5.0, "click onsets: {rel:?}");
        }
    }

    // 4. Section 0 without clicks: only the count-in remains before 8500.
    at(&app, &wid, WAVE_SECTION0);
    at(&app, &wid, SEC_CLICK);
    app.wait_contains("player section flags index=0 metronome=false count_in=false");
    let on = play_from(&mut app, &wid, 4500);
    // No click before the count-in, which starts 1564.5 ms after Play (plus the start-up of the stream).
    assert!(on[0] >= 1500.0 && on[0] <= 1700.0, "the first click is not the count-in: {on:?}");
    let want = [0.0, 645.16, 1290.32, 1935.48];
    assert!(on.len() >= want.len(), "click onsets: {on:?}");
    for (got, want) in on.iter().zip(want) {
        assert!((got - on[0] - want).abs() <= 5.0, "click onsets: {on:?}, wanted {want:?} after the first");
    }

    // 5. Name section 1.
    at(&app, &wid, WAVE_SECTION1);
    enter(&app, &wid, SEC_NAME, "Verse");
    app.wait_contains("player section name index=1 length=5");
    shot("player-section-name");

    // 6. Start jumps to the section's first beat, End to the start of the last bar; with the focus off
    // the buttons, Home jumps back.
    at(&app, &wid, SEC_START);
    let l = app.wait_contains("player section jump index=1 edge=start");
    assert_eq!(position_of(&l), 8500, "{l}");
    at(&app, &wid, SEC_END);
    let l = app.wait_contains("player section jump index=1 edge=end");
    let last_bar = position_of(&l);
    // Four beats ending with the last beat that starts before the 20 s end, on the section's beat grid.
    let beat = 60000.0 / 93.0;
    let beats = (last_bar as f64 - 8500.0) / beat;
    assert!((beats - beats.round()).abs() * beat <= 6.0, "{l}");
    assert!(last_bar > 8500 && last_bar as f64 + 3.0 * beat < 20000.0 && last_bar as f64 + 4.0 * beat >= 20000.0, "{l}");
    at(&app, &wid, BLANK);
    app.skip_seen();
    app.key(&wid, "Home");
    let l = app.wait_contains("player section jump index=1 edge=start");
    assert_eq!(position_of(&l), 8500, "{l}");

    // 7. Saved as schema 4.
    ms(600);
    let v = wait_json(&dirs, PL_PRACTICE, |v| v["tempo"]["segments"][1]["name"] == "Verse");
    assert_eq!(v["schema_version"], 5, "{v}");
    assert_eq!(v["tempo"]["segments"][1]["count_in"], true, "{v}");
    assert_eq!(v["tempo"]["segments"][0]["metronome"], false, "{v}");

    // 8. Restart: flags, name and the selection through the playhead (section 1).
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let l = app.all_lines().into_iter().rev().find(|l| l.contains("player sections segments=")).unwrap();
    assert!(l.contains("metronome=0,1 count_in=0,1 named=1"), "{l}");
    at(&app, &wid, WAVE_SECTION1);
    settle();
    shot("player-section-name-restored");
    finish(&app);
}

#[test]
#[ignore]
fn loops() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_loops");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    enter(&app, &wid, BPM, "120");
    app.wait_contains("calliope-ui: player bpm value=120");
    enter(&app, &wid, TIME, "0:02.0");
    app.wait_contains("player seek position=2000");

    // `+` starts a loop without an end; its Play is disabled.
    at(&app, &wid, LOOP_ADD);
    let l = app.wait_contains("player loop add");
    assert!(l.contains("complete=false"), "{l}");
    ms(300);
    shot("player-loop-incomplete");
    at(&app, &wid, LOOP_PLAY);
    ms(700);
    assert_eq!(app.count("player loop play"), 0, "the loop Play must be disabled without an end");

    // Playing on, `<` ends the loop at the position.
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(2000);
    at(&app, &wid, MARK_END);
    let l = app.wait_contains("player loop mark-end");
    assert!(l.contains("complete=true"), "{l}");
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    settle();
    shot("player-loop-complete");

    // Loop On (the default): two passes.
    app.skip_seen();
    at(&app, &wid, LOOP_PLAY);
    for _ in 0..2 {
        let l = app.wait_line(|l| l.contains("calliope: player wrap"), secs(15));
        assert_eq!(field(&l, "loop"), "0", "{l}"); // the index of the only loop (0-based)
        assert_eq!(field(&l, "count_in"), "false", "{l}");
    }
    at(&app, &wid, LOOP_PLAY);
    transport(&mut app, false);
    app.wait_contains("player loop stop");

    // Loop On off: the loop plays once and stops at its end.
    at(&app, &wid, LOOP_ON);
    app.wait_contains("player loop flags");
    app.skip_seen();
    at(&app, &wid, LOOP_PLAY);
    transport(&mut app, true);
    transport(&mut app, false);
    assert_eq!(app.count("calliope: player wrap"), 2, "a loop that does not repeat must not wrap");

    // Loop On again, with the loop count-in.
    at(&app, &wid, LOOP_ON);
    app.wait_contains("player loop flags");
    at(&app, &wid, LOOP_COUNT_IN);
    app.wait_contains("player loop flags");
    app.skip_seen();
    at(&app, &wid, LOOP_PLAY);
    let l = app.wait_line(|l| l.contains("calliope: player wrap"), secs(20));
    assert_eq!(field(&l, "count_in"), "true", "{l}");
    at(&app, &wid, LOOP_PLAY);
    app.wait_contains("player loop stop");

    // `>` after the loop's end drops the end.
    enter(&app, &wid, TIME, "0:06.0");
    app.wait_line(|l| l.contains("player seek position=6000"), secs(10));
    at(&app, &wid, MARK_START);
    let l = app.wait_contains("player loop mark-start");
    assert!(l.contains("complete=false"), "{l}");
    finish(&app);
}

#[test]
#[ignore]
fn restart_restores() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_restart");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    choose_source(&app, &wid, 3);
    app.wait_contains("player source key=stem:guitar");
    enter(&app, &wid, BPM, "120");
    app.wait_contains("calliope-ui: player bpm value=120");
    at(&app, &wid, LOOP_ADD);
    app.wait_contains("player loop add");
    enter(&app, &wid, TIME, "0:02.0");
    app.wait_line(|l| l.contains("player seek position=2000"), secs(10));
    at(&app, &wid, MARK_END);
    app.wait_contains("player loop mark-end");
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(1500);
    at(&app, &wid, PLAY);
    let l = app.wait_contains("player pause position=");
    let paused = position_of(&l);
    assert!(paused > 3000, "{l}");
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let v = serde_json::from_slice::<Value>(&std::fs::read(dirs.player_json(PL_PRACTICE)).unwrap()).unwrap();
    assert_eq!(v["source"], "stem:guitar", "{v}");
    assert!((v["position_ms"].as_i64().unwrap() - paused).abs() <= 2, "{v} vs {paused}");

    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    let l = pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    assert_eq!(field(&l, "source"), "stem:guitar", "{l}");
    assert_eq!(field(&l, "mode"), "bpm", "{l}");
    assert_eq!(field(&l, "loops"), "1", "{l}");
    assert!((position_of(&l) - paused).abs() <= 2, "{l} vs {paused}"); // ms rounding of the saved position
    finish(&app);
}

#[test]
#[ignore]
fn other_sources_and_leaving() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_other_sources");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;

    // A plain MP3 backing track plays.
    pick(&mut app, &wid, &mut collapsed, "mp3 backing", PL_MP3);
    let l = app
        .all_lines()
        .into_iter()
        .find(|l| l.contains("calliope: player open") && l.contains("source=audio"))
        .expect("player open line");
    assert_eq!(field(&l, "rate"), "44100", "{l}");
    let cap = dirs.capture();
    let start = mark(&app, &cap);
    at(&app, &wid, PLAY);
    app.wait_contains("audio backend=capture");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(1500);
    let heard = since(&app, &cap, start);
    let (p330, p660) = (tone_power(&heard, rate, 330.0), tone_power(&heard, rate, 660.0));
    assert!(db(p330, p660) > 40.0, "mp3: 330={p330} 660={p660}");

    // Leaving the Player while it plays pauses it.
    app.skip_seen();
    app.key(&wid, "alt+1");
    app.wait_contains("player leave pause");
    transport(&mut app, false);
    app.key(&wid, "alt+5");
    app.wait_view("player");
    collapsed = false; // the view starts with the tree shown

    // The backing file is missing: the first stem is offered instead.
    let l = pick(&mut app, &wid, &mut collapsed, "missing backing", PL_MISSING_BACKING);
    assert_eq!(field(&l, "sources"), "1", "{l}");
    assert_eq!(field(&l, "source"), "stem:vocals", "{l}");

    // No audio at all: inactive, with the message on screen.
    let l = pick(&mut app, &wid, &mut collapsed, "missing audio", PL_MISSING_AUDIO);
    assert!(l.contains("player inactive") && l.contains(PL_MISSING_AUDIO), "{l}");
    shot("player-missing-audio");
    finish(&app);
}

/// The waveform of "with tempo" (20 s; sections at 0.5 s and 10.5 s) in the floated window: its
/// left and right edge, the y of its middle and the y of its label row. The tree state decides
/// where it is; the values were read from the screenshots of the sweep.
fn wave_box(w: u32, collapsed: bool) -> (i32, i32, i32, i32) {
    let left = if collapsed { 334 } else { 634 };
    // at 1024 px with the tree shown the title wraps onto its own row
    let down = if !collapsed && w < 1280 { 35 } else { 0 };
    (left, w as i32 - 19, 180 + down, 216 + down)
}

/// "Section selected" (a click in section 1: its bar is drawn 4 px wide) and "overlay open" (its
/// label clicked) screenshots. Ends with section 0 selected and the overlay closed.
fn sections_states(app: &App, wid: &str, name: impl Fn(&str) -> String, w: u32, collapsed: bool) {
    let (x0, x1, y, label_y) = wave_box(w, collapsed);
    let bar = x0 + (x1 - x0) * 10500 / 20000;
    app.click(wid, bar + 60, y);
    settle();
    shot(&name("section-selected"));
    app.click(wid, bar + 20, label_y);
    ms(400);
    shot(&name("overlay-open"));
    app.key(wid, "Escape");
    app.click(wid, x0 + 20, y);
    ms(200);
}

/// Screenshots for the visual review (`target/gui-shots/player-*.png`): dark and light, 1280x800
/// and 1024x640, tree shown and collapsed; inactive, active (complete and incomplete loops on the
/// waveform) and counting in. The shots are looked at by hand; the test only checks that the
/// window keeps the size and that nothing is logged as an error.
///
/// The full sweep (about 46 screenshots) only runs with `CALLIOPE_GUI_SHOTS=1`; by default the test
/// runs the same flow (tree collapse, count-in) in dark at 1280x800 and takes one screenshot.
#[test]
#[ignore]
fn visual_review() {
    if !ready() {
        return;
    }
    let sweep = std::env::var_os("CALLIOPE_GUI_SHOTS").is_some_and(|v| !v.is_empty() && v != "0");
    let themes: &[&str] = if sweep { &["dark", "light"] } else { &["dark"] };
    let sizes: &[(u32, u32)] = if sweep { &[(1280, 800), (1024, 640)] } else { &[(1280, 800)] };
    for &theme in themes {
        let dirs = player_dirs(&format!("player_visual_{theme}"));
        // Section 1 of "with tempo" is named and has its count-in on, so selecting it shows the toggles on.
        let file = dirs.player_json(PL_TEMPO);
        let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        v["tempo"]["segments"][1]["name"] = "Verse".into();
        v["tempo"]["segments"][1]["count_in"] = true.into();
        std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        std::fs::create_dir_all(dirs.app_config()).unwrap();
        std::fs::write(dirs.app_config().join("settings.json"), format!(r#"{{"theme":"{theme}"}}"#)).unwrap();
        let (mut app, wid) = start_player(&dirs);
        let name = |size: &str, tree: &str, what: &str| format!("player-{theme}-{size}-{tree}-{what}");

        for &(w, h) in sizes {
            size(&wid, w, h);
            settle();
            if sweep {
                shot(&name(&format!("{w}x{h}"), "shown", "inactive"));
            }
        }
        size(&wid, 1280, 800);
        let mut collapsed = false;
        pick(&mut app, &wid, &mut collapsed, "with tempo", PL_TEMPO);
        // Count In on (the buttons are at known places at 1280x800 with the tree collapsed).
        at(&app, &wid, METRO_COUNT_IN);
        app.wait_contains("player metronome on=false count_in=true");

        for &(w, h) in sizes {
            size(&wid, w, h);
            let sz = format!("{w}x{h}");
            for tree in ["rail", "shown"] {
                if (tree == "shown") == collapsed {
                    app.key(&wid, "ctrl+l");
                    app.wait_line(|l| l.contains("player tree collapsed="), secs(5));
                    collapsed = !collapsed;
                }
                settle();
                if sweep || tree == "shown" {
                    shot(&name(&sz, tree, "active"));
                }
                if sweep {
                    sections_states(&app, &wid, |what| name(&sz, tree, what), w, collapsed);
                    // How far the pane scrolls: compare with the unscrolled shot above.
                    app.wheel(&wid, 900, 450, false, 15);
                    ms(400);
                    shot(&name(&sz, tree, "scrolled"));
                    app.wheel(&wid, 900, 450, true, 15);
                    ms(200);
                }
                // Count-in: the focus must be off the controls for Space; the label is at x=355 (rail) or 655.
                let blank = if collapsed { BLANK } else { (655, 47) };
                at(&app, &wid, blank);
                app.skip_seen();
                app.key(&wid, "space");
                app.wait_line(|l| l.contains("player play"), secs(10));
                ms(400);
                if sweep {
                    shot(&name(&sz, tree, "count-in"));
                }
                app.key(&wid, "space");
                app.wait_line(|l| l.contains("player pause"), secs(10));
            }
        }
        finish(&app);
    }
}
