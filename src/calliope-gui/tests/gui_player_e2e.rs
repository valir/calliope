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
const BPM: (i32, i32) = (534, 341);
const PLAY: (i32, i32) = (477, 629);
const TIME: (i32, i32) = (669, 628);
// Speed group (the Playback pane's second line) and the Loops group's training button ("Train...").
const SPEED_MINUS: (i32, i32) = (451, 696);
const SPEED_BOX: (i32, i32) = (528, 696);
const SPEED_PLUS: (i32, i32) = (633, 696);
const SPEED_RESET: (i32, i32) = (713, 696);
const TRAIN_OPEN: (i32, i32) = (1194, 485);
const TRAIN_OPEN_1024: (i32, i32) = (689, 175); // 1024x640, tree shown, the Player pane scrolled to its end
                                                // Loops group: the label, `+` (line 1), `>` and `<` (line 2), Play, Loop On and Count-In (line 3).
const LOOP_ADD: (i32, i32) = (1009, 350);
const MARK_START: (i32, i32) = (881, 418);
const MARK_END: (i32, i32) = (1219, 418);
const LOOP_PLAY: (i32, i32) = (889, 485);
const LOOP_ON: (i32, i32) = (981, 485);
const LOOP_COUNT_IN: (i32, i32) = (1090, 485);
// The Sections group (name box next to the BPM in the title row, eight buttons (+ - x2 /2 Prev |< >| Next),
// then Click and Count In, then the Start row: the offset field and Find start), the waveform (section 0 | the change bar at 8500 ms | section 1) and its labels.
const SEC_NAME: (i32, i32) = (723, 341);
const SEC_ADD: (i32, i32) = (373, 394);
const SEC_REMOVE: (i32, i32) = (432, 394);
const SEC_DOUBLE: (i32, i32) = (490, 394);
const SEC_PREV: (i32, i32) = (607, 394);
const SEC_START: (i32, i32) = (666, 394);
const SEC_END: (i32, i32) = (724, 394);
const SEC_NEXT: (i32, i32) = (782, 394);
const SEC_CLICK: (i32, i32) = (384, 458);
const SEC_COUNT_IN: (i32, i32) = (481, 458);
const SEC_OFFSET: (i32, i32) = (432, 511);
const WAVE_BAR: (i32, i32) = (729, 252);
const WAVE_SECTION0: (i32, i32) = (450, 252);
const WAVE_SECTION1: (i32, i32) = (1000, 252);
const WAVE_LABEL0: (i32, i32) = (370, 286);
const REW: (i32, i32) = (400, 629);
const FIND_START: (i32, i32) = (643, 511);
const SEC_DETECT: (i32, i32) = (746, 511);
const SEC_HALVE: (i32, i32) = (549, 394);

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
    app.wait_line(
        |l| l.trim_end().ends_with(&format!("select id={id}")),
        secs(10),
    );
    let l = app.wait_line(
        |l| l.contains("player active") || l.contains("player inactive"),
        secs(20),
    );
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
        assert!(
            Instant::now() < end,
            "player.json of {id} did not reach the expected state"
        );
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
    capture_from(
        cap,
        if app.count("audio backend=capture") == m.1 {
            m.0
        } else {
            0
        },
    )
}

fn position_of(line: &str) -> i64 {
    field(line, "position").parse().unwrap()
}

fn transport(app: &mut App, playing: bool) -> String {
    app.wait_line(
        |l| l.contains(&format!("calliope: player transport playing={playing}")),
        secs(15),
    )
}

#[test]
#[ignore]
fn source_and_tree() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_source_and_tree");
    let (mut app, wid) = start_player(&dirs);
    assert!(
        app.count("player inactive id=none") >= 1,
        "nothing selected: inactive"
    );
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
    let (p330, p660) = (
        tone_power(&heard, rate, 330.0),
        tone_power(&heard, rate, 660.0),
    );
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
    let (p330, p660) = (
        tone_power(&heard, rate, 330.0),
        tone_power(&heard, rate, 660.0),
    );
    assert!(db(p660, p330) > 40.0, "guitar: 330={p330} 660={p660}");

    // Pause: the position is stable.
    app.key(&wid, "space");
    let l = app.wait_contains("player pause position=");
    let paused = position_of(&l);
    assert!(paused > pos + 1000, "{paused} vs {pos}");
    let n = app.count("player transport");
    ms(1200);
    assert_eq!(
        app.count("player transport"),
        n,
        "the position moved while paused"
    );
    assert_eq!(
        app.count("player seek"),
        0,
        "playing or pausing caused a seek"
    );

    // Clicking the position slider seeks (its middle is about 10 s of the 20 s).
    app.click(&wid, 999, 628); // the position slider
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
    let v = wait_json(&dirs, PL_PRACTICE, |v| {
        v["tempo"]["segments"]
            .as_array()
            .is_some_and(|s| s.len() == 1)
    });
    assert_eq!(
        v["tempo"]["segments"][0]["start_ms"].as_f64(),
        Some(1000.0),
        "{v}"
    );
    assert_eq!(
        v["tempo"]["segments"][0]["bpm"].as_f64(),
        Some(120.0),
        "{v}"
    );

    // A new map clicks in every section (Click is on): clicks every 500 ms.
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
        assert!(
            (w[1] - w[0] - 500.0).abs() <= 5.0,
            "clicks are not 500 ms apart: {on:?}"
        );
    }

    // Count-in with the guitar: Click off and Count In on in section 1, so the only clicks are
    // the six ticks (0, 500, 1000, 1250, 1500, 1750 ms), then the tone after 2000 ms.
    choose_source(&app, &wid, 3);
    app.wait_contains("player source key=stem:guitar");
    at(&app, &wid, SEC_CLICK);
    app.wait_contains("player section flags index=0 metronome=false count_in=false");
    at(&app, &wid, SEC_COUNT_IN);
    app.wait_contains("player section flags index=0 metronome=false count_in=true");
    enter(&app, &wid, TIME, "0:01.0");
    app.wait_line(|l| l.contains("player seek position=1000"), secs(10));
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(300);
    shot("player-count-in");
    ms(3200);
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let clicks = click_onsets_ms(&heard, rate);
    assert_eq!(clicks.len(), 6, "count-in clicks: {clicks:?}");
    for (w, gap) in clicks.windows(2).zip([500.0, 500.0, 250.0, 250.0, 250.0]) {
        assert!((w[1] - w[0] - gap).abs() <= 10.0, "{clicks:?}");
    }
    let tone = tone_start_ms(&heard, rate, 660.0).expect("the guitar starts");
    assert!(
        (tone - clicks[0] - 2000.0).abs() <= 10.0,
        "the tone starts at {tone} ms, the first click at {} ms",
        clicks[0]
    );
    finish(&app);
}

const SECTIONS_FIXTURE: &str = r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
    {"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93,"detected_ms":8500}]}}"#;

/// The backend's line for a moved change (the frontend logs its own after the answer).
fn moved(app: &mut App) -> String {
    app.wait_line(
        |l| l.contains("calliope: player section move") && l.contains("applied="),
        secs(10),
    )
}

/// The segment starts of a `player sections ...` line.
fn starts_of(line: &str) -> Vec<f64> {
    field(line, "starts")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect()
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
    assert_eq!(
        app.count("player sections segments=2 bpm=120,93 starts=500,8500"),
        1
    );
    shot("player-sections");

    // 2. Drag the change bar far right: it stops 4 beats (256 steps) from where it was detected.
    app.drag(&wid, WAVE_BAR.0, 1250, WAVE_BAR.1, || ());
    let l = moved(&mut app);
    assert_eq!(field(&l, "applied"), "256", "{l}");
    let v = wait_json(&dirs, PL_PRACTICE, |v| change_start(v, 1) == 10500.0);
    assert_eq!(v["schema_version"], 7, "{v}");
    assert_eq!(
        v["tempo"]["segments"][1]["detected_ms"].as_f64(),
        Some(8500.0),
        "{v}"
    );

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

    // 5. Section 1 is the target; double it while its clicks play.
    at(&app, &wid, WAVE_SECTION1);
    enter(&app, &wid, TIME, "0:11.0");
    app.wait_contains("player seek position=11000");
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
    let between: Vec<String> = app
        .all_lines()
        .into_iter()
        .filter(|l| l.contains("player pause") || l.contains("player play"))
        .collect();
    assert_eq!(
        between.len(),
        1,
        "playback stopped or restarted: {between:?}"
    );
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let on = click_onsets_ms(&heard, rate);
    let gaps: Vec<f64> = on.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.len() >= 4, "click onsets: {on:?}");
    assert!((gaps[0] - 645.16).abs() <= 5.0, "before: {gaps:?}");
    assert!(
        (gaps[gaps.len() - 1] - 322.58).abs() <= 5.0,
        "after: {gaps:?}"
    );

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
    assert!(
        (added[2] - 15000.0).abs() <= 60000.0 / 186.0 / 64.0 / 2.0 + 0.01,
        "{l}"
    );

    // 7. Remove the new section (it is selected).
    at(&app, &wid, SEC_REMOVE);
    let l = app.wait_contains("player sections segments=2 bpm=110,186");
    assert_eq!(starts_of(&l), doubled, "{l}");

    // 8. Restart: the same sections, schema 5.
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let v = serde_json::from_slice::<Value>(&std::fs::read(dirs.player_json(PL_PRACTICE)).unwrap())
        .unwrap();
    assert_eq!(v["schema_version"], 7, "{v}");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let l = app
        .all_lines()
        .into_iter()
        .rev()
        .find(|l| l.contains("player sections segments="))
        .unwrap();
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

    // 1. Every section clicks, no count-in, no names (a schema 5 file without flags).
    assert_eq!(app.count("player sections segments=2 bpm=120,93 starts=500,8500 metronome=1,1 count_in=0,0 named=0"), 1);
    shot("player-sections-p2");

    // 2. Count-in into section 1.
    at(&app, &wid, WAVE_SECTION1);
    at(&app, &wid, SEC_COUNT_IN);
    app.wait_contains("player section flags index=1 metronome=true count_in=true");

    // 3. Played from 5.6 s: the 120-BPM clicks from 5596.8 on (4.5 beats before the section) become the six
    //    count-in ticks at 93 BPM (4 beats, the section's first beat half a beat after the last tick).
    let play_from = |app: &mut App, wid: &str, play_ms: u64| -> Vec<f64> {
        enter(app, wid, TIME, "0:05.6");
        app.wait_line(|l| l.contains("player seek position=5600"), secs(10));
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
    // The first click is the first tick at 5919.4 ms; the rest are relative to it.
    let rel: Vec<f64> = on.iter().map(|t| t - on[0] + 5919.4).collect();
    const WANT3: [f64; 7] = [5919.4, 6564.5, 7209.7, 7532.3, 7854.8, 8177.4, 8500.0];
    let want = WANT3;
    assert!(rel.len() >= want.len(), "click onsets: {rel:?}");
    // The onsets are found to a few ms: the gaps within 5 ms, the places within 10 ms of the first.
    for (i, (got, want)) in rel.iter().zip(want).enumerate() {
        assert!(
            (got - want).abs() <= 10.0,
            "click onsets: {rel:?}, wanted {want:?}"
        );
        if i > 0 {
            assert!(
                (got - rel[i - 1] - (want - WANT3[i - 1])).abs() <= 5.0,
                "click onsets: {rel:?}"
            );
        }
    }

    // 4. Section 0 without clicks: only the count-in remains before 8500.
    at(&app, &wid, WAVE_SECTION0);
    at(&app, &wid, SEC_CLICK);
    app.wait_contains("player section flags index=0 metronome=false count_in=false");
    let on = play_from(&mut app, &wid, 4500);
    // No click before the count-in, which starts 319.4 ms after Play (plus the start-up of the stream).
    assert!(
        on[0] >= 250.0 && on[0] <= 450.0,
        "the first click is not the count-in: {on:?}"
    );
    let want = [0.0, 645.16, 1290.32, 1612.9, 1935.48, 2258.06, 2580.64];
    assert!(on.len() >= want.len(), "click onsets: {on:?}");
    for (got, want) in on.iter().zip(want) {
        assert!(
            (got - on[0] - want).abs() <= 10.0,
            "click onsets: {on:?}, wanted {want:?} after the first"
        );
    }

    // 5. Name section 1.
    at(&app, &wid, WAVE_SECTION1);
    enter(&app, &wid, SEC_NAME, "Verse");
    app.wait_contains("player section name index=1 length=5");
    shot("player-section-name");

    // 6. Start jumps to the section's first beat, End to its last frame (the track end here); with the
    // focus off the buttons, Home jumps back.
    at(&app, &wid, SEC_START);
    let l = app.wait_contains("player section jump index=1 edge=start");
    assert_eq!(position_of(&l), 8500, "{l}");
    at(&app, &wid, SEC_END);
    let l = app.wait_contains("player section jump index=1 edge=end");
    assert_eq!(position_of(&l), 19999, "{l}");
    at(&app, &wid, BLANK);
    app.skip_seen();
    app.key(&wid, "Home");
    let l = app.wait_contains("player section jump index=1 edge=start");
    assert_eq!(position_of(&l), 8500, "{l}");

    // 7. Saved as schema 5.
    ms(600);
    let v = wait_json(&dirs, PL_PRACTICE, |v| {
        v["tempo"]["segments"][1]["name"] == "Verse"
    });
    assert_eq!(v["schema_version"], 7, "{v}");
    assert_eq!(v["tempo"]["segments"][1]["count_in"], true, "{v}");
    assert_eq!(v["tempo"]["segments"][0]["metronome"], false, "{v}");

    // 8. Restart: flags, name and the selection through the playhead (section 1).
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let l = app
        .all_lines()
        .into_iter()
        .rev()
        .find(|l| l.contains("player sections segments="))
        .unwrap();
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

    // `+` makes a complete loop at once: with one section it ends at the track end (20 s; the
    // nearest beat of the 120-BPM grid before it).
    at(&app, &wid, LOOP_ADD);
    let l = app.wait_contains("player loop add");
    assert!(l.contains("complete=true"), "{l}");
    let end: f64 = field(&l, "end").parse().unwrap();
    assert!(end <= 20000.0 && end > 19900.0, "{l}");
    ms(300);
    shot("player-loop-added");

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
    assert_eq!(
        app.count("calliope: player wrap"),
        2,
        "a loop that does not repeat must not wrap"
    );

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

    // `>` after the loop's end drops the end; the loop is incomplete again.
    enter(&app, &wid, TIME, "0:06.0");
    app.wait_line(|l| l.contains("player seek position=6000"), secs(10));
    at(&app, &wid, MARK_START);
    let l = app.wait_contains("player loop mark-start");
    assert!(l.contains("complete=false"), "{l}");
    ms(300);
    shot("player-loop-incomplete");
    let plays = app.count("player loop play");
    at(&app, &wid, LOOP_PLAY);
    ms(700);
    assert_eq!(
        app.count("player loop play"),
        plays,
        "the loop Play must be disabled without an end"
    );
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
    let v = serde_json::from_slice::<Value>(&std::fs::read(dirs.player_json(PL_PRACTICE)).unwrap())
        .unwrap();
    assert_eq!(v["source"], "stem:guitar", "{v}");
    assert!(
        (v["position_ms"].as_i64().unwrap() - paused).abs() <= 2,
        "{v} vs {paused}"
    );

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
    let (p330, p660) = (
        tone_power(&heard, rate, 330.0),
        tone_power(&heard, rate, 660.0),
    );
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
    let l = pick(
        &mut app,
        &wid,
        &mut collapsed,
        "missing backing",
        PL_MISSING_BACKING,
    );
    assert_eq!(field(&l, "sources"), "1", "{l}");
    assert_eq!(field(&l, "source"), "stem:vocals", "{l}");

    // No audio at all: inactive, with the message on screen.
    let l = pick(
        &mut app,
        &wid,
        &mut collapsed,
        "missing audio",
        PL_MISSING_AUDIO,
    );
    assert!(
        l.contains("player inactive") && l.contains(PL_MISSING_AUDIO),
        "{l}"
    );
    shot("player-missing-audio");
    finish(&app);
}

/// The waveform of "with tempo" (20 s; sections at 0.5 s and 10.5 s) in the floated window: its
/// left and right edge, the y of its middle and the y of its label row. The tree state decides
/// where it is; the values were read from the screenshots of the sweep.
fn wave_box(w: u32, h: u32, collapsed: bool) -> (i32, i32, i32, i32) {
    let left = if collapsed { 334 } else { 634 };
    // at 1024 px with the tree shown the title wraps onto its own row
    let down = if !collapsed && w < 1280 { 35 } else { 0 };
    // at 800 px with the tree collapsed the tablature placeholder is as tall as the pane allows
    // and the waveform sits just above the Sections and Loops row
    let tall = if collapsed && h >= 800 { 72 } else { 0 };
    (left, w as i32 - 19, 180 + down + tall, 216 + down + tall)
}

/// The "Train..." button when the Player pane is scrolled to its end (read from the `scrolled-end` screenshots).
fn train_pos(w: u32, h: u32, collapsed: bool) -> Option<(i32, i32)> {
    match (w, h, collapsed) {
        (1280, 800, true) => Some(TRAIN_OPEN),
        (1280, 800, false) => Some((985, 459)),
        (1024, 640, true) => Some((688, 344)),
        (1024, 640, false) => Some(TRAIN_OPEN_1024),
        _ => None,
    }
}

/// The last `player layout overflow=... width=... loops_beside=...` line (logged 300 ms after the pane's size changed).
fn last_layout(app: &App) -> String {
    app.all_lines()
        .into_iter()
        .rev()
        .find(|l| l.contains("player layout overflow="))
        .expect("a player layout line")
}

/// "Section selected" (a click in section 1: its bar is drawn 4 px wide) and "overlay open" (its
/// label clicked) screenshots. Ends with section 0 selected and the overlay closed.
fn sections_states(
    app: &App,
    wid: &str,
    name: impl Fn(&str) -> String,
    w: u32,
    h: u32,
    collapsed: bool,
) {
    let (x0, x1, y, label_y) = wave_box(w, h, collapsed);
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
/// window keeps the size, that nothing is logged as an error, and (1280x800, tree collapsed) the
/// layout target; the measured `player layout` lines go to `target/gui-shots/player-<theme>-layout.txt`.
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
    let sizes: &[(u32, u32)] = if sweep {
        &[(1280, 800), (1024, 640)]
    } else {
        &[(1280, 800)]
    };
    for &theme in themes {
        let dirs = player_dirs(&format!("player_visual_{theme}"));
        // Section 1 of "with tempo" is named and has its count-in on, so selecting it shows the toggles on;
        // section 0 has Click and Count In on. The file is rewritten as schema 5 (no master metronome), so
        // the section flags are the ones written here.
        let file = dirs.player_json(PL_TEMPO);
        let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        v["tempo"]["segments"][1]["name"] = "Verse".into();
        v["tempo"]["segments"][1]["count_in"] = true.into();
        v["schema_version"] = 5.into();
        v.as_object_mut().unwrap().remove("metronome");
        for i in 0..2 {
            v["tempo"]["segments"][i]["metronome"] = true.into();
        }
        v["tempo"]["segments"][0]["count_in"] = true.into();
        std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
        std::fs::create_dir_all(dirs.app_config()).unwrap();
        std::fs::write(
            dirs.app_config().join("settings.json"),
            format!(r#"{{"theme":"{theme}"}}"#),
        )
        .unwrap();
        let (mut app, wid) = start_player(&dirs);
        let name =
            |size: &str, tree: &str, what: &str| format!("player-{theme}-{size}-{tree}-{what}");
        let mut layouts: Vec<String> = Vec::new();

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
                ms(600);
                let l = last_layout(&app);
                println!("LAYOUT {theme} {sz} tree={tree}: {l}");
                layouts.push(format!(
                    "{theme} {sz} tree={tree}: {}",
                    l.split("player layout ").nth(1).unwrap_or(&l)
                ));
                if w == 1280 && tree == "rail" {
                    assert_eq!(field(&l, "loops_beside"), "1", "{l}");
                    assert!(field(&l, "overflow").parse::<i32>().unwrap() <= 14, "{l}");
                }
                if sweep || tree == "shown" {
                    shot(&name(&sz, tree, "active"));
                }
                if sweep {
                    sections_states(&app, &wid, |what| name(&sz, tree, what), w, h, collapsed);
                    // How far the pane scrolls: compare with the unscrolled shot above.
                    app.wheel(&wid, 900, 450, false, 15);
                    ms(400);
                    shot(&name(&sz, tree, "scrolled"));
                    app.wheel(&wid, 900, 450, true, 15);
                    ms(400);
                    // The Speed box refuses 20 (nothing may move), then the training popover, with the
                    // pane scrolled to its end where the button is out of sight otherwise.
                    if w == 1280 && tree == "rail" {
                        enter(&app, &wid, SPEED_BOX, "20");
                        app.wait_line(|l| l.contains("player speed refused text=20"), secs(5));
                        ms(300);
                        shot(&name(&sz, tree, "speed-invalid"));
                        at(&app, &wid, BLANK);
                        ms(200);
                    }
                    app.wheel(&wid, 900, 450, false, 40);
                    ms(400);
                    shot(&name(&sz, tree, "scrolled-end"));
                    if let Some(p) = train_pos(w, h, collapsed) {
                        app.skip_seen();
                        at(&app, &wid, p);
                        let l =
                            app.wait_line(|l| l.contains("player training-popover open"), secs(5));
                        println!("POPOVER {theme} {sz} tree={tree}: {}", popover_rect(&l));
                        ms(300);
                        shot(&name(&sz, tree, "training-popover"));
                        app.key(&wid, "Escape");
                        app.wait_line(|l| l.contains("player training-popover close"), secs(5));
                    }
                    app.wheel(&wid, 900, 450, true, 40);
                    ms(300);
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
        // Kept for the review (the test output is only shown on failure).
        let dir = repo().join("target/gui-shots");
        std::fs::write(
            dir.join(format!("player-{theme}-layout.txt")),
            layouts.join("\n") + "\n",
        )
        .unwrap();
        finish(&app);
    }
}

/// Phase 3: the compact layout (Sections beside Loops, nothing to scroll at 1280x800 with the tree
/// folded), the Play count-in of the section under the playhead (and none elsewhere), the default
/// loop end of `+` and the schema 5 file.
#[test]
#[ignore]
fn player_phase3() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_phase3");
    // Section 1 does not click, section 2 counts in.
    std::fs::write(
        dirs.player_json(PL_PRACTICE),
        r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
            {"start_ms":500,"bpm":120,"metronome":false},
            {"start_ms":8500,"bpm":93,"detected_ms":8500,"count_in":true}]}}"#,
    )
    .unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let cap = dirs.capture();

    // 1. Layout: one row for Sections and Loops, no scrolling.
    ms(800);
    let l = last_layout(&app);
    println!("LAYOUT 1280x800 tree folded: {l}");
    assert_eq!(field(&l, "loops_beside"), "1", "{l}");
    let overflow: i32 = field(&l, "overflow").parse().unwrap();
    // the plan's phase-1 scroll target: about 14 px at 1280x800 with the tree folded
    assert!(
        overflow <= 14,
        "the Player pane scrolls by {overflow} px: {l}"
    );
    shot("player-p3-layout");

    // 2. Play inside section 2 (Count In on): the playhead moves to the next beat (10435.48 ms), six ticks at its
    //    tempo, then the tone exactly 4 beats after the first tick.
    enter(&app, &wid, TIME, "0:10.0");
    app.wait_line(|l| l.contains("player seek position=10000"), secs(10));
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, BLANK);
    app.key(&wid, "space");
    app.wait_contains("audio backend=capture");
    app.wait_contains("player count-in clicks=6 beat_ms=645 section=1");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(300);
    shot("player-p3-count-in");
    ms(3700);
    app.key(&wid, "space");
    app.wait_contains("player pause");
    let heard = since(&app, &cap, start);
    let clicks = click_onsets_ms(&heard, rate);
    assert!(clicks.len() >= 6, "count-in clicks: {clicks:?}");
    let beat = 645.16;
    for (w, gap) in clicks[..6]
        .windows(2)
        .zip([beat, beat, beat / 2.0, beat / 2.0, beat / 2.0])
    {
        assert!((w[1] - w[0] - gap).abs() <= 5.0, "{clicks:?}");
    }
    let tone = tone_start_ms(&heard, rate, 330.0).expect("the backing starts");
    assert!(
        (tone - clicks[0] - 4.0 * 645.16).abs() <= 10.0,
        "the tone starts at {tone} ms, the first click at {} ms",
        clicks[0]
    );

    // 3. Play inside section 1 (no Count In, no Click): no count-in, the tone at once, no click.
    let count_ins = app.count("player count-in");
    enter(&app, &wid, TIME, "0:02.0");
    app.wait_line(|l| l.contains("player seek position=2000"), secs(10));
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, BLANK);
    app.key(&wid, "space");
    app.wait_contains("player play");
    ms(1500);
    app.key(&wid, "space");
    app.wait_contains("player pause");
    assert_eq!(
        app.count("player count-in"),
        count_ins,
        "a count-in in a section without Count In"
    );
    let heard = since(&app, &cap, start);
    let tone = tone_start_ms(&heard, rate, 330.0).expect("the backing starts");
    // The capture also holds the silence between the mark and the key press; a count-in would add about 3200 ms.
    assert!(tone <= 1000.0, "the tone starts after {tone} ms");
    // (the tone's own start shows up as an onset in the click band; nothing else may)
    let clicks: Vec<f64> = click_onsets_ms(&heard, rate)
        .into_iter()
        .filter(|c| (c - tone).abs() > 20.0)
        .collect();
    assert!(
        clicks.is_empty(),
        "section 1 does not click: {clicks:?} tone={tone}"
    );

    // 4. `+` ends the loop at the next section change, or at the track end in the last section.
    enter(&app, &wid, TIME, "0:03.0");
    app.wait_line(|l| l.contains("player seek position=3000"), secs(10));
    app.skip_seen();
    at(&app, &wid, LOOP_ADD);
    let l = app.wait_contains("player loop add");
    assert_eq!(field(&l, "complete"), "true", "{l}");
    let end: f64 = field(&l, "end").parse().unwrap();
    assert!((end - 8500.0).abs() <= 1.0, "{l}");
    enter(&app, &wid, TIME, "0:12.0");
    app.wait_line(|l| l.contains("player seek position=12000"), secs(10));
    app.skip_seen();
    at(&app, &wid, LOOP_ADD);
    let l = app.wait_contains("player loop add");
    assert_eq!(field(&l, "complete"), "true", "{l}");
    let end: f64 = field(&l, "end").parse().unwrap();
    assert!(
        end <= 20001.0 && end > 20000.0 - 2.0 * 60000.0 / 93.0 / 64.0,
        "{l}"
    );

    // 5. Saved as schema 5, without the master metronome; the first loop ends on the tick of 8500 ms.
    ms(600);
    let v = wait_json(&dirs, PL_PRACTICE, |v| {
        v["loops"].as_array().is_some_and(|a| a.len() == 2)
    });
    assert_eq!(v["schema_version"], 7, "{v}");
    assert!(v.get("metronome").is_none(), "{v}");
    let tick = v["loops"][0]["end"]["tick"].as_i64().expect("a tick end");
    assert_eq!(tick, 256, "{v}"); // (8500 - 500) ms at 120 BPM, 16 ticks per beat (31.25 ms)
    finish(&app);
}

/// The `rect=l,t,r,b viewport=WxH` of a `player training-popover open` line: inside the viewport.
fn popover_rect(l: &str) -> String {
    let r: Vec<i32> = field(l, "rect")
        .split(',')
        .map(|n| n.parse().unwrap())
        .collect();
    let (w, h) = field(l, "viewport")
        .split_once('x')
        .map(|(w, h)| (w.parse::<i32>().unwrap(), h.parse::<i32>().unwrap()))
        .unwrap();
    assert!(
        r[0] >= 0 && r[1] >= 0 && r[2] <= w && r[3] <= h,
        "the popover is not inside the window: {l}"
    );
    format!(
        "rect={} viewport={}",
        field(l, "rect"),
        field(l, "viewport")
    )
}

/// Speed and speed training: the Speed group (steps, typing, refusals, keys, Reset), the BPM
/// readout and the heard tempo, the training popover (keyboard, open/close rules, placement) and a run.
#[test]
#[ignore]
fn player_speed() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("player_speed");
    // With Tempo (120 BPM from 0.5 s, 90 BPM from 10.5 s) as schema 6, clicking, the 440 Hz vocals.
    std::fs::write(
        dirs.player_json(PL_TEMPO),
        r#"{"schema_version":6,"source":"stem:vocals","position_ms":0,"speed":1.0,"tempo":{"beats_per_bar":4,"segments":[
            {"start_ms":500,"bpm":120,"metronome":true},{"start_ms":10500,"bpm":90,"metronome":true}]},"loops":[]}"#,
    )
    .unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "with tempo", PL_TEMPO);
    let cap = dirs.capture();

    // 1. Layout.
    ms(800);
    let l = last_layout(&app);
    println!("LAYOUT 1280x800 tree folded: {l}");
    assert_eq!(field(&l, "loops_beside"), "1", "{l}");
    assert!(field(&l, "overflow").parse::<i32>().unwrap() <= 14, "{l}");
    shot("player-speed-layout");

    // 2. Three steps down: 95, 90, 85; play 3 s from 1.0 s: 2.55 s of song, the pitch unchanged.
    for pct in [95, 90, 85] {
        at(&app, &wid, SPEED_MINUS);
        app.wait_contains(&format!("player speed percent={pct} via=minus"));
    }
    enter(&app, &wid, TIME, "0:01.0");
    app.wait_line(|l| l.contains("player seek position=1000"), secs(10));
    let start = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("audio backend=capture");
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(3000);
    at(&app, &wid, PLAY);
    let l = app.wait_contains("player pause position=");
    let advanced = position_of(&l) - 1000;
    assert!(
        (advanced - 2550).abs() <= 150,
        "the position advanced {advanced} ms: {l}"
    );
    let heard = since(&app, &cap, start);
    let at440 = tone_power(&heard, rate, 440.0);
    for off in [-3.0, 3.0] {
        assert!(
            db(at440, tone_power(&heard, rate, 440.0 + off)) > 6.0,
            "the tone is not at 440 Hz (offset {off})"
        );
    }
    assert!(
        db(at440, tone_power(&heard, rate, 440.0 * 0.85)) > 20.0,
        "the pitch followed the speed"
    );

    // 3. 50 %: 60 BPM heard, the clicks 1000 ms apart.
    enter(&app, &wid, SPEED_BOX, "50");
    app.wait_contains("player speed percent=50 via=typed");
    app.wait_contains("player readout bpm=60 BPM speed=50");
    enter(&app, &wid, TIME, "0:01.0");
    app.wait_line(|l| l.contains("player seek position=1000"), secs(10));
    let start = mark(&app, &cap);
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(4500);
    shot("player-speed-50");
    at(&app, &wid, PLAY);
    app.wait_contains("player pause position=");
    let heard = since(&app, &cap, start);
    let on = click_onsets_ms(&heard, rate);
    assert!(on.len() >= 3, "click onsets: {on:?}");
    for w in on.windows(2) {
        assert!(
            (w[1] - w[0] - 1000.0).abs() <= 5.0,
            "clicks are not 1000 ms apart: {on:?}"
        );
    }

    // 4. 20 and 250 are refused; the speed stays 50.
    enter(&app, &wid, SPEED_BOX, "20");
    app.wait_contains("player speed refused text=20");
    ms(300);
    shot("player-speed-refused");
    enter(&app, &wid, SPEED_BOX, "250");
    app.wait_contains("player speed refused text=250");
    ms(1000);
    let v = wait_json(&dirs, PL_TEMPO, |v| v["speed"].as_f64().is_some());
    assert_eq!(v["speed"].as_f64(), Some(0.5), "{v}");

    // 5. `]` (the focus off the box) and Reset.
    at(&app, &wid, BLANK);
    app.key(&wid, "bracketright");
    app.wait_contains("player speed percent=55 via=key");
    at(&app, &wid, SPEED_RESET);
    app.wait_contains("player speed percent=100 via=reset");

    // 6. A loop of bar 2 (2.5 s to 4.5 s) and its training popover.
    enter(&app, &wid, TIME, "0:02.5");
    app.wait_line(|l| l.contains("player seek position=2500"), secs(10));
    at(&app, &wid, LOOP_ADD);
    app.wait_contains("player loop add");
    enter(&app, &wid, TIME, "0:04.5");
    app.wait_line(|l| l.contains("player seek position=4500"), secs(10));
    at(&app, &wid, MARK_END);
    let l = app.wait_contains("player loop mark-end");
    assert!(l.contains("complete=true"), "{l}");
    at(&app, &wid, TRAIN_OPEN);
    let l = app.wait_contains("player training-popover open");
    let rect = popover_rect(&l);
    println!("popover at 1280x800: {rect}");
    ms(300);
    shot("player-training-popover");
    app.key(&wid, "space");
    app.wait_contains("player training index=0 on=true start=70 end=100 step=5 repeats=2");
    for text in ["60", "70", "10", "1"] {
        app.key(&wid, "Tab ctrl+a");
        app.typ(&wid, text);
    }
    app.key(&wid, "Tab"); // commits the last field, the focus goes to Close
    app.wait_contains("player training index=0 on=true start=60 end=70 step=10 repeats=1");
    app.key(&wid, "Escape");
    app.wait_contains("player training-popover close via=escape");
    app.key(&wid, "space");
    app.wait_contains("player training-popover open"); // the focus is back on the button
    ms(300);
    at(&app, &wid, WAVE_SECTION0); // the popover covers the waveform's right half
    app.wait_contains("player training-popover close via=outside");

    // 7. A run: 60 % once, then 70 % until stopped. Loop Play sits under the closed popover: let its
    // close animation finish first, or the click can land on the fading popover.
    ms(400);
    app.skip_seen();
    at(&app, &wid, LOOP_PLAY);
    app.wait_contains("calliope: player training loop=0 pass=1 speed=60 repeat=1/1");
    app.wait_contains("calliope: player training loop=0 pass=2 speed=70 repeat=1/end");
    ms(500);
    shot("player-training-run");
    at(&app, &wid, LOOP_PLAY);
    app.wait_contains("player training end loop=0 speed=100");

    // 8. 80 % and the training survive a restart; the popover shows the saved plan.
    enter(&app, &wid, SPEED_BOX, "80");
    app.wait_contains("player speed percent=80 via=typed");
    let v = wait_json(&dirs, PL_TEMPO, |v| v["speed"].as_f64() == Some(0.8));
    let t = &v["loops"][0]["training"];
    assert_eq!(
        (t["on"].as_bool(), t["start"].as_u64(), t["end"].as_u64()),
        (Some(true), Some(60), Some(70)),
        "{v}"
    );
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    let l = pick(&mut app, &wid, &mut collapsed, "with tempo", PL_TEMPO);
    assert_eq!(field(&l, "speed"), "80", "{l}");
    app.skip_seen();
    at(&app, &wid, TRAIN_OPEN);
    app.wait_contains("player training-popover open");
    ms(300);
    shot("player-training-restored");
    app.key(&wid, "Escape");
    app.wait_contains("player training-popover close via=escape");

    // 1024x640 with the track tree shown.
    size(&wid, 1024, 640);
    app.key(&wid, "ctrl+l");
    app.wait_contains("player tree collapsed=false");
    settle();
    ms(600);
    shot("player-training-1024-closed");
    app.wheel(&wid, 800, 400, false, 40); // the pane scrolls to its end: the Loops pane is at the bottom
    ms(500);
    shot("player-training-1024-scrolled");
    let l = last_layout(&app);
    println!("LAYOUT 1024x640 tree shown: {l}");
    app.skip_seen();
    at(&app, &wid, TRAIN_OPEN_1024);
    let l = app.wait_contains("player training-popover open");
    println!("popover at 1024x640: {}", popover_rect(&l));
    ms(300);
    shot("player-training-popover-1024");
    finish(&app);
}

/// QA phase 3: the four size/tree cases of the layout. Whatever the size, no pane is wider than the Player
/// pane (nothing clipped sideways), the two panes of row 1 do not overlap when they are side by side, and at
/// 1280x800 with the tree folded Sections and Loops are side by side. Screenshots go to
/// `target/gui-shots/qa-p3-*.png` for a look.
#[test]
#[ignore]
fn qa_p3_layout_never_clips_sideways() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_p3_layout");
    // Section 1 is named and every toggle is on, so the section column has its longest content.
    std::fs::write(
        dirs.player_json(PL_PRACTICE),
        r#"{"schema_version":5,"tempo":{"beats_per_bar":4,"segments":[
            {"start_ms":500,"bpm":120,"name":"A rather long section name that is cut","count_in":true},
            {"start_ms":8500,"bpm":93,"detected_ms":8500,"count_in":true}]},
            "loops":[{"id":"a","start":{"ms":9000},"end":{"ms":11000},"repeat":true,"count_in":true}]}"#,
    )
    .unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    let mut report = Vec::new();
    for (w, h) in [(1280u32, 800u32), (1024, 640)] {
        size(&wid, w, h);
        for tree in ["rail", "shown"] {
            if (tree == "shown") == collapsed {
                app.key(&wid, "ctrl+l");
                app.wait_line(|l| l.contains("player tree collapsed="), secs(5));
                collapsed = !collapsed;
            }
            settle();
            ms(700);
            let l = last_layout(&app);
            let num = |k: &str| -> i32 {
                field(&l, k)
                    .parse()
                    .unwrap_or_else(|_| panic!("{k} in {l}"))
            };
            let (width, sec, lp) = (num("width"), num("sections_w"), num("loops_w"));
            report.push(format!("{w}x{h} tree={tree}: {l}"));
            assert!(sec > 0 && lp > 0, "{l}");
            assert!(
                sec <= width && lp <= width,
                "a pane is wider than the Player pane: {l}"
            );
            if field(&l, "loops_beside") == "1" {
                assert!(sec + lp <= width, "side by side but overlapping: {l}");
            }
            if w == 1280 && tree == "rail" {
                assert_eq!(field(&l, "loops_beside"), "1", "{l}");
            }
            shot(&format!("qa-p3-{w}x{h}-{tree}"));
        }
    }
    println!("{}", report.join("\n"));
    std::fs::write(
        repo().join("target/gui-shots/qa-p3-layout.txt"),
        report.join("\n") + "\n",
    )
    .unwrap();
    finish(&app);
}

const NOISY: &str = "0199c0a0-0000-7000-8000-000000000501";

/// The Player fixture plus a plain backing track "Noisy Intro" (1.2 s of faint noise, then a 440 Hz tone)
/// whose `player.json` is a schema-6 file with 120 BPM at 0:00 and a count-in.
fn noisy_dirs(test: &str) -> Dirs {
    let dirs = player_dirs(test);
    let dir = dirs.track_dir(NOISY);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        root().join("tests/fixtures/first-beat/noisy-intro.flac"),
        dir.join("noisy-intro.flac"),
    )
    .unwrap();
    let t = serde_json::json!({
        "schema_version": 2, "type": "backing", "id": NOISY, "band": "Zed Noise", "album": "First Beat",
        "title": "Noisy Intro", "composers": [], "year": null, "source_url": null, "copyright": null,
        "audio": "noisy-intro.flac", "original": null, "stems": [], "stem_model": null,
        "tablatures": [], "backings": [], "imported": "2026-10-07T10:00:00Z", "modified": "2026-10-07T10:00:00Z"
    });
    std::fs::write(dir.join("track.json"), t.to_string()).unwrap();
    std::fs::write(
        dirs.player_json(NOISY),
        r#"{"schema_version":6,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":0.0,"bpm":120.0,"count_in":true}]}}"#,
    )
    .unwrap();
    dirs
}

/// A track whose first beat is at 0:00 gets it found once, at open; the lead-in is dimmed and never
/// played; the first bar moves with keys and the mouse; Find start finds it again.
#[test]
#[ignore]
fn player_first_beat() {
    if !ready() {
        return;
    }
    let dirs = noisy_dirs("player_first_beat");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "noisy intro", NOISY);
    let cap = dirs.capture();

    // 1. Found at open.
    let l = app
        .all_lines()
        .into_iter()
        .find(|l| l.contains("calliope: player first-beat id="))
        .expect("the automatic first-beat line");
    assert_eq!(field(&l, "via"), "auto", "{l}");
    let start: f64 = field(&l, "start_ms").parse().unwrap();
    assert!((start - 1200.0).abs() <= 10.0, "{l}");
    let v = wait_json(&dirs, NOISY, |v| {
        v["tempo"]["segments"][0]["found_ms"].is_number()
    });
    let found = v["tempo"]["segments"][0]["found_ms"].as_f64().unwrap();
    assert!((found - 1200.0).abs() <= 10.0, "{v}");
    ms(1200);
    shot("player-first-beat");

    // 2. Home and Rew never go below the first beat.
    app.key(&wid, "Home");
    ms(300);
    app.skip_seen();
    at(&app, &wid, REW);
    app.wait_contains("player skip delta=-5000");
    ms(300);
    // no position the player reports (jump, transport, seek) is before the first beat
    let below: Vec<String> = app
        .all_lines()
        .into_iter()
        .filter(|l| l.contains("calliope: player") && !l.contains("player open"))
        .filter(|l| {
            ["position_ms", "position"].iter().any(|k| {
                l.contains(&format!(" {k}="))
                    && field(l, k).parse::<f64>().is_ok_and(|p| p < 1195.0)
            })
        })
        .collect();
    assert!(below.is_empty(), "below the first beat: {below:?}");

    // 3. Play from the start: count-in, then the tone; none of the noise is played.
    let m = mark(&app, &cap);
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(3800);
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    let heard = since(&app, &cap, m);
    let clicks = click_onsets_ms(&heard, rate);
    let tone = tone_start_ms(&heard, rate, 440.0).expect("the tone starts");
    let tone_clicks: Vec<f64> = clicks
        .iter()
        .cloned()
        .filter(|c| *c < tone - 50.0)
        .collect();
    assert_eq!(tone_clicks.len(), 6, "count-in clicks: {clicks:?}");
    for (w, gap) in tone_clicks
        .windows(2)
        .zip([500.0, 500.0, 250.0, 250.0, 250.0])
    {
        assert!((w[1] - w[0] - gap).abs() <= 3.0, "{clicks:?}");
    }
    assert!(
        (tone - clicks[0] - 2000.0).abs() <= 10.0,
        "tone at {tone}, clicks {clicks:?}"
    );
    let left: Vec<f32> = heard.chunks_exact(2).map(|c| c[0]).collect();
    for (i, x) in left.iter().enumerate() {
        let t = i as f64 * 1000.0 / rate;
        if t < tone - 20.0 && !clicks.iter().any(|c| t >= c - 5.0 && t < c + 80.0) {
            assert!(x.abs() < 0.0015, "noise {x} at {t} ms");
        }
    }

    // 4. Keys on the first bar: one step is 1/64 beat = 7.8125 ms; the found start stays.
    at(&app, &wid, WAVE_SECTION1);
    app.key(&wid, "Up");
    app.skip_seen();
    app.key(&wid, "Right");
    let l = moved(&mut app);
    assert_eq!(field(&l, "index"), "0", "{l}");
    assert_eq!(field(&l, "steps"), "1", "{l}");
    assert_eq!(field(&l, "applied"), "1", "{l}");
    let v = wait_json(&dirs, NOISY, |v| change_start(v, 0) > found + 7.0);
    assert!((change_start(&v, 0) - (found + 7.8125)).abs() < 0.01, "{v}");
    assert_eq!(
        v["tempo"]["segments"][0]["found_ms"].as_f64(),
        Some(found),
        "{v}"
    );

    // 5. Drag the first bar a little; the readout speaks of the found start while the button is held.
    let (x0, x1, y, _) = wave_box(1280, 800, true);
    let bar = x0 + ((x1 - x0) as f64 * (found + 7.8125) / 5200.0) as i32;
    app.drag_xy(&wid, (bar, y), (bar + 12, y), || {
        ms(200);
        shot("player-first-beat-drag");
    });
    moved(&mut app);

    // 6. Find start puts it back.
    app.skip_seen();
    at(&app, &wid, FIND_START);
    let l = app.wait_contains("calliope: player first-beat id=");
    assert_eq!(field(&l, "via"), "button", "{l}");
    let start: f64 = field(&l, "start_ms").parse().unwrap();
    assert!((start - 1200.0).abs() <= 10.0, "{l}");

    // 7. The button fits at 1280x800.
    let l = last_layout(&app);
    assert_eq!(field(&l, "overflow"), "0", "{l}");
    assert_eq!(field(&l, "loops_beside"), "1", "{l}");
    finish(&app);

    // 8. The light theme (the found start is saved, so the track opens with its bar in place).
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    std::fs::create_dir_all(dirs.app_config()).unwrap();
    std::fs::write(
        dirs.app_config().join("settings.json"),
        r#"{"theme":"light"}"#,
    )
    .unwrap();
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "noisy intro", NOISY);
    ms(1200);
    shot("player-first-beat-light");
    finish(&app);
}

/// `noisy_dirs` with a three-section `player.json` (schema 7): a found first start at 1200 ms, a detected
/// section at 3000 ms and a plain one at 4200 ms. Nobody clicks, so the tone is all that is heard.
fn tuning_dirs(test: &str) -> Dirs {
    let dirs = noisy_dirs(test);
    std::fs::write(
        dirs.player_json(NOISY),
        r#"{"schema_version":7,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":1200.0,"bpm":120.0,"found_ms":1200.0,"count_in":false,"metronome":false},{"start_ms":3000.0,"bpm":120.0,"detected_ms":3000.0,"count_in":true,"metronome":false},{"start_ms":4200.0,"bpm":120.0,"metronome":false}]}}"#,
    )
    .unwrap();
    dirs
}

/// The layout assertion of the 1280x800 window with the tree folded: no overflow, Loops beside Sections.
fn assert_tuning_layout(app: &App) {
    let l = last_layout(app);
    assert_eq!(field(&l, "overflow"), "0", "{l}");
    assert_eq!(field(&l, "loops_beside"), "1", "{l}");
    let w: i32 = field(&l, "sections_w").parse().unwrap();
    assert!(w <= 496, "{l}");
}

/// The text of the frontend jump line for section `index` (waits for it).
fn jumped(app: &mut App, index: usize, via: &str) -> String {
    let l = app.wait_line(
        |l| {
            l.contains(&format!("player section jump index={index} edge=start"))
                && l.contains(&format!("via={via}"))
        },
        secs(10),
    );
    assert!(l.contains("calliope-ui:"), "{l}");
    l
}

/// Plays for a moment, jumps with `jump` while playing and pauses again: playback goes on through the
/// jump (no pause line between), nothing but the tone is heard (no count-in clicks, no gap).
fn jump_while_playing(
    app: &mut App,
    wid: &str,
    cap: &std::path::Path,
    jump: (i32, i32),
    what: &str,
) {
    let m = mark(app, cap);
    app.skip_seen();
    at(app, wid, PLAY);
    app.wait_contains("player play");
    let rate = rate_of(app);
    ms(300);
    at(app, wid, jump);
    app.wait_contains("calliope: player section jump");
    ms(1500);
    at(app, wid, PLAY);
    app.wait_contains("player pause");
    let lines = app.all_lines();
    let from = lines
        .iter()
        .rposition(|l| l.contains("calliope: player section jump"))
        .unwrap();
    let stops = lines[from..]
        .iter()
        .filter(|l| l.contains("calliope: player transport playing=false"))
        .count();
    assert_eq!(stops, 1, "{what}: playback stopped at the jump: {lines:?}");
    let heard = since(app, cap, m);
    let tone = tone_power(&heard, rate, 440.0);
    for hz in [1500.0, 2000.0] {
        let d = db(tone_power(&heard, rate, hz), tone);
        assert!(d < -30.0, "{what}: clicks were heard ({hz} Hz at {d} dB)");
    }
    // 100 ms pieces from the tone's start to the pause all hold the tone
    // (what was captured before the first press of Play is not counted)
    let t0 = tone_start_ms(&heard, rate, 440.0).expect("the tone starts");
    let first = (t0 * rate / 1000.0) as usize;
    let left: Vec<f32> = heard.chunks_exact(2).skip(first).map(|c| c[0]).collect();
    let piece = (rate * 0.1) as usize;
    let rms: Vec<f64> = left
        .chunks_exact(piece)
        .map(|c| (c.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / c.len() as f64).sqrt())
        .collect();
    assert!(rms.len() >= 10, "{what}: too little audio");
    let top = rms.iter().cloned().fold(0.0, f64::max);
    for (i, r) in rms[1..rms.len() - 1].iter().enumerate() {
        assert!(
            *r > top * 0.5,
            "{what}: a gap in the tone at piece {i}: {rms:?}"
        );
    }
}

/// The first bar's start is typed in the Sections pane (steps of 1/64 beat from the found start), clamped,
/// refused when it is not a number; Next / Prev go to the section starts and keep playing; the keys show the
/// readout for a moment.
#[test]
#[ignore]
fn player_first_beat_tuning() {
    if !ready() {
        return;
    }
    let dirs = tuning_dirs("player_first_beat_tuning");
    let (mut app, wid) = start_player(&dirs);
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "noisy intro", NOISY);
    let cap = dirs.capture();

    // 1. The layout.
    ms(600);
    assert_tuning_layout(&app);
    shot("player-tuning");

    // 2. Keyboard readout: it shows after a key and is gone 1.5 s later.
    let (x0, x1, y, _) = wave_box(1280, 800, true);
    app.click(&wid, x0 + (x1 - x0) * 2000 / 5200, y);
    app.skip_seen();
    app.key(&wid, "Right");
    let l = moved(&mut app);
    assert_eq!(field(&l, "index"), "0", "{l}");
    assert_eq!(field(&l, "steps"), "1", "{l}");
    assert_eq!(field(&l, "applied"), "1", "{l}");
    shot("player-tuning-key-readout");
    ms(2000);
    shot("player-tuning-key-readout-gone");

    // 3. A typed offset.
    app.skip_seen();
    enter(&app, &wid, SEC_OFFSET, "+10");
    app.wait_contains("calliope-ui: player section offset index=0 text=+10 steps=10");
    let l = moved(&mut app);
    assert_eq!(field(&l, "steps"), "10", "{l}");
    assert_eq!(field(&l, "applied"), "10", "{l}");
    let v = wait_json(&dirs, NOISY, |v| {
        (change_start(v, 0) - 1278.125).abs() < 0.01
    });
    assert_eq!(
        v["tempo"]["segments"][0]["found_ms"].as_f64(),
        Some(1200.0),
        "{v}"
    );

    // 4. Beyond the limit: the first bar stops at 0:00 and the field says how far it went.
    app.skip_seen();
    enter(&app, &wid, SEC_OFFSET, "-9999");
    moved(&mut app);
    wait_json(&dirs, NOISY, |v| change_start(v, 0).abs() < 0.01);
    ms(300);
    shot("player-tuning-clamped");
    enter(&app, &wid, SEC_OFFSET, "+10");
    wait_json(&dirs, NOISY, |v| {
        (change_start(v, 0) - 1278.125).abs() < 0.01
    });

    // 5. Text that is no number is refused and nothing moves.
    let before = std::fs::read(dirs.player_json(NOISY)).unwrap();
    app.skip_seen();
    enter(&app, &wid, SEC_OFFSET, "1.5");
    app.wait_contains("calliope-ui: player section offset refused text=1.5");
    ms(500);
    assert_eq!(
        std::fs::read(dirs.player_json(NOISY)).unwrap(),
        before,
        "player.json changed"
    );
    shot("player-tuning-invalid");
    app.key(&wid, "Escape");
    ms(200);

    // 6. Next: the start of section 2. Find start is disabled there and the field shows its offset.
    at(&app, &wid, BLANK);
    app.skip_seen();
    at(&app, &wid, SEC_NEXT);
    let l = jumped(&mut app, 1, "next");
    assert!((position_of(&l) - 3000).abs() <= 1, "{l}");
    ms(300);
    shot("player-tuning-section2");

    // 7. An offset in section 2 moves only its bar.
    app.skip_seen();
    enter(&app, &wid, SEC_OFFSET, "-8");
    let l = moved(&mut app);
    assert_eq!(field(&l, "index"), "1", "{l}");
    assert_eq!(field(&l, "steps"), "-8", "{l}");
    assert_eq!(field(&l, "applied"), "-8", "{l}");
    let v = wait_json(&dirs, NOISY, |v| (change_start(v, 1) - 2937.5).abs() < 0.01);
    assert!((change_start(&v, 0) - 1278.125).abs() < 0.01, "{v}");
    assert!((change_start(&v, 2) - 4200.0).abs() < 0.01, "{v}");

    // 8. Prev from the middle of section 2 goes to the first beat.
    enter(&app, &wid, TIME, "0:03.5");
    ms(300);
    app.skip_seen();
    at(&app, &wid, SEC_PREV);
    let l = jumped(&mut app, 0, "prev");
    assert!((position_of(&l) - 1278).abs() <= 1, "{l}");

    // 9. Next, Next: the last section, where Next is disabled.
    app.skip_seen();
    at(&app, &wid, SEC_NEXT);
    let l = jumped(&mut app, 1, "next");
    assert!((position_of(&l) - 2937).abs() <= 1, "{l}");
    at(&app, &wid, SEC_NEXT);
    let l = jumped(&mut app, 2, "next");
    assert!((position_of(&l) - 4200).abs() <= 1, "{l}");
    ms(300);
    shot("player-tuning-last");

    // 10. While playing, Next and |< keep the playback going (no count-in) and nothing but the tone is heard.
    app.skip_seen();
    at(&app, &wid, SEC_PREV);
    jumped(&mut app, 1, "prev");
    at(&app, &wid, SEC_PREV);
    jumped(&mut app, 0, "prev");
    ms(300);
    jump_while_playing(&mut app, &wid, &cap, SEC_NEXT, "Next");
    enter(&app, &wid, TIME, "0:03.5");
    ms(300);
    jump_while_playing(&mut app, &wid, &cap, SEC_START, "|<");

    // 11. The layout again.
    assert_tuning_layout(&app);
    finish(&app);
}

// ------------------------------------------------------------------ Detect BPM (gui-section-bpm-detect)

/// Track 304 ("With Tempo": backing 330 Hz, vocals 440 Hz, guitar 660 Hz, 22 050 Hz): a Click-on 100 BPM
/// section at 500 ms and a 120 BPM one at 17 800 ms, the backing selected.
fn detect_dirs(test: &str) -> Dirs {
    let dirs = player_dirs(test);
    std::fs::write(
        dirs.player_json(PL_TEMPO),
        r#"{"schema_version":7,"source":"backing:backing","tempo":{"beats_per_bar":4,"segments":[{"start_ms":500,"bpm":100,"found_ms":500,"metronome":true},{"start_ms":17800,"bpm":120,"detected_ms":17800,"metronome":false}]}}"#,
    )
    .unwrap();
    dirs
}

/// Enters `time` in the position field and waits for the seek.
fn go_to(app: &mut App, wid: &str, time: &str) {
    app.skip_seen();
    enter(app, wid, TIME, time);
    app.wait_contains("player seek position=");
}

/// Clicks the Detect BPM button and returns the backend's start line (or the refusal).
fn press_detect(app: &mut App, wid: &str) -> String {
    app.skip_seen();
    at(app, wid, SEC_DETECT);
    app.wait_line(
        |l| {
            l.contains("calliope: player detect refused")
                || (l.contains("calliope: player detect id=") && l.contains("from_ms="))
        },
        secs(10),
    )
}

/// The wait for the `player detected` line of the run, then the frontend's line for it.
fn detected(app: &mut App) -> String {
    let l = app.wait_line(|l| l.contains("calliope: player detected id="), secs(15));
    app.wait_line(
        |l| l.contains("calliope-ui: player detected index="),
        secs(10),
    );
    l
}

fn wait_phase_uploading(app: &mut App) {
    app.wait_line(
        |l| {
            l.contains("calliope: player detect id=")
                && ["phase=uploading", "phase=waiting", "phase=detecting"]
                    .iter()
                    .any(|p| l.contains(p))
        },
        secs(10),
    );
}

fn posts(dirs: &Dirs) -> usize {
    dirs.stems_log()
        .lines()
        .filter(|l| l.contains("method=POST") && l.contains("path=/v1/beats"))
        .count()
}

fn bpm0(v: &Value) -> Option<f64> {
    v["tempo"]["segments"][0]["bpm"].as_f64()
}

/// What the stub tracker was sent last: (seconds, 330 Hz power, 660 Hz power).
fn last_piece(dirs: &Dirs) -> (f64, f64, f64) {
    let pcm = decode(&dirs.base().join("stems-work/stub-tracker-last.flac"));
    let rate = 22050.0;
    (
        pcm.len() as f64 / 2.0 / rate,
        tone_power(&pcm, rate, 330.0),
        tone_power(&pcm, rate, 660.0),
    )
}

/// Detect BPM on the Start row: the piece, the source, the result and its texts, Cancel, the
/// result dropped by a removed section or another track, and a detection while playing with the Click on.
#[test]
#[ignore]
fn player_section_bpm_detect() {
    if !ready() {
        return;
    }
    let dirs = detect_dirs("player_section_bpm_detect");
    let (mut app, wid) = start_player_stems(&dirs, "bpm:93.4");
    let mut collapsed = false;
    pick(&mut app, &wid, &mut collapsed, "with tempo", PL_TEMPO);
    let cap = dirs.capture();

    // 1. The new button fits at 1280x800.
    ms(1000);
    assert_tuning_layout(&app);
    shot("player-detect");

    // 2. 7.2 s from the playhead, the backing; 93.4 BPM applied to section 0 only.
    go_to(&mut app, &wid, "0:10.5");
    let l = press_detect(&mut app, &wid);
    assert!(
        l.contains("index=0 source=backing:backing from_ms=10500 to_ms=17700"),
        "{l}"
    );
    let l = detected(&mut app);
    assert!(l.contains("new=93.4 hint=none applied=true"), "{l}");
    let v = wait_json(&dirs, PL_TEMPO, |v| bpm0(v) == Some(93.4));
    assert_eq!(
        v["tempo"]["segments"][0]["start_ms"].as_f64(),
        Some(500.0),
        "{v}"
    );
    assert_eq!(
        v["tempo"]["segments"][0]["found_ms"].as_f64(),
        Some(500.0),
        "{v}"
    );
    assert_eq!(
        v["tempo"]["segments"][1]["start_ms"].as_f64(),
        Some(17800.0),
        "{v}"
    );
    assert_eq!(
        v["tempo"]["segments"][1]["bpm"].as_f64(),
        Some(120.0),
        "{v}"
    );
    let (secs_sent, p330, p660) = last_piece(&dirs);
    assert!(
        (secs_sent - 7.2).abs() < 2.0 / 22050.0 + 0.001,
        "{secs_sent} s sent"
    );
    assert!(db(p330, p660) >= 20.0, "330={p330} 660={p660}");
    let parts: Vec<_> = std::fs::read_dir(dirs.track_dir(PL_TEMPO))
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".calliope-beats-")
        })
        .collect();
    assert!(parts.is_empty(), "left over: {parts:?}");
    ms(300);
    shot("player-detect-done");

    // 3. Stops at the section's end.
    go_to(&mut app, &wid, "0:14.8");
    let l = press_detect(&mut app, &wid);
    assert!(l.contains("from_ms=14800 to_ms=17800"), "{l}");
    detected(&mut app);
    let (secs_sent, ..) = last_piece(&dirs);
    assert!((secs_sent - 3.0).abs() < 0.001, "{secs_sent} s sent");

    // 4. Less than a bar before the end: refused, the server not called, nothing saved.
    go_to(&mut app, &wid, "0:17.0");
    ms(700);
    let (before, saved) = (
        posts(&dirs),
        std::fs::read(dirs.player_json(PL_TEMPO)).unwrap(),
    );
    let l = press_detect(&mut app, &wid);
    assert!(
        l.contains("refused") && l.contains("reason=too-short"),
        "{l}"
    );
    app.wait_contains("calliope-ui: player detect error");
    ms(500);
    assert_eq!(posts(&dirs), before, "the server was called");
    assert_eq!(std::fs::read(dirs.player_json(PL_TEMPO)).unwrap(), saved);
    shot("player-detect-short");

    // 5. The selected source (Guitar), a result about twice the old one.
    set_tracker_mode(&dirs, "bpm:186.8");
    choose_source(&app, &wid, 2);
    app.wait_contains("player source key=stem:guitar");
    go_to(&mut app, &wid, "0:10.5");
    let l = press_detect(&mut app, &wid);
    assert!(l.contains("source=stem:guitar"), "{l}");
    let l = detected(&mut app);
    assert!(l.contains("new=186.8 hint=double applied=true"), "{l}");
    wait_json(&dirs, PL_TEMPO, |v| bpm0(v) == Some(186.8));
    let (_, p330, p660) = last_piece(&dirs);
    assert!(db(p660, p330) >= 20.0, "330={p330} 660={p660}");
    ms(300);
    shot("player-detect-double");
    at(&app, &wid, SEC_HALVE);
    wait_json(&dirs, PL_TEMPO, |v| bpm0(v) == Some(93.4));

    // 6. No steady beat: a message, the BPM stays.
    set_tracker_mode(&dirs, "irregular");
    press_detect(&mut app, &wid);
    let l = app.wait_line(|l| l.contains("calliope: player detect failed"), secs(15));
    assert!(l.contains("message=No steady beat"), "{l}");
    ms(1000);
    assert_eq!(bpm0(&wait_json(&dirs, PL_TEMPO, |_| true)), Some(93.4));
    shot("player-detect-failed");

    // 7. Cancel while it runs.
    set_tracker_mode(&dirs, "slow-bpm:150");
    press_detect(&mut app, &wid);
    wait_phase_uploading(&mut app);
    shot("player-detect-running");
    app.skip_seen();
    at(&app, &wid, SEC_DETECT);
    let l = app.wait_line(
        |l| l.contains("calliope: player detect cancelled"),
        secs(10),
    );
    assert!(l.contains("reason=button"), "{l}");
    ms(4000);
    assert_eq!(
        app.count("applied=true"),
        3,
        "only the three earlier results"
    );
    assert_eq!(bpm0(&wait_json(&dirs, PL_TEMPO, |_| true)), Some(93.4));
    shot("player-detect-cancelled");

    // 8. The section is removed during the run: the result is dropped.
    go_to(&mut app, &wid, "0:17.9");
    press_detect(&mut app, &wid);
    wait_phase_uploading(&mut app);
    app.skip_seen();
    at(&app, &wid, SEC_REMOVE);
    app.wait_contains("calliope: player section remove id=");
    let l = app.wait_line(|l| l.contains("calliope: player detected id="), secs(15));
    assert!(l.contains("applied=false reason=changed"), "{l}");
    let v = wait_json(&dirs, PL_TEMPO, |v| {
        v["tempo"]["segments"]
            .as_array()
            .is_some_and(|s| s.len() == 1)
    });
    assert_eq!(bpm0(&v), Some(93.4), "{v}");

    // 9. Playing with the Click on: the clicks follow the new BPM, the tone and the transport go on.
    set_tracker_mode(&dirs, "bpm:120");
    go_to(&mut app, &wid, "0:02.0");
    let m = mark(&app, &cap);
    let stops = app.count("calliope: player transport playing=false");
    app.skip_seen();
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    let rate = rate_of(&app);
    ms(1500);
    press_detect(&mut app, &wid);
    let l = detected(&mut app);
    assert!(l.contains("new=120 ") && l.contains("applied=true"), "{l}");
    ms(2000);
    let end_stops = app.count("calliope: player transport playing=false");
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    assert_eq!(end_stops, stops, "playback stopped");
    let heard = since(&app, &cap, m);
    let on = click_onsets_ms(&heard, rate);
    let gaps: Vec<f64> = on.windows(2).map(|w| w[1] - w[0]).collect();
    let near = |g: f64, want: f64| (g - want).abs() <= 3.0;
    let old = gaps.iter().take_while(|g| near(**g, 642.0)).count();
    let new = gaps.iter().rev().take_while(|g| near(**g, 500.0)).count();
    assert!(old >= 1 && new >= 2, "click gaps {gaps:?} (onsets {on:?})");
    assert!(gaps.len() - old - new <= 1, "click gaps {gaps:?}");
    // the tone goes on through the edit
    let w = (rate / 100.0) as usize;
    let left: Vec<f32> = heard.chunks_exact(2).map(|c| c[0]).collect();
    // (the first 0.2 s and the pause at the end are left out)
    let rms: Vec<f32> = left[(rate / 5.0) as usize..left.len() - (rate / 5.0) as usize]
        .chunks_exact(w)
        .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt())
        .collect();
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let low: Vec<usize> = (0..rms.len()).filter(|&i| rms[i] < 0.5 * median).collect();
    assert!(
        low.is_empty(),
        "a gap in the tone: windows {low:?} of {} (median {median})",
        rms.len()
    );
    wait_json(&dirs, PL_TEMPO, |v| bpm0(v) == Some(120.0));

    // 10. Another track opened during a run: the result is dropped.
    set_tracker_mode(&dirs, "slow-bpm:150");
    go_to(&mut app, &wid, "0:10.5");
    press_detect(&mut app, &wid);
    wait_phase_uploading(&mut app);
    pick(&mut app, &wid, &mut collapsed, "practice song", PL_PRACTICE);
    assert_eq!(
        app.count("calliope: player detect cancelled"),
        2,
        "the run is cancelled by the other track"
    );
    assert_eq!(app.count("reason=closed"), 1);
    ms(4000);
    assert_eq!(app.count("applied=true"), 4, "the run was dropped");
    pick(&mut app, &wid, &mut collapsed, "with tempo", PL_TEMPO);
    let v = wait_json(&dirs, PL_TEMPO, |_| true);
    assert_eq!(bpm0(&v), Some(120.0), "{v}");

    // 11. Still fits.
    ms(1000);
    assert_tuning_layout(&app);
    finish(&app);
}
