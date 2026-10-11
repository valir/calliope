//! The Player (plan 2.7): the real-time `Engine` owned by the audio callback and the
//! `PlayerManager` that opens a track, drives the engine through a bounded command channel,
//! and persists `player.json` (debounced, plus a periodic position save while playing).
//! Metronome, count-in and loops come in the next task.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::audio_out::{OutputBackend, OutputHandle};
use crate::player_source::{self, Pcm, Source, PEAK_BUCKETS};
use crate::player_state::{self, LoopRec, PlayerState};
use crate::tempo_map::{Boundary, Segment};
use crate::transport::Transport;

pub(crate) const SUPERSEDED: &str = "superseded";
const OUTPUT_LOST: &str = "Playback was interrupted; try again";
/// A change is written this long after the last change.
pub(crate) const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// While playing, the position is saved this often.
pub(crate) const POSITION_SAVE: Duration = Duration::from_secs(10);
/// Minimum time between `transport` events while only the position moves.
pub(crate) const EVENT_INTERVAL: Duration = Duration::from_millis(100);
const FADE_MS: u32 = 3;
const COMMANDS: usize = 64;

// ---------------------------------------------------------------------- engine

/// Commands from the manager to the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmd {
    /// Start sound at the engine position (restarts from 0 at the end of the track).
    Play,
    Pause,
    /// Move; while playing the sound stays off until `Resume` (the scrub rule).
    Seek(u64),
    /// Move at once, sound continues (with a fade-in).
    Jump(u64),
    /// End of the scrub silence.
    Resume,
}

/// Status written by the callback, read by the manager (atomics only).
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// Position in frames. The manager also stores here when no output is open.
    pub position: AtomicU64,
    /// The source played to its end.
    pub ended: AtomicBool,
}

/// Pure render state; never allocates or locks.
pub(crate) struct Engine {
    pcm: Arc<Pcm>,
    pos: u64,
    playing: bool,
    audible: bool,
    fade_len: u32,
    fade_left: u32,
    shared: Arc<Shared>,
}

impl Engine {
    pub fn new(pcm: Arc<Pcm>, pos: u64, shared: Arc<Shared>) -> Self {
        let fade_len = (pcm.rate * FADE_MS / 1000).max(1);
        let pos = pos.min(pcm.frames);
        Engine { pcm, pos, playing: false, audible: false, fade_len, fade_left: 0, shared }
    }

    fn start_fade(&mut self) {
        self.fade_left = self.fade_len;
    }

    pub fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play => {
                if self.pos >= self.pcm.frames {
                    self.pos = 0;
                }
                self.playing = true;
                self.audible = true;
                self.start_fade();
            }
            Cmd::Pause => {
                self.playing = false;
                self.audible = false;
            }
            Cmd::Seek(f) => {
                self.pos = f.min(self.pcm.frames);
                if self.playing {
                    self.audible = false;
                }
            }
            Cmd::Jump(f) => {
                self.pos = f.min(self.pcm.frames);
                if self.playing && self.audible {
                    self.start_fade();
                }
            }
            Cmd::Resume => {
                if self.playing {
                    self.audible = true;
                    self.start_fade();
                }
            }
        }
    }

    /// Fills an interleaved stereo block.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if self.playing && self.audible {
            let frames = out.len() / 2;
            let n = self.pcm.frames.saturating_sub(self.pos).min(frames as u64) as usize;
            let ch = self.pcm.channels.max(1) as usize;
            for i in 0..n {
                let base = (self.pos as usize + i) * ch;
                let l = f32::from(self.pcm.samples[base]) / 32768.0;
                let r = if ch >= 2 { f32::from(self.pcm.samples[base + 1]) / 32768.0 } else { l };
                let g = if self.fade_left > 0 {
                    let g = (self.fade_len - self.fade_left) as f32 / self.fade_len as f32;
                    self.fade_left -= 1;
                    g
                } else {
                    1.0
                };
                out[2 * i] = l * g;
                out[2 * i + 1] = r * g;
            }
            self.pos += n as u64;
            if self.pos >= self.pcm.frames {
                self.playing = false;
                self.audible = false;
                self.shared.ended.store(true, Ordering::Release);
            }
        }
        self.shared.position.store(self.pos, Ordering::Release);
    }
}

// -------------------------------------------------------------------- IPC types

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PlayerTransport {
    pub playing: bool,
    pub counting_in: bool,
    pub resume_pending: bool,
    pub position_ms: u64,
    pub loop_playing: bool,
    pub bpm: f64,
    pub tempo_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SourceEntry {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PlayerLoop {
    pub id: String,
    pub start: Boundary,
    pub end: Option<Boundary>,
    pub start_ms: f64,
    pub end_ms: Option<f64>,
    pub repeat: bool,
    pub count_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SnapTempo {
    pub beats_per_bar: u32,
    pub segments: Vec<Segment>,
    pub multi: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SnapMetronome {
    pub on: bool,
    pub count_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct PlayerSnapshot {
    pub id: String,
    pub title: String,
    pub sources: Vec<SourceEntry>,
    pub source: String,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub peaks: Vec<f32>,
    pub mode: &'static str,
    pub tempo: SnapTempo,
    pub speed: f64,
    pub metronome: SnapMetronome,
    pub loops: Vec<PlayerLoop>,
    pub current_loop: Option<usize>,
    pub loop_message: Option<String>,
    pub transport: PlayerTransport,
    pub can_change_speed: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum PlayerEvent {
    Loading {
        id: String,
        progress: f64,
    },
    Transport {
        id: String,
        #[serde(flatten)]
        state: PlayerTransport,
    },
    AudioError {
        id: String,
        message: String,
    },
    StateError {
        id: String,
        message: String,
    },
}

pub(crate) type EventSink = Arc<dyn Fn(PlayerEvent) + Send + Sync>;

// ------------------------------------------------------------------ open inputs

/// What the IPC layer extracts from the track record.
#[derive(Debug, Clone)]
pub(crate) struct OpenSpec {
    pub id: String,
    pub title: String,
    pub dir: PathBuf,
    /// `player_source::sources(record)`; files are relative to `dir`.
    pub sources: Vec<Source>,
    pub ffmpeg: Option<PathBuf>,
}

/// Handed out by `begin_open`; a later `begin_open` or `close` cancels it.
#[derive(Clone)]
pub(crate) struct OpenTicket {
    gen: u64,
    cancel: Arc<AtomicBool>,
}

/// A decoded track; produced without holding the manager.
pub(crate) struct Loaded {
    state: PlayerState,
    note: Option<String>,
    source: String,
    pcm: Pcm,
    peaks: Vec<f32>,
}

/// A source change in progress (`begin_select`).
#[derive(Clone)]
pub(crate) struct SelectJob {
    id: String,
    key: String,
    token: u64,
    dir: PathBuf,
    file: String,
    ffmpeg: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
}

pub(crate) struct LoadedSource {
    pcm: Pcm,
    peaks: Vec<f32>,
}

fn progress_sink(id: &str, sink: &EventSink) -> impl FnMut(u64, u64) {
    let id = id.to_string();
    let sink = sink.clone();
    let mut last = -1.0f64;
    move |done, total| {
        if total == 0 {
            return;
        }
        let p = (done as f64 / total as f64).clamp(0.0, 1.0);
        if p - last >= 0.01 || p >= 1.0 {
            last = p;
            sink(PlayerEvent::Loading { id: id.clone(), progress: p });
        }
    }
}

/// Reads `player.json`, picks the source and decodes it (long; call without the manager lock).
pub(crate) fn load(spec: &OpenSpec, ticket: &OpenTicket, sink: &EventSink) -> Result<Loaded, String> {
    let (mut state, note, _writable) = player_state::load(&spec.dir);
    let keys: Vec<&str> = spec.sources.iter().map(|s| s.key.as_str()).collect();
    let source = player_state::choose_source(state.source.as_deref(), &keys)
        .ok_or_else(|| format!("{} has no audio to play.", spec.title))?;
    let entry = spec.sources.iter().find(|s| s.key == source).expect("chosen from the list");
    let mut progress = progress_sink(&spec.id, sink);
    let pcm = player_source::decode(&spec.dir.join(&entry.file), spec.ffmpeg.as_deref(), &ticket.cancel, &mut progress)?;
    if ticket.cancel.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    let peaks = player_source::peaks(&pcm, PEAK_BUCKETS);
    state.source = Some(source.clone());
    Ok(Loaded { state, note, source, pcm, peaks })
}

// ---------------------------------------------------------------------- manager

struct Output {
    handle: Box<dyn OutputHandle>,
    tx: SyncSender<Cmd>,
}

/// What it takes to open the device; `open` blocks and must run without the manager lock.
pub(crate) struct OutputRequest {
    backend: Arc<dyn OutputBackend>,
    shared: Arc<Shared>,
    pcm: Arc<Pcm>,
    start: u64,
}

pub(crate) struct OpenedOutput {
    shared: Arc<Shared>,
    output: Output,
}

impl OutputRequest {
    pub fn open(self) -> Result<OpenedOutput, String> {
        let (tx, rx): (SyncSender<Cmd>, Receiver<Cmd>) = sync_channel(COMMANDS);
        let mut engine = Engine::new(self.pcm.clone(), self.start, self.shared.clone());
        let handle = self.backend.open(
            self.pcm.rate,
            Box::new(move |out| {
                while let Ok(c) = rx.try_recv() {
                    engine.apply(c);
                }
                engine.render(out);
            }),
        )?;
        Ok(OpenedOutput { shared: self.shared, output: Output { handle, tx } })
    }
}

#[derive(PartialEq, Clone, Copy)]
struct EventKey {
    playing: bool,
    resume_pending: bool,
    counting_in: bool,
}

struct Session {
    id: String,
    title: String,
    dir: PathBuf,
    sources: Vec<Source>,
    ffmpeg: Option<PathBuf>,
    state: PlayerState,
    note: Option<String>,
    pcm: Arc<Pcm>,
    peaks: Vec<f32>,
    shared: Arc<Shared>,
    transport: Transport,
    output: Option<Output>,
    dirty_at: Option<Instant>,
    next_pos_save: Option<Instant>,
    /// Token of the source change in progress.
    switching: Option<u64>,
    select_gen: u64,
    select_cancel: Option<Arc<AtomicBool>>,
    last_key: EventKey,
    last_position: u64,
    last_emit: Option<Instant>,
}

impl Session {
    fn pos(&self) -> u64 {
        self.shared.position.load(Ordering::Acquire)
    }

    fn set_pos(&self, frames: u64) {
        self.shared.position.store(frames.min(self.pcm.frames), Ordering::Release);
    }

    fn ms_of(&self, frames: u64) -> u64 {
        frames * 1000 / u64::from(self.pcm.rate.max(1))
    }

    fn frames_of(&self, ms: u64) -> u64 {
        (ms * u64::from(self.pcm.rate) / 1000).min(self.pcm.frames)
    }

    fn send(&self, cmd: Cmd) {
        if let Some(o) = &self.output {
            let _ = o.tx.try_send(cmd);
        }
    }

    fn mark_dirty(&mut self, now: Instant) {
        if !self.state.read_only {
            self.dirty_at = Some(now);
        }
    }

    fn key(&self) -> EventKey {
        EventKey {
            playing: self.transport.shows_playing(),
            resume_pending: self.transport.resume_pending(),
            counting_in: false,
        }
    }

    fn transport_state(&self) -> PlayerTransport {
        let ms = self.ms_of(self.pos());
        PlayerTransport {
            playing: self.transport.shows_playing(),
            counting_in: false,
            resume_pending: self.transport.resume_pending(),
            position_ms: ms,
            loop_playing: false,
            bpm: self.state.tempo.bpm_at(ms as f64) * self.state.speed,
            tempo_index: self.state.tempo.segment_at(ms as f64),
        }
    }

    fn snapshot(&self) -> PlayerSnapshot {
        let st = &self.state;
        let ms_of = |b: Boundary| match b {
            Boundary::Ms { ms } => ms,
            Boundary::Tick { tick } => st.tempo.ms_at(tick),
        };
        let loops: Vec<PlayerLoop> = st.loops.iter().map(|l: &LoopRec| PlayerLoop {
            id: l.id.clone(),
            start: l.start,
            end: l.end,
            start_ms: ms_of(l.start),
            end_ms: l.end.map(ms_of),
            repeat: l.repeat,
            count_in: l.count_in,
        })
        .collect();
        let current_loop = st.current_loop.as_ref().and_then(|c| st.loops.iter().position(|l| &l.id == c));
        PlayerSnapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            sources: self.sources.iter().map(|s| SourceEntry { key: s.key.clone(), label: s.label.clone() }).collect(),
            source: st.source.clone().unwrap_or_default(),
            duration_ms: self.ms_of(self.pcm.frames),
            sample_rate: self.pcm.rate,
            peaks: self.peaks.clone(),
            mode: if st.tempo.is_empty() { "time" } else { "bpm" },
            tempo: SnapTempo {
                beats_per_bar: st.tempo.beats_per_bar,
                segments: st.tempo.segments.clone(),
                multi: st.tempo.is_multi(),
            },
            speed: st.speed,
            metronome: SnapMetronome { on: st.metronome.on, count_in: st.metronome.count_in },
            loops,
            current_loop,
            loop_message: None,
            transport: self.transport_state(),
            can_change_speed: false,
            note: self.note.clone(),
        }
    }
}

/// Writes `player.json` with the live position. Errors are reported, not retried until the
/// next change.
fn save_session(sink: &EventSink, s: &mut Session) {
    s.dirty_at = None;
    if s.state.read_only {
        return;
    }
    s.state.position_ms = s.ms_of(s.pos());
    match player_state::save(&s.dir, &s.state) {
        Ok(()) => eprintln!("calliope: player state saved id={}", s.id),
        Err(message) => {
            eprintln!("calliope: player state error id={} message={message}", s.id);
            sink(PlayerEvent::StateError {
                id: s.id.clone(),
                message: format!("Could not save the player settings: {message}"),
            });
        }
    }
}

/// Writes when something changed since the last write (a dirty mark or a moved position).
fn flush_session(sink: &EventSink, s: &mut Session) {
    if s.dirty_at.is_some() || s.ms_of(s.pos()) != s.state.position_ms {
        save_session(sink, s);
    }
}

fn publish(sink: &EventSink, s: &mut Session, now: Instant) -> PlayerTransport {
    let state = s.transport_state();
    let key = s.key();
    let key_changed = key != s.last_key;
    let moved = state.position_ms != s.last_position;
    let due = s.last_emit.is_none_or(|t| now.saturating_duration_since(t) >= EVENT_INTERVAL);
    if key_changed || (key.playing && due) || (!key.playing && moved) {
        if key_changed {
            eprintln!(
                "calliope: player transport playing={} counting_in={} loop=none position_ms={}",
                state.playing, state.counting_in, state.position_ms
            );
        }
        s.last_key = key;
        s.last_position = state.position_ms;
        s.last_emit = Some(now);
        sink(PlayerEvent::Transport { id: s.id.clone(), state: state.clone() });
    }
    state
}

pub(crate) struct PlayerManager {
    backend: Arc<dyn OutputBackend>,
    sink: EventSink,
    session: Option<Session>,
    pending: Option<OpenTicket>,
    generation: u64,
    /// Output handles taken out of sessions. Closing a real device can block, so when
    /// `defer_drops` is set they wait here until the caller has released the manager lock.
    trash: Vec<Box<dyn OutputHandle>>,
    defer_drops: bool,
}

impl PlayerManager {
    pub fn new(backend: Arc<dyn OutputBackend>, sink: EventSink) -> Self {
        Self { backend, sink, session: None, pending: None, generation: 0, trash: Vec::new(), defer_drops: false }
    }

    pub fn defer_drops(&mut self) {
        self.defer_drops = true;
    }

    pub fn take_trash(&mut self) -> Vec<Box<dyn OutputHandle>> {
        std::mem::take(&mut self.trash)
    }

    pub fn sink(&self) -> EventSink {
        self.sink.clone()
    }

    fn retire(&mut self, handle: Option<Box<dyn OutputHandle>>) {
        if let Some(h) = handle {
            if self.defer_drops {
                self.trash.push(h);
            } else {
                drop(h);
            }
        }
    }

    // ------------------------------------------------------------------ open

    /// Closes (and flushes) the current session and cancels any load in progress.
    pub fn begin_open(&mut self) -> OpenTicket {
        self.close();
        self.generation += 1;
        let ticket = OpenTicket { gen: self.generation, cancel: Arc::new(AtomicBool::new(false)) };
        self.pending = Some(ticket.clone());
        ticket
    }

    pub fn is_opening(&self) -> bool {
        self.pending.is_some()
    }

    pub fn abort_open(&mut self, ticket: &OpenTicket) {
        if self.pending.as_ref().is_some_and(|t| t.gen == ticket.gen) {
            self.pending = None;
        }
    }

    /// Installs a loaded track unless a newer `begin_open`/`close` superseded the ticket.
    pub fn finish_open(&mut self, ticket: &OpenTicket, spec: &OpenSpec, loaded: Loaded) -> Result<PlayerSnapshot, String> {
        if ticket.gen != self.generation || ticket.cancel.load(Ordering::Relaxed) {
            return Err(SUPERSEDED.into());
        }
        self.pending = None;
        let Loaded { mut state, note, source, pcm, peaks } = loaded;
        let pcm = Arc::new(pcm);
        let mut frames = state.position_ms.saturating_mul(u64::from(pcm.rate)) / 1000;
        if frames >= pcm.frames {
            frames = 0; // past the end gives 0:00.0
        }
        let shared = Arc::new(Shared::default());
        shared.position.store(frames, Ordering::Release);
        state.source = Some(source.clone());
        let mut session = Session {
            id: spec.id.clone(),
            title: spec.title.clone(),
            dir: spec.dir.clone(),
            sources: spec.sources.clone(),
            ffmpeg: spec.ffmpeg.clone(),
            state,
            note,
            pcm,
            peaks,
            shared,
            transport: Transport::new(),
            output: None,
            dirty_at: None,
            next_pos_save: None,
            switching: None,
            select_gen: 0,
            select_cancel: None,
            last_key: EventKey { playing: false, resume_pending: false, counting_in: false },
            last_position: 0,
            last_emit: None,
        };
        session.state.position_ms = session.ms_of(frames); // what is on disk or equal to it
        let snap = session.snapshot();
        eprintln!(
            "calliope: player open id={} source={} duration_ms={} rate={} position_ms={}",
            snap.id, source, snap.duration_ms, snap.sample_rate, snap.transport.position_ms
        );
        let old = self.session.replace(session).and_then(|s| s.output).map(|o| o.handle);
        self.retire(old);
        Ok(snap)
    }

    /// Whole open in one call; the IPC layer splits it to keep the manager unlocked.
    pub fn open(&mut self, spec: &OpenSpec) -> Result<PlayerSnapshot, String> {
        let ticket = self.begin_open();
        let sink = self.sink.clone();
        let loaded = match load(spec, &ticket, &sink) {
            Ok(l) => l,
            Err(e) => {
                self.abort_open(&ticket);
                return Err(e);
            }
        };
        self.finish_open(&ticket, spec, loaded)
    }

    /// Flushes `player.json`, stops playback and drops the session.
    pub fn close(&mut self) {
        if let Some(t) = self.pending.take() {
            t.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(mut s) = self.session.take() {
            if let Some(c) = &s.select_cancel {
                c.store(true, Ordering::Relaxed);
            }
            s.send(Cmd::Pause);
            flush_session(&self.sink, &mut s);
            self.retire(s.output.take().map(|o| o.handle));
        }
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.id == id)
    }

    fn session(&mut self, id: &str) -> Result<&mut Session, String> {
        match self.session.as_mut() {
            Some(s) if s.id == id => Ok(s),
            _ => Err(format!("no player session for {id}")),
        }
    }

    pub fn snapshot(&self, id: &str) -> Result<PlayerSnapshot, String> {
        match self.session.as_ref() {
            Some(s) if s.id == id => Ok(s.snapshot()),
            _ => Err(format!("no player session for {id}")),
        }
    }

    pub fn current(&self) -> Option<PlayerSnapshot> {
        self.session.as_ref().map(Session::snapshot)
    }

    // ------------------------------------------------------------ transport

    /// Step 1 of Play (under the lock): says whether the device must be opened by the caller
    /// without the lock.
    pub fn play_request(&self, id: &str) -> Result<Option<OutputRequest>, String> {
        match self.session.as_ref() {
            Some(s) if s.id == id => {
                if s.switching.is_some() {
                    return Err("The source is still loading".into());
                }
                Ok(if s.output.is_some() {
                    None
                } else {
                    Some(OutputRequest {
                        backend: self.backend.clone(),
                        shared: s.shared.clone(),
                        pcm: s.pcm.clone(),
                        start: s.pos(),
                    })
                })
            }
            _ => Err(format!("no player session for {id}")),
        }
    }

    /// Stores a freshly opened handle unless the session went away, changed source or has one.
    fn install(&mut self, opened: Option<OpenedOutput>) {
        let Some(o) = opened else { return };
        match self.session.as_mut() {
            Some(s) if s.output.is_none() && Arc::ptr_eq(&s.shared, &o.shared) => {
                s.output = Some(o.output);
                s.send(Cmd::Seek(s.pos())); // the position may have moved since the request
            }
            _ => self.retire(Some(o.output.handle)),
        }
    }

    /// Step 2 of Play: starts. Nothing changes if the device failed to open.
    pub fn play_with(&mut self, id: &str, opened: Option<OpenedOutput>, now: Instant) -> Result<PlayerTransport, String> {
        self.session(id)?;
        self.install(opened);
        let sink = self.sink.clone();
        let s = self.session(id)?;
        if s.output.is_none() {
            return Err(OUTPUT_LOST.into());
        }
        if s.pos() >= s.pcm.frames {
            s.set_pos(0);
            s.send(Cmd::Seek(0));
        }
        s.transport.play();
        s.send(Cmd::Play);
        s.next_pos_save = Some(now + POSITION_SAVE);
        Ok(publish(&sink, s, now))
    }

    pub fn play(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let opened = self.play_request(id)?.map(OutputRequest::open).transpose()?;
        self.play_with(id, opened, now)
    }

    pub fn pause(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.transport.pause();
        s.send(Cmd::Pause);
        s.next_pos_save = None;
        s.mark_dirty(now);
        Ok(publish(&sink, s, now))
    }

    /// Moves to `position_ms` (clamped); while playing the sound resumes 0.3 s later.
    pub fn seek(&mut self, id: &str, position_ms: u64, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let f = s.frames_of(position_ms);
        s.set_pos(f);
        s.transport.seek(now);
        s.send(Cmd::Seek(f));
        s.mark_dirty(now);
        eprintln!("calliope: player seek position_ms={}", s.ms_of(f));
        Ok(publish(&sink, s, now))
    }

    pub fn nudge(&mut self, id: &str, delta_ms: i64, now: Instant) -> Result<PlayerTransport, String> {
        let s = self.session(id)?;
        let cur = s.ms_of(s.pos()) as i64;
        self.seek(id, (cur + delta_ms).max(0) as u64, now)
    }

    /// Rew/FF: an instant jump (no scrub silence), clamped.
    pub fn skip(&mut self, id: &str, delta_ms: i64, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let cur = s.ms_of(s.pos()) as i64;
        let f = s.frames_of((cur + delta_ms).max(0) as u64);
        s.set_pos(f);
        s.send(Cmd::Jump(f));
        s.mark_dirty(now);
        Ok(publish(&sink, s, now))
    }

    /// Leaving the view: pause, close the output, flush.
    pub fn leave(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.transport.pause();
        s.send(Cmd::Pause);
        s.next_pos_save = None;
        s.mark_dirty(now);
        flush_session(&sink, s);
        let out = s.output.take().map(|o| o.handle);
        let st = publish(&sink, s, now);
        self.retire(out);
        Ok(st)
    }

    // -------------------------------------------------------- source change

    /// Step 1 (under the lock): stops playback, keeps the position, closes the output.
    /// `None` when `key` is already the source.
    pub fn begin_select(&mut self, id: &str, key: &str, now: Instant) -> Result<Option<SelectJob>, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let entry = s.sources.iter().find(|x| x.key == key).cloned().ok_or_else(|| format!("unknown source {key}"))?;
        if s.state.source.as_deref() == Some(key) && s.switching.is_none() {
            return Ok(None);
        }
        if let Some(c) = &s.select_cancel {
            c.store(true, Ordering::Relaxed);
        }
        s.transport.stop();
        s.send(Cmd::Pause);
        s.next_pos_save = None;
        let out = s.output.take().map(|o| o.handle);
        s.select_gen += 1;
        s.switching = Some(s.select_gen);
        let cancel = Arc::new(AtomicBool::new(false));
        s.select_cancel = Some(cancel.clone());
        s.mark_dirty(now);
        let job = SelectJob {
            id: s.id.clone(),
            key: key.to_string(),
            token: s.select_gen,
            dir: s.dir.clone(),
            file: entry.file,
            ffmpeg: s.ffmpeg.clone(),
            cancel,
        };
        publish(&sink, s, now);
        self.retire(out);
        Ok(Some(job))
    }

    pub fn abort_select(&mut self, job: &SelectJob) {
        if let Some(s) = self.session.as_mut().filter(|s| s.id == job.id && s.switching == Some(job.token)) {
            s.switching = None;
        }
    }

    /// Step 3 (under the lock): installs the decoded source at the same position in time.
    pub fn finish_select(&mut self, job: &SelectJob, loaded: LoadedSource, now: Instant) -> Result<PlayerSnapshot, String> {
        let s = match self.session.as_mut() {
            Some(s) if s.id == job.id && s.switching == Some(job.token) && !job.cancel.load(Ordering::Relaxed) => s,
            _ => return Err(SUPERSEDED.into()),
        };
        let ms = s.ms_of(s.pos());
        s.pcm = Arc::new(loaded.pcm);
        s.peaks = loaded.peaks;
        let mut f = s.frames_of(ms);
        if f >= s.pcm.frames {
            f = 0;
        }
        s.shared = Arc::new(Shared::default());
        s.set_pos(f);
        s.state.source = Some(job.key.clone());
        s.switching = None;
        s.select_cancel = None;
        s.mark_dirty(now);
        eprintln!(
            "calliope: player open id={} source={} duration_ms={} rate={} position_ms={}",
            s.id, job.key, s.ms_of(s.pcm.frames), s.pcm.rate, s.ms_of(f)
        );
        Ok(s.snapshot())
    }

    /// Whole source change in one call; the IPC layer splits it to keep the manager unlocked.
    pub fn select_source(&mut self, id: &str, key: &str, now: Instant) -> Result<PlayerSnapshot, String> {
        let Some(job) = self.begin_select(id, key, now)? else { return self.snapshot(id) };
        let sink = self.sink.clone();
        match load_source(&job, &sink) {
            Ok(l) => self.finish_select(&job, l, now),
            Err(e) => {
                self.abort_select(&job);
                Err(e)
            }
        }
    }

    // ---------------------------------------------------------------- ticker

    /// Called every ~20 ms by the ticker thread.
    pub fn tick(&mut self, now: Instant) {
        let sink = self.sink.clone();
        let mut retired = None;
        if let Some(s) = self.session.as_mut() {
            let failed = s.output.as_ref().and_then(|o| o.handle.error());
            let ended = s.shared.ended.swap(false, Ordering::AcqRel);
            if failed.is_some() || ended {
                s.transport.ended();
                s.next_pos_save = None;
                s.send(Cmd::Pause);
                if ended {
                    s.set_pos(0);
                    s.send(Cmd::Seek(0));
                    save_session(&sink, s);
                } else {
                    s.mark_dirty(now);
                }
                if let Some(message) = failed {
                    retired = s.output.take().map(|o| o.handle);
                    sink(PlayerEvent::AudioError { id: s.id.clone(), message });
                }
            } else if s.transport.tick(now) {
                s.send(Cmd::Resume);
            }
            if s.transport.shows_playing() && s.next_pos_save.is_some_and(|t| now >= t) {
                s.next_pos_save = Some(now + POSITION_SAVE);
                save_session(&sink, s);
            }
            if s.dirty_at.is_some_and(|t| now.saturating_duration_since(t) >= SAVE_DEBOUNCE) {
                save_session(&sink, s);
            }
            publish(&sink, s, now);
        }
        self.retire(retired);
    }
}

/// Decodes the source of a `SelectJob` (long; call without the manager lock).
pub(crate) fn load_source(job: &SelectJob, sink: &EventSink) -> Result<LoadedSource, String> {
    let mut progress = progress_sink(&job.id, sink);
    let pcm = player_source::decode(&job.dir.join(&job.file), job.ffmpeg.as_deref(), &job.cancel, &mut progress)?;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    let peaks = player_source::peaks(&pcm, PEAK_BUCKETS);
    Ok(LoadedSource { pcm, peaks })
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
