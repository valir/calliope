//! QA acceptance GUI tests for the Player (hostile files, BPM 0, loops without an end). Run with:
//! `DISPLAY=:1 cargo test --features e2e-hooks --test gui_player_acceptance_e2e -- --ignored --test-threads=1`
//! NullBackend with a capture file only: no real audio device is ever opened.
#![cfg(feature = "e2e-hooks")]

mod common;

use common::*;
use serde_json::Value;
use std::time::Duration;

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg")
}
const BLANK: (i32, i32) = (356, 47);
const PLAY: (i32, i32) = (477, 469);
const BPM: (i32, i32) = (445, 287);
const METRO: (i32, i32) = (754, 329);
const METRO_COUNT_IN: (i32, i32) = (860, 329);
const LOOP_ADD: (i32, i32) = (457, 609);
const MARK_END: (i32, i32) = (1203, 609);
const LOOP_PLAY: (i32, i32) = (402, 676);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}
fn ms(m: u64) {
    std::thread::sleep(Duration::from_millis(m));
}
fn at(app: &App, wid: &str, p: (i32, i32)) {
    app.click(wid, p.0, p.1);
}
fn enter(app: &App, wid: &str, p: (i32, i32), text: &str) {
    at(app, wid, p);
    app.key(wid, "ctrl+a");
    app.typ(wid, text);
    app.key(wid, "Return");
}
fn pick(app: &mut App, wid: &str, query: &str, id: &str) -> String {
    app.key(wid, "ctrl+f ctrl+a");
    app.typ(wid, query);
    settle();
    app.key(wid, "Down Down Down Return");
    app.wait_line(|l| l.trim_end().ends_with(&format!("select id={id}")), secs(10));
    let l = app.wait_line(|l| l.contains("player active") || l.contains("player inactive"), secs(20));
    app.key(wid, "ctrl+l");
    app.wait_contains("player tree collapsed=true");
    settle();
    l
}
fn json(dirs: &Dirs, id: &str) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(dirs.player_json(id)).ok()?).ok()
}

/// A corrupt player.json: the Player opens on defaults; a change replaces it by a valid file.
#[test]
#[ignore]
fn corrupt_player_json_opens_and_is_replaced_on_change() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_player_corrupt");
    std::fs::write(dirs.player_json(PL_PRACTICE), b"{ this is not json").unwrap();
    let (mut app, wid) = start_player(&dirs);
    let l = pick(&mut app, &wid, "practice song", PL_PRACTICE);
    assert!(l.contains("player active"), "{l}");
    assert_eq!(field(&l, "source"), "backing:backing", "{l}");
    shot("qa-player-corrupt-json");
    enter(&app, &wid, BPM, "100");
    app.wait_contains("calliope-ui: player bpm value=100");
    ms(1500);
    let v = json(&dirs, PL_PRACTICE).expect("player.json was replaced by a valid file");
    assert_eq!(v["tempo"]["segments"][0]["bpm"], 100.0);
    app.no_csp_violation();
    app.assert_no_real_audio();
}

/// A player.json of a newer schema is never touched, however the user changes things.
#[test]
#[ignore]
fn newer_schema_player_json_is_left_alone() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_player_newer");
    let text = r#"{"schema_version": 7, "source": "stem:guitar", "future_field": [1,2,3]}"#;
    std::fs::write(dirs.player_json(PL_PRACTICE), text).unwrap();
    let (mut app, wid) = start_player(&dirs);
    pick(&mut app, &wid, "practice song", PL_PRACTICE);
    enter(&app, &wid, BPM, "100");
    app.wait_contains("calliope-ui: player bpm value=100");
    at(&app, &wid, PLAY);
    app.wait_contains("player play");
    ms(1200);
    at(&app, &wid, PLAY);
    app.wait_contains("player pause");
    ms(1500);
    shot("qa-player-newer-schema");
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
    assert_eq!(std::fs::read_to_string(dirs.player_json(PL_PRACTICE)).unwrap(), text);
}

/// BPM 0: bad BPM texts are refused, the metronome buttons do nothing, and Play makes no click.
#[test]
#[ignore]
fn bpm_zero_has_no_clicks_and_bad_bpm_text_is_refused() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_player_bpm0");
    let (mut app, wid) = start_player(&dirs);
    pick(&mut app, &wid, "practice song", PL_PRACTICE);
    let cap = dirs.capture();
    for bad in ["abc", "5", "301", "1e2", "12-0", "12.55"] {
        app.skip_seen();
        enter(&app, &wid, BPM, bad);
        ms(300);
        assert_eq!(app.count("player bpm value"), 0, "{bad} was sent");
    }
    shot("qa-player-bad-bpm");
    app.skip_seen();
    at(&app, &wid, METRO);
    at(&app, &wid, METRO_COUNT_IN);
    ms(300);
    assert_eq!(app.count("player metronome"), 0, "the disabled metronome buttons acted");
    let start = (capture_len(&cap), app.count("audio backend=capture"));
    at(&app, &wid, BLANK);
    app.key(&wid, "space");
    app.wait_contains("player play");
    ms(2500);
    app.key(&wid, "space");
    app.wait_contains("player pause");
    let rate = rate_of(&app);
    let from = if app.count("audio backend=capture") == start.1 { start.0 } else { 0 };
    let heard = capture_from(&cap, from);
    // The backing is a 330 Hz tone: a click would put energy at 1.5 and 2 kHz.
    let (tone, c15, c20) = (tone_power(&heard, rate, 330.0), tone_power(&heard, rate, 1500.0), tone_power(&heard, rate, 2000.0));
    assert!(db(tone, c15) > 40.0 && db(tone, c20) > 40.0, "clicks at BPM 0: 330={tone} 1500={c15} 2000={c20}");
    assert_eq!(app.count("count-in"), 0);
    app.no_csp_violation();
    app.assert_no_real_audio();
}

/// `+` then `<` at the start: refused with the message; Loop Play stays disabled.
#[test]
#[ignore]
fn loop_without_end_refuses_end_at_start_in_the_gui() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_player_loop_noend");
    let (mut app, wid) = start_player(&dirs);
    pick(&mut app, &wid, "practice song", PL_PRACTICE);
    at(&app, &wid, LOOP_ADD);
    app.wait_contains("player loop add");
    app.skip_seen();
    at(&app, &wid, MARK_END);
    let l = app.wait_contains("player loop mark-end");
    assert_eq!(field(&l, "complete"), "false", "{l}");
    ms(300);
    shot("qa-player-loop-end-refused");
    app.skip_seen();
    at(&app, &wid, LOOP_PLAY);
    ms(500);
    assert_eq!(app.count("player loop play"), 0, "Play of a loop without an end acted");
    ms(1500);
    let v = json(&dirs, PL_PRACTICE).unwrap();
    assert!(v["loops"][0]["end"].is_null(), "{v}");
    app.no_csp_violation();
}

/// player.json cannot be written (a directory is in its place; file permissions do not work here
/// because the app runs in a user namespace as root): the Player keeps playing and the failure is
/// logged. What the user sees is judged on the screenshot.
#[test]
#[ignore]
fn unwritable_player_json_keeps_playing() {
    if !ready() {
        return;
    }
    let dirs = player_dirs("qa_player_unwritable");
    std::fs::create_dir(dirs.player_json(PL_PRACTICE)).unwrap();
    let (mut app, wid) = start_player(&dirs);
    pick(&mut app, &wid, "practice song", PL_PRACTICE);
    enter(&app, &wid, BPM, "100");
    app.wait_contains("calliope-ui: player bpm value=100");
    let l = app.wait_line(|l| l.contains("player state error"), secs(5));
    ms(500);
    shot("qa-player-unwritable");
    at(&app, &wid, BLANK);
    app.key(&wid, "space");
    app.wait_contains("player play");
    ms(800);
    app.key(&wid, "space");
    app.wait_contains("player pause");
    assert!(l.contains("message="), "{l}");
    app.close_gracefully(&wid);
    app.wait_exit(secs(5));
}
