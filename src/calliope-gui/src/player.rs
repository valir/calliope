//! The Player (plan 2.7): the real-time `Engine` owned by the audio callback and the
//! `PlayerManager` that opens a track, drives the engine through a bounded command channel,
//! and persists `player.json` (debounced, plus a periodic position save while playing).
//! The engine also mixes the metronome and count-in clicks and repeats loops (plans 2.4, 2.6).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::audio_out::{OutputBackend, OutputHandle};
use crate::metronome::{count_in_plan, section_clicks, Click, CountIn};
use crate::player_source::{self, Pcm, Source, PEAK_BUCKETS};
use crate::player_state::{self, LoopRec, Mark, PlayerState};
use crate::tempo_map::{Boundary, Edge, Scale, Segment, TempoMap};
use crate::transport::Transport;

pub const SUPERSEDED: &str = "superseded";
const OUTPUT_LOST: &str = "Playback was interrupted; try again";
/// A change is written this long after the last change.
pub(crate) const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// While playing, the position is saved this often.
pub(crate) const POSITION_SAVE: Duration = Duration::from_secs(10);
/// Minimum time between `transport` events while only the position moves.
pub(crate) const EVENT_INTERVAL: Duration = Duration::from_millis(100);
const FADE_MS: u32 = 3;
const COMMANDS: usize = 64;
const TRASH: usize = 8;
pub(crate) const NO_TEMPO: &str = "Enter a BPM first";
pub(crate) const NO_SPEED: &str = "Speed change is not available yet";
pub(crate) const LOOP_INCOMPLETE: &str = "Set the loop end first";

// ---------------------------------------------------------------------- engine

/// A beat of the grid: (source frame, first beat of a bar). Every beat clicks in the same tone.
pub(crate) type Beat = (u64, bool);

/// The current complete loop, in source frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LoopSpec {
    pub start: u64,
    pub end: u64,
    /// Loop On: playback reaching the end jumps back to the start.
    pub repeat: bool,
    /// Count-in before every repeat (the loop's own Count-In).
    pub count_in: Option<CountIn>,
}

/// Commands from the manager to the callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cmd {
    /// Start sound at the engine position (restarts from 0 at the end of the track), after
    /// the count-in when given.
    Play { count_in: Option<CountIn> },
    /// Jump to the loop start and play it; with Loop On it repeats, otherwise it stops at the end.
    PlayLoop { count_in: Option<CountIn> },
    Pause,
    /// Move; while playing the sound stays off until `Resume` (the scrub rule).
    Seek(u64),
    /// Move at once, sound continues (with a fade-in).
    Jump(u64),
    /// End of the scrub silence.
    Resume,
    /// The current complete loop (none for no loop or an incomplete one).
    SetLoop(Option<LoopSpec>),
    SetGrid(Arc<Vec<Beat>>),
    SetMetronome(bool),
}

/// Status written by the callback, read by the manager (atomics only).
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// Position in frames. The manager also stores here when no output is open.
    pub position: AtomicU64,
    /// The source played to its end.
    pub ended: AtomicBool,
    /// A count-in is playing. The manager sets it when it sends a count-in; the callback keeps it true.
    pub counting_in: AtomicBool,
    /// A loop is being played with the loop Play (same ownership rule).
    pub loop_playing: AtomicBool,
    /// A loop played once reached its end and stopped at its start.
    pub loop_stopped: AtomicBool,
    /// Loop repeats so far.
    pub wraps: AtomicU64,
}

struct CountState {
    plan: CountIn,
    at: u64,
}

/// A click being mixed in.
#[derive(Clone, Copy)]
struct Voice {
    at: usize,
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
    click: Click,
    grid: Arc<Vec<Beat>>,
    beat_idx: usize,
    beat_dirty: bool,
    metronome: bool,
    voice: Option<Voice>,
    count: Option<CountState>,
    lp: Option<LoopSpec>,
    loop_play: bool,
    trash: Option<SyncSender<Arc<Vec<Beat>>>>,
}

impl Engine {
    pub fn new(pcm: Arc<Pcm>, pos: u64, shared: Arc<Shared>) -> Self {
        let fade_len = (pcm.rate * FADE_MS / 1000).max(1);
        let pos = pos.min(pcm.frames);
        let click = Click::new(pcm.rate);
        Engine {
            pcm,
            pos,
            playing: false,
            audible: false,
            fade_len,
            fade_left: 0,
            shared,
            click,
            grid: Arc::new(Vec::new()),
            beat_idx: 0,
            beat_dirty: true,
            metronome: false,
            voice: None,
            count: None,
            lp: None,
            loop_play: false,
            trash: None,
        }
    }

    /// Replaced grids go back through this channel, so they are freed outside the callback.
    pub fn with_trash(mut self, trash: SyncSender<Arc<Vec<Beat>>>) -> Self {
        self.trash = Some(trash);
        self
    }

    fn start_fade(&mut self) {
        self.fade_left = self.fade_len;
    }

    fn move_to(&mut self, f: u64) {
        self.pos = f.min(self.pcm.frames);
        self.beat_dirty = true;
        self.voice = None;
    }

    fn begin(&mut self, count_in: Option<CountIn>) {
        self.playing = true;
        self.audible = true;
        self.voice = None;
        self.beat_dirty = true;
        self.count = count_in.map(|plan| CountState { plan, at: 0 });
        if self.count.is_none() {
            self.start_fade();
        }
    }

    pub fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { count_in } => {
                if self.pos >= self.pcm.frames {
                    self.pos = 0;
                }
                self.begin(count_in);
            }
            Cmd::PlayLoop { count_in } => {
                if let Some(l) = self.lp {
                    self.pos = l.start.min(self.pcm.frames);
                    self.loop_play = true;
                } else if self.pos >= self.pcm.frames {
                    self.pos = 0;
                }
                self.begin(count_in);
            }
            Cmd::Pause => {
                self.playing = false;
                self.audible = false;
                self.count = None;
                self.loop_play = false;
                self.voice = None;
            }
            Cmd::Seek(f) => {
                self.move_to(f);
                self.count = None;
                if self.playing {
                    self.audible = false;
                }
            }
            Cmd::Jump(f) => {
                self.move_to(f);
                self.count = None;
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
            Cmd::SetLoop(l) => {
                self.lp = l;
                if l.is_none() {
                    self.loop_play = false;
                }
            }
            Cmd::SetGrid(g) => {
                let old = std::mem::replace(&mut self.grid, g);
                self.beat_dirty = true;
                if let Some(t) = &self.trash {
                    let _ = t.try_send(old); // when full, freed here (rare)
                }
            }
            Cmd::SetMetronome(on) => {
                self.metronome = on;
                self.beat_dirty = true;
            }
        }
    }

    /// Adds the running click voice to frame `i` of `out`.
    fn mix_voice(&mut self, out: &mut [f32], i: usize) {
        if let Some(v) = self.voice {
            let s = self.click.samples();
            if let Some(&x) = s.get(v.at) {
                out[2 * i] += x;
                out[2 * i + 1] += x;
                self.voice = Some(Voice { at: v.at + 1 });
            } else {
                self.voice = None;
            }
        }
    }

    /// Fills an interleaved stereo block.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let ch = self.pcm.channels.max(1) as usize;
        if self.pcm.frames == 0 || self.pcm.samples.len() < ch {
            return; // nothing to play
        }
        let mut i = 0;
        while i < frames && self.playing {
            if let Some(c) = self.count.as_mut() {
                if c.at >= c.plan.start {
                    self.count = None;
                    self.start_fade();
                    self.voice = None;
                    continue;
                }
                if c.plan.clicks.contains(&c.at) {
                    self.voice = Some(Voice { at: 0 });
                }
                c.at += 1;
                self.mix_voice(out, i);
                i += 1;
                continue;
            }
            if !self.audible {
                break;
            }
            if self.metronome {
                if self.beat_dirty {
                    self.beat_idx = self.grid.partition_point(|b| b.0 < self.pos);
                    self.beat_dirty = false;
                }
                while let Some(&(f, _)) = self.grid.get(self.beat_idx) {
                    if f > self.pos {
                        break;
                    }
                    if f == self.pos {
                        self.voice = Some(Voice { at: 0 });
                    }
                    self.beat_idx += 1;
                }
            }
            let base = self.pos as usize * ch;
            let l = f32::from(self.pcm.samples[base]) / 32768.0;
            let r = if ch >= 2 { f32::from(self.pcm.samples[base + 1]) / 32768.0 } else { l };
            let mut g = if self.fade_left > 0 {
                let g = (self.fade_len - self.fade_left) as f32 / self.fade_len as f32;
                self.fade_left -= 1;
                g
            } else {
                1.0
            };
            let wraps = self.lp.filter(|l| l.repeat || self.loop_play);
            if let Some(l) = wraps {
                if self.pos < l.end && self.pos >= l.start && l.end - self.pos <= u64::from(self.fade_len) {
                    g *= (l.end - self.pos) as f32 / self.fade_len as f32;
                }
            }
            out[2 * i] = l * g;
            out[2 * i + 1] = r * g;
            self.mix_voice(out, i);
            out[2 * i] = out[2 * i].clamp(-1.0, 1.0);
            out[2 * i + 1] = out[2 * i + 1].clamp(-1.0, 1.0);
            i += 1;
            self.pos += 1;
            if let Some(l) = wraps.filter(|l| self.pos == l.end) {
                if l.repeat {
                    self.pos = l.start;
                    self.beat_dirty = true;
                    self.shared.wraps.fetch_add(1, Ordering::AcqRel);
                    self.count = l.count_in.map(|plan| CountState { plan, at: 0 });
                    if self.count.is_none() {
                        self.start_fade();
                    }
                } else {
                    self.pos = l.start; // played once: back to the loop start
                    self.playing = false;
                    self.audible = false;
                    self.loop_play = false;
                    self.voice = None;
                    self.shared.loop_stopped.store(true, Ordering::Release);
                }
                continue;
            }
            if self.pos >= self.pcm.frames {
                self.playing = false;
                self.audible = false;
                self.loop_play = false;
                self.shared.ended.store(true, Ordering::Release);
            }
        }
        if !self.playing {
            self.count = None;
        }
        self.shared.counting_in.store(self.count.is_some(), Ordering::Release);
        self.shared.loop_playing.store(self.playing && self.loop_play, Ordering::Release);
        self.shared.position.store(self.pos, Ordering::Release);
    }
}

// -------------------------------------------------------------------- IPC types

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerTransport {
    pub playing: bool,
    pub counting_in: bool,
    pub resume_pending: bool,
    pub position_ms: u64,
    pub loop_playing: bool,
    pub bpm: f64,
    pub tempo_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceEntry {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerLoop {
    pub id: String,
    pub start: Boundary,
    pub end: Option<Boundary>,
    pub start_ms: f64,
    pub end_ms: Option<f64>,
    pub repeat: bool,
    pub count_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SnapTempo {
    pub beats_per_bar: u32,
    pub segments: Vec<Segment>,
    pub multi: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlayerSnapshot {
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
    pub loops: Vec<PlayerLoop>,
    pub current_loop: Option<usize>,
    pub loop_message: Option<String>,
    pub transport: PlayerTransport,
    pub can_change_speed: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PlayerEvent {
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
    /// The session was released (the Editor wrote a new tempo map): the UI forgets it and
    /// reopens the track when it next shows it.
    Closed {
        id: String,
    },
}

pub type EventSink = Arc<dyn Fn(PlayerEvent) + Send + Sync>;

// ------------------------------------------------------------------ open inputs

/// What the IPC layer extracts from the track record.
#[derive(Debug, Clone)]
pub struct OpenSpec {
    pub id: String,
    pub title: String,
    pub dir: PathBuf,
    /// `player_source::sources(record)`; files are relative to `dir`.
    pub sources: Vec<Source>,
    pub ffmpeg: Option<PathBuf>,
}

/// Handed out by `begin_open`; a later `begin_open` or `close` cancels it.
#[derive(Clone)]
pub struct OpenTicket {
    gen: u64,
    cancel: Arc<AtomicBool>,
}

/// A decoded track; produced without holding the manager.
pub struct Loaded {
    state: PlayerState,
    note: Option<String>,
    source: String,
    pcm: Pcm,
    peaks: Vec<f32>,
    /// FNV-1a revision of the `player.json` bytes `state` was read from (none: no file).
    revision: Option<String>,
}

/// A source change in progress (`begin_select`).
#[derive(Clone)]
pub struct SelectJob {
    id: String,
    key: String,
    token: u64,
    dir: PathBuf,
    file: String,
    ffmpeg: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
}

pub struct LoadedSource {
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

/// Revision of `<dir>/player.json`: the FNV-1a hash of its bytes; none when it can't be read.
fn revision_of(dir: &std::path::Path) -> Option<String> {
    std::fs::read(dir.join(player_state::PLAYER_FILE)).ok().map(|b| crate::fsutil::fnv1a64_hex(&b))
}

/// Reads `player.json`, picks the source and decodes it (long; call without the manager lock).
pub(crate) fn load(spec: &OpenSpec, ticket: &OpenTicket, sink: &EventSink) -> Result<Loaded, String> {
    // The bytes are read first, so the state is at least as new as the revision.
    let revision = revision_of(&spec.dir);
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
    Ok(Loaded { state, note, source, pcm, peaks, revision })
}

// ---------------------------------------------------------------------- manager

struct Output {
    handle: Box<dyn OutputHandle>,
    tx: SyncSender<Cmd>,
    /// Grids the callback has replaced; dropped here, on the ticker thread.
    trash: Receiver<Arc<Vec<Beat>>>,
}

/// What it takes to open the device; `open` blocks and must run without the manager lock.
pub struct OutputRequest {
    backend: Arc<dyn OutputBackend>,
    shared: Arc<Shared>,
    pcm: Arc<Pcm>,
    start: u64,
}

pub struct OpenedOutput {
    shared: Arc<Shared>,
    output: Output,
}

impl OutputRequest {
    pub fn open(self) -> Result<OpenedOutput, String> {
        let (tx, rx): (SyncSender<Cmd>, Receiver<Cmd>) = sync_channel(COMMANDS);
        let (trash_tx, trash) = sync_channel(TRASH);
        let mut engine = Engine::new(self.pcm.clone(), self.start, self.shared.clone()).with_trash(trash_tx);
        let handle = self.backend.open(
            self.pcm.rate,
            Box::new(move |out| {
                while let Ok(c) = rx.try_recv() {
                    engine.apply(c);
                }
                engine.render(out);
            }),
        )?;
        Ok(OpenedOutput { shared: self.shared, output: Output { handle, tx, trash } })
    }
}

#[derive(PartialEq, Clone, Copy)]
struct EventKey {
    playing: bool,
    resume_pending: bool,
    counting_in: bool,
    loop_playing: bool,
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
    /// Refusal of the last `<`/`>`, shown by the Loops section.
    loop_message: Option<String>,
    last_wraps: u64,
    /// The loop (start, end) in ms the click grid was last computed for (10.6).
    grid_wrap: Option<(f64, f64)>,
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
        (ms.saturating_mul(u64::from(self.pcm.rate)) / 1000).min(self.pcm.frames)
    }

    fn send(&self, cmd: Cmd) {
        if let Some(o) = &self.output {
            while o.trash.try_recv().is_ok() {}
            if o.tx.try_send(cmd).is_err() {
                eprintln!("calliope: player command dropped (queue full)");
            }
        }
    }

    fn pos_ms_f(&self) -> f64 {
        self.pos() as f64 * 1000.0 / f64::from(self.pcm.rate.max(1))
    }

    fn frame_of_ms_f(&self, ms: f64) -> u64 {
        ((ms * f64::from(self.pcm.rate) / 1000.0).round().max(0.0) as u64).min(self.pcm.frames)
    }

    /// The first frame at or after `ms` (a tolerance absorbs float noise on whole-frame times).
    fn frame_at_or_after_ms(&self, ms: f64) -> u64 {
        (((ms * f64::from(self.pcm.rate) / 1000.0) - 1e-6).ceil().max(0.0) as u64).min(self.pcm.frames)
    }

    fn boundary_ms(&self, b: Boundary) -> f64 {
        match b {
            Boundary::Ms { ms } => ms,
            Boundary::Tick { tick } => self.state.tempo.ms_at(tick),
        }
    }

    /// The count-in for a start at `ms`; none without a tempo.
    fn plan_at(&self, ms: f64) -> Option<CountIn> {
        let bpm = self.state.tempo.bpm_at(ms) * self.state.speed;
        (!self.state.tempo.is_empty() && bpm > 0.0).then(|| count_in_plan(bpm, self.pcm.rate))
    }

    /// Logs a count-in; Play adds the index of the section that counted in.
    fn log_count_in(&self, plan: &Option<CountIn>, section: Option<usize>) {
        if let Some(p) = plan {
            let section = section.map_or_else(String::new, |i| format!(" section={i}"));
            eprintln!("calliope: player count-in clicks=3 beat_ms={}{section}", self.ms_of(p.clicks[1]));
        }
    }

    fn grid(&self) -> Arc<Vec<Beat>> {
        let rate = f64::from(self.pcm.rate);
        let dur_ms = self.pcm.frames as f64 * 1000.0 / rate;
        let clicks = section_clicks(&self.state.tempo, dur_ms, self.wrap_ms());
        let mut frames: Vec<u64> = clicks.into_iter().map(|ms| (ms * rate / 1000.0).round() as u64).collect();
        frames.dedup();
        Arc::new(frames.into_iter().map(|f| (f, false)).collect())
    }

    /// The current complete loop (start, end) in ms when it repeats; the click list skips a count-in it ends in.
    fn wrap_ms(&self) -> Option<(f64, f64)> {
        let l = self.state.current()?;
        if !l.repeat {
            return None;
        }
        let (start, end) = (self.boundary_ms(l.start), self.boundary_ms(l.end?));
        (end > start).then_some((start, end))
    }

    /// The current loop as the engine plays it; none when absent or incomplete.
    fn loop_spec(&self) -> Option<LoopSpec> {
        let l = self.state.current()?;
        let end = self.frame_of_ms_f(self.boundary_ms(l.end?));
        let start = self.frame_of_ms_f(self.boundary_ms(l.start));
        if end <= start {
            return None;
        }
        let count_in = if l.count_in { self.plan_at(self.boundary_ms(l.start)) } else { None };
        Some(LoopSpec { start, end, repeat: l.repeat, count_in })
    }

    fn sync_loop(&mut self) {
        self.send(Cmd::SetLoop(self.loop_spec()));
        if self.wrap_ms() != self.grid_wrap {
            self.sync_grid();
        }
    }

    fn sync_grid(&mut self) {
        self.grid_wrap = self.wrap_ms();
        self.send(Cmd::SetGrid(self.grid()));
        self.send(Cmd::SetMetronome(!self.state.tempo.is_empty()));
    }

    /// Pauses the engine and clears the count-in and loop-play marks at once.
    fn halt(&self) {
        self.send(Cmd::Pause);
        self.shared.counting_in.store(false, Ordering::Release);
        self.shared.loop_playing.store(false, Ordering::Release);
    }

    fn loop_index(&self) -> Option<usize> {
        let id = self.state.current_loop.as_ref()?;
        self.state.loops.iter().position(|l| &l.id == id)
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
            counting_in: self.shared.counting_in.load(Ordering::Acquire),
            loop_playing: self.shared.loop_playing.load(Ordering::Acquire),
        }
    }

    fn transport_state(&self) -> PlayerTransport {
        let ms = self.ms_of(self.pos());
        // The section lookup uses the exact frame time: the whole-ms `position_ms` floors below a fractional
        // section start the Start jump just landed on (a tolerance absorbs float noise on whole-frame times).
        let exact = self.pos_ms_f() + 1e-6;
        PlayerTransport {
            playing: self.transport.shows_playing(),
            counting_in: self.shared.counting_in.load(Ordering::Acquire),
            resume_pending: self.transport.resume_pending(),
            position_ms: ms,
            loop_playing: self.shared.loop_playing.load(Ordering::Acquire),
            bpm: self.state.tempo.bpm_at(exact) * self.state.speed,
            tempo_index: self.state.tempo.segment_at(exact),
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
            loops,
            current_loop,
            loop_message: self.loop_message.clone(),
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

/// Like `flush_session`, but reports a failed write instead of only emitting an event.
fn try_flush(s: &mut Session) -> Result<(), String> {
    if s.state.read_only || (s.dirty_at.is_none() && s.ms_of(s.pos()) == s.state.position_ms) {
        return Ok(());
    }
    let position = s.ms_of(s.pos());
    let mut state = s.state.clone();
    state.position_ms = position;
    player_state::save(&s.dir, &state).map_err(|e| {
        eprintln!("calliope: player state error id={} message={e}", s.id);
        format!("Could not save the Player's settings first: {e}")
    })?;
    s.state.position_ms = position;
    s.dirty_at = None;
    eprintln!("calliope: player state saved id={}", s.id);
    Ok(())
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
                "calliope: player transport playing={} counting_in={} loop={} position_ms={}",
                state.playing,
                state.counting_in,
                if state.loop_playing { s.loop_index().map_or("none".to_string(), |n| n.to_string()) } else { "none".into() },
                state.position_ms
            );
        }
        s.last_key = key;
        s.last_position = state.position_ms;
        s.last_emit = Some(now);
        sink(PlayerEvent::Transport { id: s.id.clone(), state: state.clone() });
    }
    state
}

/// What the Loops section asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopAction {
    Add,
    Remove,
    /// Previous (-1) or next (+1); jumps to the start of the loop at once.
    Select(i32),
    /// `>` (Start) and `<` (End): the loop edge at the position.
    Mark(Mark),
    /// Typed bar1/bar2 value.
    Set(Mark, Boundary),
    Flags {
        repeat: bool,
        count_in: bool,
    },
    Play,
    Stop,
}

/// One edit of the tempo sections (the Player's Sections group and waveform).
#[derive(Debug, Clone, PartialEq)]
pub enum SectionEdit {
    Bpm { index: usize, at: f64, bpm: f64 },
    Scale { index: usize, at: f64, factor: Scale },
    Add { position: f64, bpm: f64 },
    Remove { index: usize, at: f64 },
    Move { index: usize, at: f64, steps: i64 },
    Flags { index: usize, at: f64, metronome: bool, count_in: bool },
    Name { index: usize, at: f64, name: String },
}

pub struct PlayerManager {
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
        let Loaded { mut state, mut note, source, pcm, peaks, revision } = loaded;
        if revision_of(&spec.dir) != revision {
            // `player.json` was written (the Editor's tempo map) after it was read: use the new
            // content, so that write is not lost. The decoded source stays.
            let (fresh, fresh_note, _) = player_state::load(&spec.dir);
            eprintln!("calliope: player reread id={} (player.json changed while opening)", spec.id);
            state = fresh;
            note = fresh_note;
        }
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
            last_key: EventKey { playing: false, resume_pending: false, counting_in: false, loop_playing: false },
            last_position: 0,
            last_emit: None,
            loop_message: None,
            last_wraps: 0,
            grid_wrap: None,
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
    #[allow(dead_code)] // whole-call form, used by the tests
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
            s.halt();
            flush_session(&self.sink, &mut s);
            self.retire(s.output.take().map(|o| o.handle));
        }
    }

    /// Saves the pending state (loops, position, source, metronome) of the session for `id`, if
    /// there is one; it stays open. Fails when the write fails.
    pub fn flush_if_open(&mut self, id: &str) -> Result<(), String> {
        match self.session.as_mut() {
            Some(s) if s.id == id => try_flush(s),
            _ => Ok(()),
        }
    }

    /// Hands the track over to a writer of `player.json` (plan 2.2): flushes the session for
    /// `id`, stops it, closes the output, drops it and emits `Closed`. When the flush fails
    /// nothing changes. Returns whether there was a session.
    pub fn release(&mut self, id: &str) -> Result<bool, String> {
        match self.session.as_mut() {
            Some(s) if s.id == id => {
                // a newer-schema file can't be written: refuse before interrupting the Player
                if !player_state::load(&s.dir).2 {
                    return Err(crate::tempo_store::NEWER.into());
                }
                try_flush(s)?
            }
            _ => return Ok(false),
        }
        let mut s = self.session.take().expect("checked above");
        if let Some(c) = &s.select_cancel {
            c.store(true, Ordering::Relaxed);
        }
        s.halt();
        self.retire(s.output.take().map(|o| o.handle));
        eprintln!("calliope: player released id={id}");
        (self.sink)(PlayerEvent::Closed { id: id.to_string() });
        Ok(true)
    }

    #[allow(dead_code)] // whole-call form, used by the tests
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

    /// The open session's id and whether it is sounding (no snapshot built).
    pub fn current_id_playing(&self) -> Option<(String, bool)> {
        self.session.as_ref().map(|s| (s.id.clone(), s.transport.shows_playing()))
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
                s.sync_grid();
                s.sync_loop();
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
        // A count-in precedes a start from paused or stopped, not a play that is already running,
        // when the section under the playhead has Count In.
        let pos_ms = s.pos_ms_f();
        let plan = if s.transport.shows_playing() || !s.state.tempo.count_in_at(pos_ms) {
            None
        } else {
            s.plan_at(pos_ms)
        };
        s.log_count_in(&plan, s.state.tempo.segment_at(pos_ms));
        s.shared.counting_in.store(plan.is_some(), Ordering::Release);
        s.transport.play();
        s.send(Cmd::Play { count_in: plan });
        s.next_pos_save = Some(now + POSITION_SAVE);
        Ok(publish(&sink, s, now))
    }

    #[allow(dead_code)] // whole-call form, used by the tests
    pub fn play(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let opened = self.play_request(id)?.map(OutputRequest::open).transpose()?;
        self.play_with(id, opened, now)
    }

    pub fn pause(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.transport.pause();
        s.halt();
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
        self.seek(id, cur.saturating_add(delta_ms).max(0) as u64, now)
    }

    /// Rew/FF: an instant jump (no scrub silence), clamped.
    pub fn skip(&mut self, id: &str, delta_ms: i64, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let cur = s.ms_of(s.pos()) as i64;
        let f = s.frames_of(cur.saturating_add(delta_ms).max(0) as u64);
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
        s.halt();
        s.next_pos_save = None;
        s.mark_dirty(now);
        flush_session(&sink, s);
        let out = s.output.take().map(|o| o.handle);
        let st = publish(&sink, s, now);
        self.retire(out);
        Ok(st)
    }

    // ------------------------------------------------------ tempo, metronome

    /// Types a BPM at the current position (0 clears the segment there); resets the speed to
    /// 1.0, re-snaps the loops and uploads the new grid.
    pub fn set_bpm(&mut self, id: &str, bpm: f64, now: Instant) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        let mut map: TempoMap = s.state.tempo.clone();
        map.set_bpm_at(s.pos_ms_f(), bpm)?;
        s.state.speed = 1.0;
        s.state.apply_tempo_change(map);
        s.loop_message = None;
        s.sync_grid();
        s.sync_loop();
        s.mark_dirty(now);
        eprintln!("calliope: player bpm value={bpm} position_ms={}", s.ms_of(s.pos()));
        Ok(s.snapshot())
    }

    /// Edits the tempo sections of the open session without pausing or moving the playback:
    /// loops keep their time, the engine gets the new grid and the file is saved after the debounce.
    /// A refusal changes nothing.
    pub fn edit_sections(&mut self, id: &str, edit: SectionEdit, now: Instant) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        let end_ms = s.pcm.frames as f64 * 1000.0 / f64::from(s.pcm.rate.max(1));
        let old = &s.state.tempo;
        let guard = |index: usize, at: f64| match old.segments.get(index) {
            // `at` travelled through JSON. serde_json is built with `float_roundtrip`, so it is exact, but the
            // tolerance keeps a stale-guard refusal impossible for a one-ulp difference. 1e-6 ms is far below
            // one fine step (`fine_ms` is about 0.65 ms at 300 BPM) so a different section never matches.
            Some(g) if (g.start_ms - at).abs() < 1e-6 => Ok(()),
            _ => Err("The tempo sections changed; try again".to_string()),
        };
        let (kind, index, map, extra) = match edit {
            SectionEdit::Bpm { index, at, bpm } => {
                guard(index, at)?;
                ("bpm", index, old.set_section_bpm(index, bpm)?, String::new())
            }
            SectionEdit::Scale { index, at, factor } => {
                guard(index, at)?;
                ("scale", index, old.scale_section(index, factor)?, String::new())
            }
            SectionEdit::Add { position, bpm } => {
                let (map, index) = old.add_change(position, bpm, end_ms)?;
                ("add", index, map, String::new())
            }
            SectionEdit::Remove { index, at } => {
                guard(index, at)?;
                ("remove", index, old.remove_change(index)?, String::new())
            }
            SectionEdit::Move { index, at, steps } => {
                guard(index, at)?;
                let (map, applied) = old.move_change(index, steps, end_ms)?;
                ("move", index, map, format!(" steps={steps} applied={applied}"))
            }
            SectionEdit::Flags { index, at, metronome, count_in } => {
                guard(index, at)?;
                ("flags", index, old.set_section_flags(index, metronome, count_in)?, String::new())
            }
            SectionEdit::Name { index, at, name } => {
                guard(index, at)?;
                ("name", index, old.set_section_name(index, &name)?, String::new())
            }
        };
        s.state.apply_tempo_change(map);
        s.loop_message = None;
        s.sync_grid();
        s.sync_loop();
        s.mark_dirty(now);
        let segs = &s.state.tempo.segments;
        let bpms: Vec<String> = segs.iter().map(|g| g.bpm.to_string()).collect();
        let flag = |f: fn(&Segment) -> bool| segs.iter().map(|g| if f(g) { "1" } else { "0" }).collect::<Vec<_>>().join(",");
        eprintln!(
            "calliope: player section {kind} id={id} index={index} segments={} bpm={}{extra} metronome={} count_in={}",
            bpms.len(),
            bpms.join(","),
            flag(|g| g.meta.metronome),
            flag(|g| g.meta.count_in)
        );
        Ok(s.snapshot())
    }

    /// Start / End of a section: an instant jump like loop Prev/Next; playback goes on, a pre-roll is cancelled.
    pub fn jump_section(&mut self, id: &str, index: usize, at: f64, edge: Edge, now: Instant) -> Result<PlayerSnapshot, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        match s.state.tempo.segments.get(index) {
            Some(g) if (g.start_ms - at).abs() < 1e-6 => {}
            _ => return Err("The tempo sections changed; try again".to_string()),
        }
        let end_ms = s.pcm.frames as f64 * 1000.0 / f64::from(s.pcm.rate.max(1));
        let ms = s.state.tempo.section_jump_ms(index, edge, end_ms)?;
        // A Start jump must not land before the section start (the lookup would pick the previous section).
        let f = if edge == Edge::Start { s.frame_at_or_after_ms(ms) } else { s.frame_of_ms_f(ms) };
        s.set_pos(f);
        s.send(Cmd::Jump(f));
        s.mark_dirty(now);
        publish(&sink, s, now);
        eprintln!(
            "calliope: player section jump id={id} index={index} edge={} position_ms={}",
            if edge == Edge::Start { "start" } else { "end" },
            s.ms_of(f)
        );
        Ok(s.snapshot())
    }

    /// The speed buttons: not in this increment.
    pub fn speed_action(&mut self, id: &str) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        Err(if s.state.tempo.is_empty() { NO_TEMPO } else { NO_SPEED }.into())
    }

    // ---------------------------------------------------------------- loops

    fn playable_loop(&self, id: &str) -> Result<(), String> {
        match self.session.as_ref() {
            Some(s) if s.id == id => match s.state.current() {
                None => Err("There is no loop selected".into()),
                Some(l) if !l.is_complete() => Err(LOOP_INCOMPLETE.into()),
                Some(_) => Ok(()),
            },
            _ => Err(format!("no player session for {id}")),
        }
    }

    /// Step 1 of loop Play: refuses an incomplete loop, then as `play_request`.
    pub fn loop_play_request(&self, id: &str) -> Result<Option<OutputRequest>, String> {
        self.playable_loop(id)?;
        self.play_request(id)
    }

    /// Step 2 of loop Play: jumps to the loop start and plays (count-in per 2.6).
    pub fn loop_play_with(&mut self, id: &str, opened: Option<OpenedOutput>, now: Instant) -> Result<PlayerSnapshot, String> {
        self.playable_loop(id)?;
        self.session(id)?;
        self.install(opened);
        let sink = self.sink.clone();
        let s = self.session(id)?;
        if s.output.is_none() {
            return Err(OUTPUT_LOST.into());
        }
        let l = s.state.current().expect("checked");
        let (start_ms, loop_count_in) = (s.boundary_ms(l.start), l.count_in);
        let start = s.frame_of_ms_f(start_ms);
        let plan = if loop_count_in { s.plan_at(start_ms) } else { None };
        s.loop_message = None;
        s.set_pos(start);
        s.log_count_in(&plan, None);
        s.shared.counting_in.store(plan.is_some(), Ordering::Release);
        s.shared.loop_playing.store(true, Ordering::Release);
        s.transport.play();
        s.send(Cmd::PlayLoop { count_in: plan });
        s.next_pos_save = Some(now + POSITION_SAVE);
        publish(&sink, s, now);
        Ok(s.snapshot())
    }

    /// One Loops-section action (all but loop Play, which needs the device: `loop_play_with`).
    pub fn loop_action(&mut self, id: &str, action: LoopAction, now: Instant) -> Result<PlayerSnapshot, String> {
        if action == LoopAction::Play {
            let opened = self.loop_play_request(id)?.map(OutputRequest::open).transpose()?;
            return self.loop_play_with(id, opened, now);
        }
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.loop_message = None;
        let pos_ms = s.pos_ms_f();
        match action {
            LoopAction::Add => {
                let track_end_ms = s.ms_of(s.pcm.frames);
                s.state.add(pos_ms, track_end_ms as f64)?;
            }
            LoopAction::Remove => s.state.remove()?,
            LoopAction::Select(delta) => {
                let n = s.state.loops.len() as i64;
                if n == 0 {
                    return Err("There is no loop selected".into());
                }
                // With none current, Next picks the first loop and Prev the last.
                let cur = s.loop_index().map_or(if delta >= 0 { -1 } else { n }, |i| i as i64);
                let to = (cur + i64::from(delta)).clamp(0, n - 1) as usize;
                let id = s.state.loops[to].id.clone();
                s.state.select(&id)?;
                let l = &s.state.loops[to];
                let f = s.frame_of_ms_f(s.boundary_ms(l.start));
                s.set_pos(f);
                s.send(Cmd::Jump(f)); // at once, no scrub silence
            }
            LoopAction::Mark(edge) => match s.state.mark(edge, pos_ms) {
                Ok(()) => {}
                Err(m) if m == player_state::END_BEFORE_START => s.loop_message = Some(m),
                Err(m) => return Err(m),
            },
            LoopAction::Set(edge, value) => {
                let ms = s.boundary_ms(value);
                let dur = s.ms_of(s.pcm.frames) as f64;
                let tick_ok = !matches!(value, Boundary::Tick { tick } if tick < 0);
                if !tick_ok || !ms.is_finite() || ms < 0.0 || ms > dur {
                    return Err("The loop boundary must be within the track".into());
                }
                s.state.set(edge, value)?
            }
            LoopAction::Flags { repeat, count_in } => s.state.flags(Some(repeat), Some(count_in))?,
            LoopAction::Stop => {
                let l = s.state.current().ok_or("There is no loop selected")?;
                let f = s.frame_of_ms_f(s.boundary_ms(l.start));
                s.transport.stop();
                s.halt();
                s.set_pos(f);
                s.send(Cmd::Seek(f));
                s.next_pos_save = None;
            }
            LoopAction::Play => unreachable!("handled above"),
        }
        s.sync_loop();
        s.mark_dirty(now);
        publish(&sink, s, now);
        Ok(s.snapshot())
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
        s.halt();
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
    #[allow(dead_code)] // whole-call form, used by the tests
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
            let loop_stopped = s.shared.loop_stopped.swap(false, Ordering::AcqRel);
            let wraps = s.shared.wraps.load(Ordering::Acquire);
            if wraps != s.last_wraps {
                s.last_wraps = wraps;
                eprintln!(
                    "calliope: player wrap loop={} count_in={}",
                    s.loop_index().map_or("none".to_string(), |n| n.to_string()),
                    s.state.current().is_some_and(|l| l.count_in)
                );
            }
            if let Some(o) = &s.output {
                while o.trash.try_recv().is_ok() {}
            }
            if loop_stopped && failed.is_none() && !ended {
                s.transport.stop(); // the engine is already back at the loop start
                s.next_pos_save = None;
                s.mark_dirty(now);
            } else if failed.is_some() || ended {
                s.transport.ended();
                s.next_pos_save = None;
                s.halt();
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
pub fn load_source(job: &SelectJob, sink: &EventSink) -> Result<LoadedSource, String> {
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
