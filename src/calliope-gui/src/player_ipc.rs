//! Player commands (plan 2.9): `PlayerHost` (the Tauri-managed wrapper around `PlayerManager`,
//! the same shape as `EditorState`) and the thin async command wrappers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::Manager;

use crate::audio_out::OutputBackend;
use crate::ipc::{EditorState, RepoState};
use crate::player::{
    self, EventSink, LoopAction, OpenSpec, OutputRequest, PlayerEvent, PlayerManager, PlayerSnapshot,
    PlayerTransport, SectionEdit,
};
use crate::player_source;
use crate::player_state::Mark;
use crate::repository::Repository;
use crate::tempo_map::{Boundary, Scale, TempoMap};
use crate::tempo_store::{FileTempoStore, TempoStore};

/// Managed app state of the Player: the session manager plus the slot holding the webview's
/// current event channel (replaced by `open_player` and `watch_player`).
pub struct PlayerHost {
    mgr: Mutex<PlayerManager>,
    slot: Arc<Mutex<Option<EventSink>>>,
    ffmpeg: Option<PathBuf>,
}

impl PlayerHost {
    pub fn new(backend: Arc<dyn OutputBackend>, ffmpeg: Option<PathBuf>) -> Self {
        let slot: Arc<Mutex<Option<EventSink>>> = Arc::new(Mutex::new(None));
        let forward = slot.clone();
        let sink: EventSink = Arc::new(move |ev| {
            let target = forward.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(t) = target {
                t(ev);
            }
        });
        let mut mgr = PlayerManager::new(backend, sink);
        // Closing a real device can block: handles are dropped after the lock is released.
        mgr.defer_drops();
        Self { mgr: Mutex::new(mgr), slot, ffmpeg }
    }

    /// Runs `f` on the manager; retired output handles are dropped after the lock is released.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut PlayerManager) -> R) -> R {
        let (r, trash) = {
            let mut m = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
            let r = f(&mut m);
            (r, m.take_trash())
        };
        drop(trash);
        r
    }

    fn set_sink(&self, sink: EventSink) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    /// Ticker body (every 20 ms).
    pub fn tick(&self, now: Instant) {
        self.with(|m| m.tick(now));
    }

    /// Pauses the open session when it is sounding (the Editor is about to play).
    pub fn pause_current(&self) {
        self.with(|m| {
            if let Some((id, true)) = m.current_id_playing() {
                let _ = m.pause(&id, Instant::now());
            }
        });
    }

    /// App exit: pauses, closes the session (flushing `player.json` with the position) and
    /// closes the device, within about `cap`.
    pub fn shutdown(&self, cap: Duration) {
        let end = Instant::now() + cap;
        let lock_end = Instant::now() + cap / 2;
        let guard = loop {
            match self.mgr.try_lock() {
                Ok(g) => break Some(g),
                Err(std::sync::TryLockError::Poisoned(e)) => break Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if Instant::now() >= lock_end {
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };
        let Some(mut m) = guard else {
            eprintln!("calliope: player busy at exit; its state was not flushed");
            return;
        };
        if let Some((id, _)) = m.current_id_playing() {
            let _ = m.pause(&id, Instant::now());
        }
        m.close();
        let trash = m.take_trash();
        drop(m);
        // Closing the device may hang: do it on a helper thread and wait a little.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(trash);
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(end.saturating_duration_since(Instant::now()));
    }
}

/// The app's tempo store (plan 2.2): `player.json`, with the Player's session for the track
/// flushed (`load`) or released (`replace`) first, both under one lock of the manager, so
/// neither side loses the other's writes.
pub(crate) struct PlayerTempoStore {
    host: Arc<PlayerHost>,
}

impl PlayerTempoStore {
    pub(crate) fn new(host: Arc<PlayerHost>) -> Self {
        Self { host }
    }
}

impl TempoStore for PlayerTempoStore {
    fn load(&self, id: &str, dir: &Path) -> Result<TempoMap, String> {
        self.host.with(|m| {
            m.flush_if_open(id)?;
            FileTempoStore.load(id, dir)
        })
    }

    fn replace(&self, id: &str, dir: &Path, f: &dyn Fn(&TempoMap) -> Result<TempoMap, String>) -> Result<TempoMap, String> {
        self.host.with(|m| {
            m.release(id)?;
            FileTempoStore.replace(id, dir, f)
        })
    }
}

// ---- command bodies ----

pub(crate) fn do_open_player(host: &PlayerHost, repo: &RepoState, id: &str, sink: EventSink) -> Result<PlayerSnapshot, String> {
    // The open is registered before anything slow, so overlapping opens are ordered by their
    // arrival here and the newest one owns the event sink.
    let (ticket, events) = host.with(|m| {
        host.set_sink(sink);
        (m.begin_open(), m.sink())
    });
    let result = (|| {
        let spec = {
            let root = repo.lock();
            let r = Repository::new(root.clone());
            let rec = r.load_record(id)?;
            OpenSpec {
                dir: r.tracks_dir().join(&rec.id),
                id: rec.id.clone(),
                title: rec.title.clone(),
                sources: player_source::sources(&rec),
                ffmpeg: host.ffmpeg.clone(),
            }
        };
        let loaded = player::load(&spec, &ticket, &events)?;
        host.with(|m| m.finish_open(&ticket, &spec, loaded))
    })();
    if result.is_err() {
        host.with(|m| m.abort_open(&ticket));
    }
    result
}

pub(crate) fn do_close_player(host: &PlayerHost) {
    host.with(|m| m.close());
}

/// Re-attach after a webview reload. It does not take the sink from an open in progress.
pub(crate) fn do_watch_player(host: &PlayerHost, sink: EventSink) -> Option<PlayerSnapshot> {
    host.with(|m| {
        if !m.is_opening() {
            host.set_sink(sink);
        }
        m.current()
    })
}

/// One sound at a time: a sounding Editor is stopped.
fn stop_editor(editor: &EditorState) {
    editor.with(|m| {
        if let Some(s) = m.current() {
            if s.transport.playing || s.transport.resume_pending {
                let _ = m.stop(&s.id);
            }
        }
    });
}

pub(crate) fn do_player_play(host: &PlayerHost, editor: &EditorState, id: &str) -> Result<PlayerTransport, String> {
    let req = host.with(|m| m.play_request(id))?;
    stop_editor(editor);
    // The device is opened without the manager lock.
    let opened = req.map(OutputRequest::open).transpose()?;
    host.with(|m| m.play_with(id, opened, Instant::now()))
}

pub(crate) fn do_player_select_source(host: &PlayerHost, id: &str, key: &str) -> Result<PlayerSnapshot, String> {
    let Some(job) = host.with(|m| m.begin_select(id, key, Instant::now()))? else {
        return host.with(|m| m.snapshot(id));
    };
    let sink = host.with(|m| m.sink());
    match player::load_source(&job, &sink) {
        Ok(loaded) => host.with(|m| m.finish_select(&job, loaded, Instant::now())),
        Err(e) => {
            host.with(|m| m.abort_select(&job));
            Err(e)
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Edge {
    Start,
    End,
}

impl From<Edge> for Mark {
    fn from(e: Edge) -> Mark {
        match e {
            Edge::Start => Mark::Start,
            Edge::End => Mark::End,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum TempoAction {
    SetBpm { bpm: f64 },
    SpeedStep { steps: i32 },
    SpeedReset,
    SectionBpm { index: usize, at: f64, bpm: f64 },
    ScaleSection { index: usize, at: f64, factor: Factor },
    AddChange { position: f64, bpm: f64 },
    RemoveChange { index: usize, at: f64 },
    MoveChange { index: usize, at: f64, steps: i64 },
    SectionFlags { index: usize, at: f64, metronome: bool, count_in: bool },
    SectionName { index: usize, at: f64, name: String },
    JumpSection { index: usize, at: f64, edge: Edge },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Factor {
    Double,
    Halve,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum LoopCmd {
    Add,
    Remove,
    Select { delta: i32 },
    Mark { edge: Edge },
    Set { edge: Edge, value: Boundary },
    Flags { repeat: bool, count_in: bool },
    Play,
    Stop,
}

impl From<LoopCmd> for LoopAction {
    fn from(c: LoopCmd) -> LoopAction {
        match c {
            LoopCmd::Add => LoopAction::Add,
            LoopCmd::Remove => LoopAction::Remove,
            LoopCmd::Select { delta } => LoopAction::Select(delta),
            LoopCmd::Mark { edge } => LoopAction::Mark(edge.into()),
            LoopCmd::Set { edge, value } => LoopAction::Set(edge.into(), value),
            LoopCmd::Flags { repeat, count_in } => LoopAction::Flags { repeat, count_in },
            LoopCmd::Play => LoopAction::Play,
            LoopCmd::Stop => LoopAction::Stop,
        }
    }
}

fn edit(host: &PlayerHost, id: &str, e: SectionEdit) -> Result<PlayerSnapshot, String> {
    host.with(|m| m.edit_sections(id, e, Instant::now()))
}

pub(crate) fn do_player_tempo(host: &PlayerHost, id: &str, action: TempoAction) -> Result<PlayerSnapshot, String> {
    match action {
        TempoAction::SetBpm { bpm } => host.with(|m| m.set_bpm(id, bpm, Instant::now())),
        TempoAction::SpeedStep { .. } | TempoAction::SpeedReset => host.with(|m| m.speed_action(id)),
        TempoAction::SectionBpm { index, at, bpm } => edit(host, id, SectionEdit::Bpm { index, at, bpm }),
        TempoAction::ScaleSection { index, at, factor } => {
            let factor = if factor == Factor::Double { Scale::Double } else { Scale::Halve };
            edit(host, id, SectionEdit::Scale { index, at, factor })
        }
        TempoAction::AddChange { position, bpm } => edit(host, id, SectionEdit::Add { position, bpm }),
        TempoAction::RemoveChange { index, at } => edit(host, id, SectionEdit::Remove { index, at }),
        TempoAction::MoveChange { index, at, steps } => edit(host, id, SectionEdit::Move { index, at, steps }),
        TempoAction::SectionFlags { index, at, metronome, count_in } => {
            edit(host, id, SectionEdit::Flags { index, at, metronome, count_in })
        }
        TempoAction::SectionName { index, at, name } => edit(host, id, SectionEdit::Name { index, at, name }),
        TempoAction::JumpSection { index, at, edge } => {
            let edge = if edge == Edge::Start { crate::tempo_map::Edge::Start } else { crate::tempo_map::Edge::End };
            host.with(|m| m.jump_section(id, index, at, edge, Instant::now()))
        }
    }
}

pub(crate) fn do_player_loop(host: &PlayerHost, editor: &EditorState, id: &str, cmd: LoopCmd) -> Result<PlayerSnapshot, String> {
    if cmd == LoopCmd::Play {
        let req = host.with(|m| m.loop_play_request(id))?;
        stop_editor(editor);
        let opened = req.map(OutputRequest::open).transpose()?;
        return host.with(|m| m.loop_play_with(id, opened, Instant::now()));
    }
    host.with(|m| m.loop_action(id, cmd.into(), Instant::now()))
}

fn event_sink(events: tauri::ipc::Channel<PlayerEvent>) -> EventSink {
    Arc::new(move |ev| {
        let _ = events.send(ev);
    })
}

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

// ---- Tauri commands ----

#[tauri::command]
pub(crate) async fn open_player(
    app: tauri::AppHandle,
    id: String,
    events: tauri::ipc::Channel<PlayerEvent>,
) -> Result<PlayerSnapshot, String> {
    blocking(move || do_open_player(&app.state::<Arc<PlayerHost>>(), &app.state::<RepoState>(), &id, event_sink(events))).await
}

#[tauri::command]
pub(crate) async fn close_player(app: tauri::AppHandle) -> Result<(), String> {
    blocking(move || {
        do_close_player(&app.state::<Arc<PlayerHost>>());
        Ok(())
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_player(app: tauri::AppHandle) -> Result<Option<PlayerSnapshot>, String> {
    blocking(move || Ok(app.state::<Arc<PlayerHost>>().with(|m| m.current()))).await
}

#[tauri::command]
pub(crate) async fn watch_player(
    app: tauri::AppHandle,
    events: tauri::ipc::Channel<PlayerEvent>,
) -> Result<Option<PlayerSnapshot>, String> {
    blocking(move || Ok(do_watch_player(&app.state::<Arc<PlayerHost>>(), event_sink(events)))).await
}

#[tauri::command]
pub(crate) async fn player_select_source(app: tauri::AppHandle, id: String, source: String) -> Result<PlayerSnapshot, String> {
    blocking(move || do_player_select_source(&app.state::<Arc<PlayerHost>>(), &id, &source)).await
}

#[tauri::command]
pub(crate) async fn player_play(app: tauri::AppHandle, id: String) -> Result<PlayerTransport, String> {
    blocking(move || do_player_play(&app.state::<Arc<PlayerHost>>(), &app.state::<EditorState>(), &id)).await
}

#[tauri::command]
pub(crate) async fn player_pause(app: tauri::AppHandle, id: String) -> Result<PlayerTransport, String> {
    blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.pause(&id, Instant::now()))).await
}

#[tauri::command]
pub(crate) async fn player_seek(app: tauri::AppHandle, id: String, position: u64) -> Result<PlayerTransport, String> {
    blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.seek(&id, position, Instant::now()))).await
}

#[tauri::command]
pub(crate) async fn player_nudge(app: tauri::AppHandle, id: String, delta: i64) -> Result<PlayerTransport, String> {
    blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.nudge(&id, delta, Instant::now()))).await
}

#[tauri::command]
pub(crate) async fn player_skip(app: tauri::AppHandle, id: String, delta: i64) -> Result<PlayerTransport, String> {
    blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.skip(&id, delta, Instant::now()))).await
}

#[tauri::command]
pub(crate) async fn player_leave(app: tauri::AppHandle, id: String) -> Result<PlayerTransport, String> {
    blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.leave(&id, Instant::now()))).await
}

#[tauri::command]
pub(crate) async fn player_tempo(app: tauri::AppHandle, id: String, action: TempoAction) -> Result<PlayerSnapshot, String> {
    blocking(move || do_player_tempo(&app.state::<Arc<PlayerHost>>(), &id, action)).await
}

#[tauri::command]
pub(crate) async fn player_loop(app: tauri::AppHandle, id: String, action: LoopCmd) -> Result<PlayerSnapshot, String> {
    blocking(move || do_player_loop(&app.state::<Arc<PlayerHost>>(), &app.state::<EditorState>(), &id, action)).await
}

#[cfg(test)]
#[path = "player_ipc_tests.rs"]
mod tests;
