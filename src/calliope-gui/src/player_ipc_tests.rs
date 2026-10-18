//! Player IPC layer tests: copies of `tests/fixtures/library-player`, `ManualBackend`s (never a
//! real device) and the real command bodies.

use super::*;
use crate::audio_out::ManualBackend;
use crate::picker::ScriptedPicker;
use crate::player_state;
use std::path::Path;

const PRACTICE: &str = "0199c0a0-0000-7000-8000-000000000301";
const STEMS_ONLY: &str = "0199c0a0-0000-7000-8000-000000000302";
const MISSING_AUDIO: &str = "0199c0a0-0000-7000-8000-000000000306";

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

struct Env {
    dir: tempfile::TempDir,
    repo: RepoState,
    host: PlayerHost,
    editor: EditorState,
    player_audio: ManualBackend,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-player"), &root);
    let repo = RepoState::new(root, Box::new(ScriptedPicker::new(dir.path().join("answers"))));
    let player_audio = ManualBackend::new();
    let editor_audio = ManualBackend::new();
    let host = PlayerHost::new(Arc::new(player_audio.clone()), None);
    let editor = EditorState::new(Arc::new(editor_audio.clone()));
    Env { dir, repo, host, editor, player_audio }
}

fn open(e: &Env, id: &str) -> Result<PlayerSnapshot, String> {
    do_open_player(&e.host, &e.repo, id, Arc::new(|_| {}))
}

fn state_path(e: &Env, id: &str) -> PathBuf {
    e.dir.path().join("lib/tracks").join(id).join("player.json")
}

#[test]
fn commands_need_the_open_session() {
    let e = env();
    let err = e.host.with(|m| m.pause("nope", Instant::now())).unwrap_err();
    assert_eq!(err, "no player session for nope");
    open(&e, PRACTICE).unwrap();
    let err = do_player_play(&e.host, &e.editor, STEMS_ONLY).unwrap_err();
    assert_eq!(err, "no player session for 0199c0a0-0000-7000-8000-000000000302");
}

#[test]
fn open_play_seek_pause_leave() {
    let e = env();
    let snap = open(&e, PRACTICE).unwrap();
    assert_eq!(snap.id, PRACTICE);
    assert!(!snap.sources.is_empty());
    let t = do_player_play(&e.host, &e.editor, PRACTICE).unwrap();
    assert!(t.playing);
    assert!(e.player_audio.is_open());
    let t = e.host.with(|m| m.seek(PRACTICE, 500, Instant::now())).unwrap();
    assert!(t.position_ms <= 800);
    let t = e.host.with(|m| m.pause(PRACTICE, Instant::now())).unwrap();
    assert!(!t.playing);
    let t = e.host.with(|m| m.skip(PRACTICE, 1000, Instant::now())).unwrap();
    assert!(t.position_ms >= 1000);
    let t = e.host.with(|m| m.leave(PRACTICE, Instant::now())).unwrap();
    assert!(!t.playing);
    assert!(!e.player_audio.is_open());
    assert!(state_path(&e, PRACTICE).exists());
}

#[test]
fn track_without_audio_cannot_open() {
    let e = env();
    let err = open(&e, MISSING_AUDIO).unwrap_err();
    assert!(err.contains("has no audio to play"), "{err}");
    assert!(e.host.with(|m| m.current()).is_none());
}

#[test]
fn select_source_keeps_the_session() {
    let e = env();
    let snap = open(&e, PRACTICE).unwrap();
    let other = snap.sources.iter().find(|s| s.key != snap.source).unwrap().key.clone();
    let after = do_player_select_source(&e.host, PRACTICE, &other).unwrap();
    assert_eq!(after.source, other);
    let same = do_player_select_source(&e.host, PRACTICE, &other).unwrap();
    assert_eq!(same.source, other);
    assert!(do_player_select_source(&e.host, PRACTICE, "stem:nothing").is_err());
}

#[test]
fn tempo_flags_and_loop_commands() {
    let e = env();
    open(&e, PRACTICE).unwrap();
    let err = do_player_tempo(&e.host, PRACTICE, TempoAction::SpeedStep { steps: 1 }).unwrap_err();
    assert_eq!(err, crate::player::NO_TEMPO);
    let snap = do_player_tempo(&e.host, PRACTICE, TempoAction::SetBpm { bpm: 120.0 }).unwrap();
    assert_eq!(snap.transport.bpm, 120.0);
    let a = TempoAction::ScaleSection { index: 0, at: snap.tempo.segments[0].start_ms, factor: Factor::Double };
    assert_eq!(do_player_tempo(&e.host, PRACTICE, a).unwrap().tempo.segments[0].bpm, 240.0);
    let a = TempoAction::RemoveChange { index: 1, at: 0.0 };
    assert_eq!(do_player_tempo(&e.host, PRACTICE, a).unwrap_err(), "The tempo sections changed; try again");
    let err = do_player_tempo(&e.host, PRACTICE, TempoAction::SpeedReset).unwrap_err();
    assert_eq!(err, crate::player::NO_SPEED);
    let start = snap.tempo.segments[0].start_ms;
    let a = TempoAction::SectionFlags { index: 0, at: start, metronome: false, count_in: true };
    let snap = do_player_tempo(&e.host, PRACTICE, a).unwrap();
    assert!(!snap.tempo.segments[0].meta.metronome && snap.tempo.segments[0].meta.count_in);
    let snap = do_player_loop(&e.host, &e.editor, PRACTICE, LoopCmd::Add).unwrap();
    assert_eq!(snap.loops.len(), 1);
    assert!(snap.loops[0].end_ms.is_some(), "Add gives the default end");
    let snap = do_player_loop(&e.host, &e.editor, PRACTICE, LoopCmd::Set { edge: Edge::Start, value: Boundary::Tick { tick: 16 } }).unwrap();
    assert_eq!(snap.loops[0].start, Boundary::Tick { tick: 16 });
}

#[test]
fn action_json_matches_the_typescript_types() {
    let t: TempoAction = serde_json::from_str(r#"{"kind":"set-bpm","bpm":92.5}"#).unwrap();
    assert_eq!(t, TempoAction::SetBpm { bpm: 92.5 });
    let t: TempoAction = serde_json::from_str(r#"{"kind":"speed-step","steps":-2}"#).unwrap();
    assert_eq!(t, TempoAction::SpeedStep { steps: -2 });
    assert_eq!(serde_json::from_str::<TempoAction>(r#"{"kind":"speed-reset"}"#).unwrap(), TempoAction::SpeedReset);
    let sec = |j: &str| serde_json::from_str::<TempoAction>(j);
    assert_eq!(sec(r#"{"kind":"section-bpm","index":1,"at":8500.0,"bpm":93.5}"#).unwrap(), TempoAction::SectionBpm { index: 1, at: 8500.0, bpm: 93.5 });
    assert_eq!(sec(r#"{"kind":"scale-section","index":2,"at":9.5,"factor":"halve"}"#).unwrap(), TempoAction::ScaleSection { index: 2, at: 9.5, factor: Factor::Halve });
    assert!(sec(r#"{"kind":"scale-section","index":2,"at":9.5,"factor":"triple"}"#).is_err());
    assert_eq!(sec(r#"{"kind":"add-change","position":4000.0,"bpm":100}"#).unwrap(), TempoAction::AddChange { position: 4000.0, bpm: 100.0 });
    assert_eq!(sec(r#"{"kind":"remove-change","index":1,"at":8500.0}"#).unwrap(), TempoAction::RemoveChange { index: 1, at: 8500.0 });
    assert_eq!(sec(r#"{"kind":"move-change","index":1,"at":8500.0,"steps":-3}"#).unwrap(), TempoAction::MoveChange { index: 1, at: 8500.0, steps: -3 });
    assert_eq!(
        sec(r#"{"kind":"section-flags","index":1,"at":8500.0,"metronome":false,"count_in":true}"#).unwrap(),
        TempoAction::SectionFlags { index: 1, at: 8500.0, metronome: false, count_in: true }
    );
    assert_eq!(
        sec(r#"{"kind":"section-name","index":1,"at":8500.0,"name":"Verse"}"#).unwrap(),
        TempoAction::SectionName { index: 1, at: 8500.0, name: "Verse".into() }
    );
    assert_eq!(
        sec(r#"{"kind":"jump-section","index":0,"at":500.0,"edge":"end"}"#).unwrap(),
        TempoAction::JumpSection { index: 0, at: 500.0, edge: Edge::End }
    );
    assert!(sec(r#"{"kind":"jump-section","index":0,"at":500.0,"edge":"middle"}"#).is_err());
    let cases = [
        (r#"{"kind":"add"}"#, LoopCmd::Add),
        (r#"{"kind":"remove"}"#, LoopCmd::Remove),
        (r#"{"kind":"select","delta":-1}"#, LoopCmd::Select { delta: -1 }),
        (r#"{"kind":"mark","edge":"end"}"#, LoopCmd::Mark { edge: Edge::End }),
        (r#"{"kind":"set","edge":"start","value":{"tick":32}}"#, LoopCmd::Set { edge: Edge::Start, value: Boundary::Tick { tick: 32 } }),
        (r#"{"kind":"set","edge":"end","value":{"ms":1500.5}}"#, LoopCmd::Set { edge: Edge::End, value: Boundary::Ms { ms: 1500.5 } }),
        (r#"{"kind":"flags","repeat":true,"count_in":false}"#, LoopCmd::Flags { repeat: true, count_in: false }),
        (r#"{"kind":"play"}"#, LoopCmd::Play),
        (r#"{"kind":"stop"}"#, LoopCmd::Stop),
    ];
    for (json, want) in cases {
        assert_eq!(serde_json::from_str::<LoopCmd>(json).unwrap(), want, "{json}");
    }
    assert!(serde_json::from_str::<LoopCmd>(r#"{"kind":"mark","edge":"middle"}"#).is_err());
}

#[test]
fn player_play_stops_the_editor_and_editor_play_pauses_the_player() {
    let e = env();
    let host = Arc::new(PlayerHost::new(Arc::new(e.player_audio.clone()), None));
    let h = host.clone();
    e.editor.set_before_play(Arc::new(move || h.pause_current()));
    // Editor first.
    let ed = crate::ipc::do_open_editor(&e.editor, &e.repo, &crate::tempo_store::FileTempoStore, PRACTICE, Arc::new(|_| {})).unwrap();
    assert_eq!(ed.id, PRACTICE);
    do_open_player(&host, &e.repo, PRACTICE, Arc::new(|_| {})).unwrap();
    crate::ipc::do_editor_play(&e.editor, PRACTICE).unwrap();
    assert!(e.editor.with(|m| m.current()).unwrap().transport.playing);
    // Player Play stops the Editor.
    let t = do_player_play(&host, &e.editor, PRACTICE).unwrap();
    assert!(t.playing);
    let es = e.editor.with(|m| m.current()).unwrap();
    assert!(!es.transport.playing);
    assert_eq!(es.transport.position_ms, 0);
    // Editor Play pauses the Player.
    crate::ipc::do_editor_play(&e.editor, PRACTICE).unwrap();
    assert!(!host.with(|m| m.current()).unwrap().transport.playing);
    assert!(e.editor.with(|m| m.current()).unwrap().transport.playing);
}

#[test]
fn shutdown_pauses_closes_and_saves_the_position() {
    let e = env();
    open(&e, PRACTICE).unwrap();
    do_player_play(&e.host, &e.editor, PRACTICE).unwrap();
    e.host.with(|m| m.seek(PRACTICE, 1200, Instant::now())).unwrap();
    e.host.shutdown(Duration::from_secs(2));
    assert!(e.host.with(|m| m.current()).is_none());
    assert!(!e.player_audio.is_open());
    let (st, _, _) = player_state::load(state_path(&e, PRACTICE).parent().unwrap());
    assert!(st.position_ms >= 1000, "{}", st.position_ms);
}

#[test]
fn tick_runs_without_a_session_and_with_one() {
    let e = env();
    e.host.tick(Instant::now());
    open(&e, PRACTICE).unwrap();
    e.host.tick(Instant::now());
}

#[test]
fn tempo_store_flushes_on_load_and_releases_on_replace() {
    use crate::tempo_map::{Segment, SectionMeta, TempoMap};
    use crate::tempo_store::TempoStore;
    let e = env();
    let host = Arc::new(PlayerHost::new(Arc::new(e.player_audio.clone()), None));
    let store = PlayerTempoStore::new(host.clone());
    let sink: crate::player::EventSink = Arc::new(|_| {});
    do_open_player(&host, &e.repo, PRACTICE, sink).unwrap();
    let dir = state_path(&e, PRACTICE).parent().unwrap().to_path_buf();
    host.with(|m| m.seek(PRACTICE, 3000, Instant::now())).unwrap();
    assert!(!state_path(&e, PRACTICE).exists());
    store.load(PRACTICE, &dir).unwrap();
    assert!(state_path(&e, PRACTICE).exists(), "load flushed the dirty session");
    assert!(host.with(|m| m.is_open(PRACTICE)), "and left it open");

    host.with(|m| m.seek(PRACTICE, 5000, Instant::now())).unwrap();
    let map = TempoMap { beats_per_bar: 4, segments: vec![Segment { start_ms: 0.0, bpm: 90.0, detected_ms: None, on_beat: false, meta: SectionMeta::default() }] };
    store.replace(PRACTICE, &dir, &|_| Ok(map.clone())).unwrap();
    assert!(!host.with(|m| m.is_open(PRACTICE)));
    let st = player_state::load(&dir).0;
    assert_eq!(st.position_ms, 5000);
    assert_eq!(st.tempo, map);
}

/// QA: M1 at the IPC boundary. A start that went out in the JSON snapshot comes back in a JSON
/// `TempoAction` and must be accepted by every section action, at non-dyadic positions.
#[test]
fn qa_json_roundtrip_starts_are_accepted_by_every_section_action() {
    let e = env();
    open(&e, PRACTICE).unwrap();
    do_player_tempo(&e.host, PRACTICE, TempoAction::SetBpm { bpm: 93.0 }).unwrap();
    let act = |j: String| serde_json::from_str::<TempoAction>(&j).unwrap();
    for n in 0..12 {
        let pos = 1777.3 + n as f64 * 1234.567 + 60000.0 / 93.0 * 0.123;
        let s = do_player_tempo(&e.host, PRACTICE, TempoAction::AddChange { position: pos, bpm: 100.0 }).unwrap();
        // what the front end sees: the snapshot as JSON text
        let text = serde_json::to_string(&s).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let segs = v["tempo"]["segments"].as_array().unwrap();
        let index = segs.iter().position(|g| (g["start_ms"].as_f64().unwrap() - pos).abs() < 40.0).unwrap();
        assert!(index >= 1);
        // the raw number text, as the browser would echo it back
        let raw = segs[index]["start_ms"].to_string();
        let a = |kind: &str, extra: &str| act(format!(r#"{{"kind":"{kind}","index":{index},"at":{raw}{extra}}}"#));
        let ok = |t: TempoAction, what: &str| {
            do_player_tempo(&e.host, PRACTICE, t).unwrap_or_else(|er| panic!("{what} at {raw} (n={n}): {er}"))
        };
        ok(a("section-bpm", r#","bpm":110"#), "section-bpm");
        ok(a("scale-section", r#","factor":"double""#), "scale-section");
        ok(a("scale-section", r#","factor":"halve""#), "scale-section halve");
        ok(a("move-change", r#","steps":1"#), "move-change");
        // the move changed the start; re-read it from JSON again, then remove
        let v: serde_json::Value = serde_json::to_value(e.host.with(|m| m.snapshot(PRACTICE)).unwrap()).unwrap();
        let segs = v["tempo"]["segments"].as_array().unwrap();
        let moved = segs.iter().position(|g| g["start_ms"].as_f64().unwrap() > pos + 1.0 && g["bpm"].as_f64() != Some(93.0)).unwrap_or(index);
        let raw2 = segs[moved]["start_ms"].to_string();
        let t = act(format!(r#"{{"kind":"remove-change","index":{moved},"at":{raw2}}}"#));
        let after = ok(t, "remove-change");
        assert_eq!(after.tempo.segments.len(), segs.len() - 1);
    }
}
