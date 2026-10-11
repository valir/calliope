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
fn tempo_metronome_and_loop_commands() {
    let e = env();
    open(&e, PRACTICE).unwrap();
    let err = do_player_tempo(&e.host, PRACTICE, TempoAction::SpeedStep { steps: 1 }).unwrap_err();
    assert_eq!(err, crate::player::NO_TEMPO);
    let snap = do_player_tempo(&e.host, PRACTICE, TempoAction::SetBpm { bpm: 120.0 }).unwrap();
    assert_eq!(snap.transport.bpm, 120.0);
    let err = do_player_tempo(&e.host, PRACTICE, TempoAction::SpeedReset).unwrap_err();
    assert_eq!(err, crate::player::NO_SPEED);
    let snap = e.host.with(|m| m.set_metronome(PRACTICE, true, true, Instant::now())).unwrap();
    assert!(snap.metronome.on && snap.metronome.count_in);
    let snap = do_player_loop(&e.host, &e.editor, PRACTICE, LoopCmd::Add).unwrap();
    assert_eq!(snap.loops.len(), 1);
    let err = do_player_loop(&e.host, &e.editor, PRACTICE, LoopCmd::Play).unwrap_err();
    assert_eq!(err, crate::player::LOOP_INCOMPLETE);
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
    let ed = crate::ipc::do_open_editor(&e.editor, &e.repo, PRACTICE, Arc::new(|_| {})).unwrap();
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
    use crate::tempo_map::{Segment, TempoMap};
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
    let map = TempoMap { beats_per_bar: 4, segments: vec![Segment { start_ms: 0.0, bpm: 90.0, detected_ms: None }] };
    store.replace(PRACTICE, &dir, &|_| Ok(map.clone())).unwrap();
    assert!(!host.with(|m| m.is_open(PRACTICE)));
    let st = player_state::load(&dir).0;
    assert_eq!(st.position_ms, 5000);
    assert_eq!(st.tempo, map);
}
