//! Editor sessions and the playback engine (plan 2.4, 2.7, 2.8): stems decoded into memory,
//! a render callback that mixes them, solo, the transport rules and the events for the UI.
//! No Tauri types: events go to a sink, time is passed in, the output is a backend trait.

#![allow(dead_code)] // used by the IPC layer (task 10) and the tests

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::audio_out::{OutputBackend, OutputHandle};
use crate::mixer;
use crate::stem_audio::{self, Common, StemPcm};
use crate::track_meta::StemEntry;
use crate::transport::Transport;

/// Frames mixed per step in the render callback.
const CHUNK: usize = 512;
/// The render callback uses a fixed array of inputs, so it never allocates.
pub const MAX_STEMS: usize = 16;
/// Minimum time between `transport` events while only the position moves.
pub const EVENT_INTERVAL: Duration = Duration::from_millis(100);
/// `clipping` stays true this long after the last clipped sample.
pub const CLIP_HOLD: Duration = Duration::from_secs(1);
pub const SUPERSEDED: &str = "superseded";

// ------------------------------------------------------------------ IPC types

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LaneState {
    pub name: String,
    /// `None` = Off.
    pub gain_db: Option<f32>,
    pub unmuted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransportState {
    pub playing: bool,
    pub position_ms: u64,
    pub resume_pending: bool,
    pub solo: Option<String>,
    pub clipping: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VariantTarget {
    pub id: String,
    pub name: String,
    pub file: String,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditorSnapshot {
    pub id: String,
    pub title: String,
    pub stems: Vec<LaneState>,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub transport: TransportState,
    pub variant: VariantTarget,
    /// Save progress 0..1 while a save runs (task 9).
    pub saving: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EditorEvent {
    Loading { id: String, done: usize, total: usize },
    Transport {
        id: String,
        #[serde(flatten)]
        state: TransportState,
    },
    AudioError { id: String, message: String },
}

pub type EventSink = Arc<dyn Fn(EditorEvent) + Send + Sync>;

// ---------------------------------------------------------------- open inputs

/// The first backing variant of a track, as far as the editor needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantSpec {
    pub id: String,
    pub name: String,
    pub file: String,
    pub mix: Value,
}

/// What the IPC layer extracts from the track record.
#[derive(Debug, Clone)]
pub struct OpenSpec {
    pub id: String,
    pub title: String,
    pub dir: PathBuf,
    pub stems: Vec<StemEntry>,
    /// The record's `missing` list (paths relative to `dir`).
    pub missing: Vec<String>,
    pub variant: Option<VariantSpec>,
}

/// Handed out by `begin_open`; a later `begin_open` or `close` cancels it.
#[derive(Clone)]
pub struct OpenTicket {
    gen: u64,
    cancel: Arc<AtomicBool>,
}

/// Decoded stems; produced without holding the manager.
pub struct Loaded {
    stems: Vec<StemPcm>,
    common: Common,
}

/// Decodes every stem (long; call it without the manager lock). Emits `loading` events.
pub fn load(spec: &OpenSpec, ticket: &OpenTicket, sink: &dyn Fn(EditorEvent)) -> Result<Loaded, String> {
    let superseded = || ticket.cancel.load(Ordering::Relaxed);
    if spec.stems.is_empty() {
        return Err(format!("{} has no stems.", spec.title));
    }
    if spec.stems.len() > MAX_STEMS {
        return Err(format!("A track can have at most {MAX_STEMS} stems."));
    }
    for s in &spec.stems {
        if spec.missing.contains(&s.file) || !spec.dir.join(&s.file).is_file() {
            return Err(format!("Stem file {} is missing.", s.file));
        }
    }
    let mut infos = Vec::new();
    for s in &spec.stems {
        infos.push((s.name.clone(), stem_audio::probe(&spec.dir.join(&s.file))?));
    }
    let common = stem_audio::check_compatible(&infos)?;
    let total = spec.stems.len();
    let loading = |done| sink(EditorEvent::Loading { id: spec.id.clone(), done, total });
    loading(0);
    let mut stems = Vec::new();
    for (i, s) in spec.stems.iter().enumerate() {
        if superseded() {
            return Err(SUPERSEDED.into());
        }
        let pcm = stem_audio::decode_i16(&spec.dir.join(&s.file), &ticket.cancel, &mut |_, _| {})
            .map_err(|e| if superseded() { SUPERSEDED.to_string() } else { e })?;
        stems.push(pcm);
        loading(i + 1);
    }
    if superseded() {
        return Err(SUPERSEDED.into());
    }
    Ok(Loaded { stems, common })
}

fn parse_mix(mix: &Value, names: &[String]) -> Vec<LaneState> {
    let entries = mix.get("stems").and_then(Value::as_array);
    names
        .iter()
        .map(|name| {
            let entry = entries.and_then(|a| a.iter().find(|e| e.get("name").and_then(Value::as_str) == Some(name)));
            let gain_db = match entry.and_then(|e| e.get("gain_db")) {
                None => Some(0.0),
                Some(Value::Null) => None,
                Some(v) => v.as_f64().map_or(Some(0.0), |f| mixer::clamp_db(f as f32)),
            };
            let unmuted = entry.and_then(|e| e.get("unmuted")).and_then(Value::as_bool).unwrap_or(false);
            LaneState { name: name.clone(), gain_db, unmuted }
        })
        .collect()
}

// --------------------------------------------------------------- shared player

/// State shared with the audio callback.
struct PlayerShared {
    stems: Vec<StemPcm>,
    total_frames: u64,
    /// f32 bits; solo already applied.
    gains: Vec<AtomicU32>,
    position: AtomicU64,
    audible: AtomicBool,
    clipped: AtomicU64,
    ended: AtomicBool,
}

fn render(shared: &PlayerShared, scratch: &mut [Vec<f32>], out: &mut [f32]) {
    let frames = out.len() / 2;
    if !shared.audible.load(Ordering::Relaxed) {
        out.fill(0.0);
        return;
    }
    let start = shared.position.load(Ordering::Acquire);
    let available = shared.total_frames.saturating_sub(start).min(frames as u64) as usize;
    let mut gains = [0.0f32; MAX_STEMS];
    for (g, a) in gains.iter_mut().zip(&shared.gains) {
        *g = f32::from_bits(a.load(Ordering::Relaxed));
    }
    let n_stems = shared.stems.len();
    let mut done = 0;
    let mut clipped = 0u64;
    while done < available {
        let n = CHUNK.min(available - done);
        for k in 0..n_stems {
            if gains[k] != 0.0 {
                mixer::to_stereo_f32(&shared.stems[k], start + done as u64, n, &mut scratch[k]);
            }
        }
        let mut inputs: [&[f32]; MAX_STEMS] = [&[]; MAX_STEMS];
        for k in 0..n_stems {
            inputs[k] = &scratch[k][..n * 2];
        }
        clipped += u64::from(mixer::mix(&inputs[..n_stems], &gains[..n_stems], &mut out[done * 2..(done + n) * 2]));
        done += n;
    }
    out[done * 2..].fill(0.0);
    if clipped > 0 {
        shared.clipped.fetch_add(clipped, Ordering::Relaxed);
    }
    let new = start + done as u64;
    // A concurrent seek (plain store) wins over this advance.
    if shared.position.compare_exchange(start, new, Ordering::AcqRel, Ordering::Acquire).is_ok()
        && new >= shared.total_frames
    {
        shared.ended.store(true, Ordering::Release);
    }
}

// --------------------------------------------------------------------- session

#[derive(PartialEq, Clone)]
struct EventKey {
    playing: bool,
    resume_pending: bool,
    solo: Option<String>,
    clipping: bool,
}

struct Session {
    id: String,
    title: String,
    sample_rate: u32,
    lanes: Vec<LaneState>,
    variant: VariantTarget,
    shared: Arc<PlayerShared>,
    transport: Transport,
    solo: Option<usize>,
    output: Option<Box<dyn OutputHandle>>,
    // clipping
    clip_seen: u64,
    clip_until: Option<Instant>,
    clipping: bool,
    // events
    last_key: EventKey,
    last_position: u64,
    last_emit: Option<Instant>,
}

impl Session {
    fn position_ms(&self) -> u64 {
        frames_to_ms(self.shared.position.load(Ordering::Acquire), self.sample_rate)
    }

    fn state(&self) -> TransportState {
        TransportState {
            playing: self.transport.shows_playing(),
            position_ms: self.position_ms(),
            resume_pending: self.transport.resume_pending(),
            solo: self.solo.map(|i| self.lanes[i].name.clone()),
            clipping: self.clipping,
        }
    }

    fn key(&self) -> EventKey {
        EventKey {
            playing: self.transport.shows_playing(),
            resume_pending: self.transport.resume_pending(),
            solo: self.solo.map(|i| self.lanes[i].name.clone()),
            clipping: self.clipping,
        }
    }

    /// Effective gains (solo applied) and the audible flag, pushed to the callback.
    fn sync_shared(&self) {
        for (i, lane) in self.lanes.iter().enumerate() {
            let g = match self.solo {
                Some(s) if s != i => 0.0,
                _ => mixer::gain_factor(lane.gain_db, lane.unmuted),
            };
            self.shared.gains[i].store(g.to_bits(), Ordering::Relaxed);
        }
        self.shared.audible.store(self.transport.audible(), Ordering::Release);
    }

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            stems: self.lanes.clone(),
            duration_ms: frames_to_ms(self.shared.total_frames, self.sample_rate),
            sample_rate: self.sample_rate,
            transport: self.state(),
            variant: self.variant.clone(),
            saving: None,
        }
    }
}

fn frames_to_ms(frames: u64, rate: u32) -> u64 {
    frames * 1000 / u64::from(rate)
}

fn ms_to_frames(ms: u64, rate: u32) -> u64 {
    ms * u64::from(rate) / 1000
}

// --------------------------------------------------------------------- manager

pub struct EditorManager {
    backend: Box<dyn OutputBackend>,
    sink: EventSink,
    session: Option<Session>,
    remembered: HashMap<String, Vec<LaneState>>,
    pending: Option<OpenTicket>,
    generation: u64,
}

impl EditorManager {
    pub fn new(backend: Box<dyn OutputBackend>, sink: EventSink) -> Self {
        Self { backend, sink, session: None, remembered: HashMap::new(), pending: None, generation: 0 }
    }

    pub fn sink(&self) -> EventSink {
        self.sink.clone()
    }

    /// Closes the current session and cancels any load in progress; returns the ticket for
    /// the new load.
    pub fn begin_open(&mut self) -> OpenTicket {
        self.close();
        self.generation += 1;
        let ticket = OpenTicket { gen: self.generation, cancel: Arc::new(AtomicBool::new(false)) };
        self.pending = Some(ticket.clone());
        ticket
    }

    /// Installs a loaded track unless a newer `begin_open`/`close` superseded the ticket.
    pub fn finish_open(&mut self, ticket: &OpenTicket, spec: &OpenSpec, loaded: Loaded) -> Result<EditorSnapshot, String> {
        if ticket.gen != self.generation || ticket.cancel.load(Ordering::Relaxed) {
            return Err(SUPERSEDED.into());
        }
        self.pending = None;
        let names: Vec<String> = spec.stems.iter().map(|s| s.name.clone()).collect();
        let lanes = match self.remembered.get(&spec.id) {
            Some(r) if r.len() == names.len() && r.iter().zip(&names).all(|(l, n)| &l.name == n) => r.clone(),
            _ => match &spec.variant {
                Some(v) => parse_mix(&v.mix, &names),
                None => names.iter().map(|n| LaneState { name: n.clone(), gain_db: Some(0.0), unmuted: false }).collect(),
            },
        };
        let variant = match &spec.variant {
            Some(v) => VariantTarget {
                id: v.id.clone(),
                name: v.name.clone(),
                file: v.file.clone(),
                exists: spec.dir.join(&v.file).is_file(),
            },
            None => VariantTarget {
                id: "backing".into(),
                name: "Backing".into(),
                file: "backings/backing.flac".into(),
                exists: false,
            },
        };
        let Loaded { stems, common } = loaded;
        let shared = Arc::new(PlayerShared {
            gains: (0..stems.len()).map(|_| AtomicU32::new(0)).collect(),
            stems,
            total_frames: common.frames,
            position: AtomicU64::new(0),
            audible: AtomicBool::new(false),
            clipped: AtomicU64::new(0),
            ended: AtomicBool::new(false),
        });
        let session = Session {
            id: spec.id.clone(),
            title: spec.title.clone(),
            sample_rate: common.sample_rate,
            lanes,
            variant,
            shared,
            transport: Transport::new(),
            solo: None,
            output: None,
            clip_seen: 0,
            clip_until: None,
            clipping: false,
            last_key: EventKey { playing: false, resume_pending: false, solo: None, clipping: false },
            last_position: 0,
            last_emit: None,
        };
        session.sync_shared();
        let snap = session.snapshot();
        eprintln!("calliope: editor open id={} stems={} duration_ms={}", snap.id, snap.stems.len(), snap.duration_ms);
        self.session = Some(session);
        Ok(snap)
    }

    /// Whole open in one call (begin, load, finish); the IPC layer splits it to keep the
    /// manager unlocked during the load.
    pub fn open(&mut self, spec: &OpenSpec) -> Result<EditorSnapshot, String> {
        let ticket = self.begin_open();
        let sink = self.sink.clone();
        let loaded = load(spec, &ticket, &*sink)?;
        self.finish_open(&ticket, spec, loaded)
    }

    /// Stops playback, remembers the mix, cancels a pending load.
    pub fn close(&mut self) {
        if let Some(t) = self.pending.take() {
            t.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(s) = self.session.take() {
            s.shared.audible.store(false, Ordering::Release);
            self.remembered.insert(s.id.clone(), s.lanes.clone());
            drop(s.output);
        }
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.id == id)
    }

    fn session(&mut self, id: &str) -> Result<&mut Session, String> {
        match self.session.as_mut() {
            Some(s) if s.id == id => Ok(s),
            _ => Err(format!("no editor session for {id}")),
        }
    }

    pub fn snapshot(&self, id: &str) -> Result<EditorSnapshot, String> {
        match self.session.as_ref() {
            Some(s) if s.id == id => Ok(s.snapshot()),
            _ => Err(format!("no editor session for {id}")),
        }
    }

    /// The open session's snapshot, if any (re-attach after a webview reload).
    pub fn current(&self) -> Option<EditorSnapshot> {
        self.session.as_ref().map(Session::snapshot)
    }

    fn ensure_output(backend: &dyn OutputBackend, s: &mut Session) -> Result<(), String> {
        if s.output.is_some() {
            return Ok(());
        }
        let shared = s.shared.clone();
        let mut scratch: Vec<Vec<f32>> = (0..shared.stems.len()).map(|_| vec![0.0; CHUNK * 2]).collect();
        let handle = backend.open(s.sample_rate, Box::new(move |out| render(&shared, &mut scratch, out)))?;
        s.output = Some(handle);
        Ok(())
    }

    /// Commits a state change: pushes shared state and emits a transport event if needed.
    fn publish(sink: &EventSink, s: &mut Session, now: Instant) -> TransportState {
        s.sync_shared();
        let state = s.state();
        let key = s.key();
        let pos = s.shared.position.load(Ordering::Acquire);
        let immediate = key != s.last_key || (!state.playing && pos != s.last_position);
        let due = state.playing
            && pos != s.last_position
            && s.last_emit.is_none_or(|t| now.saturating_duration_since(t) >= EVENT_INTERVAL);
        if immediate || due {
            if immediate {
                eprintln!(
                    "calliope: editor transport playing={} audible={} position_ms={} solo={} clipping={}",
                    state.playing,
                    s.transport.audible(),
                    state.position_ms,
                    state.solo.as_deref().unwrap_or("none"),
                    state.clipping
                );
            }
            s.last_key = key;
            s.last_position = pos;
            s.last_emit = Some(now);
            sink(EditorEvent::Transport { id: s.id.clone(), state: state.clone() });
        }
        state
    }

    /// Mix Play: ends solo; needs a checked stem.
    pub fn play(&mut self, id: &str) -> Result<TransportState, String> {
        let (backend, sink) = (&*self.backend, &self.sink);
        let s = match self.session.as_mut() {
            Some(s) if s.id == id => s,
            _ => return Err(format!("no editor session for {id}")),
        };
        if !s.lanes.iter().any(|l| l.unmuted) {
            return Err("Check a stem first".into());
        }
        s.solo = None;
        Self::start(backend, sink, s)
    }

    fn start(backend: &dyn OutputBackend, sink: &EventSink, s: &mut Session) -> Result<TransportState, String> {
        Self::ensure_output(backend, s)?;
        if s.shared.position.load(Ordering::Acquire) >= s.shared.total_frames {
            s.shared.position.store(0, Ordering::Release);
        }
        s.shared.ended.store(false, Ordering::Release);
        s.transport.play();
        Ok(Self::publish(sink, s, Instant::now()))
    }

    /// Lane Play: checks the stem, solos it and plays; on the soloed stem it ends the solo.
    pub fn lane_play(&mut self, id: &str, name: &str) -> Result<TransportState, String> {
        let (backend, sink) = (&*self.backend, &self.sink);
        let s = match self.session.as_mut() {
            Some(s) if s.id == id => s,
            _ => return Err(format!("no editor session for {id}")),
        };
        let idx = s.lanes.iter().position(|l| l.name == name).ok_or_else(|| format!("no stem {name}"))?;
        if s.solo == Some(idx) {
            s.solo = None;
            return Ok(Self::publish(sink, s, Instant::now()));
        }
        s.lanes[idx].unmuted = true;
        s.solo = Some(idx);
        Self::start(backend, sink, s)
    }

    pub fn end_solo(&mut self, id: &str) -> Result<TransportState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.solo = None;
        Ok(Self::publish(&sink, s, Instant::now()))
    }

    /// Pause keeps the solo and the open (silent) output.
    pub fn pause(&mut self, id: &str) -> Result<TransportState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.transport.pause();
        Ok(Self::publish(&sink, s, Instant::now()))
    }

    /// Back to 0:00.0, solo off, output closed.
    pub fn stop(&mut self, id: &str) -> Result<TransportState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        Ok(Self::stop_session(&sink, s, Instant::now()))
    }

    fn stop_session(sink: &EventSink, s: &mut Session, now: Instant) -> TransportState {
        s.transport.stop();
        s.shared.audible.store(false, Ordering::Release);
        s.shared.position.store(0, Ordering::Release);
        s.shared.ended.store(false, Ordering::Release);
        s.solo = None;
        s.output = None;
        Self::publish(sink, s, now)
    }

    pub fn seek(&mut self, id: &str, position_ms: u64, now: Instant) -> Result<TransportState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let frames = ms_to_frames(position_ms, s.sample_rate).min(s.shared.total_frames);
        Ok(Self::seek_frames(&sink, s, frames, now))
    }

    pub fn nudge(&mut self, id: &str, delta_ms: i64, now: Instant) -> Result<TransportState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let cur = s.shared.position.load(Ordering::Acquire) as i64;
        let delta = delta_ms * i64::from(s.sample_rate) / 1000;
        let frames = (cur + delta).clamp(0, s.shared.total_frames as i64) as u64;
        Ok(Self::seek_frames(&sink, s, frames, now))
    }

    fn seek_frames(sink: &EventSink, s: &mut Session, frames: u64, now: Instant) -> TransportState {
        s.shared.position.store(frames, Ordering::Release);
        s.shared.ended.store(false, Ordering::Release);
        s.transport.seek(now);
        s.sync_shared();
        Self::publish(sink, s, now)
    }

    /// Sets a lane. `gain_db` is snapped to the 0.5 dB grid (below -59.75 it is Off).
    pub fn set_stem(&mut self, id: &str, name: &str, gain_db: Option<f32>, unmuted: bool) -> Result<LaneState, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let idx = s.lanes.iter().position(|l| l.name == name).ok_or_else(|| format!("no stem {name}"))?;
        s.lanes[idx].gain_db = gain_db.and_then(mixer::clamp_db);
        s.lanes[idx].unmuted = unmuted;
        if !unmuted && s.solo == Some(idx) {
            s.solo = None;
        }
        Self::publish(&sink, s, Instant::now());
        Ok(s.lanes[idx].clone())
    }

    /// Called every ~20 ms by the ticker thread.
    pub fn tick(&mut self, now: Instant) {
        let sink = self.sink.clone();
        let Some(s) = self.session.as_mut() else { return };
        if let Some(msg) = s.output.as_ref().and_then(|o| o.error()) {
            Self::stop_session(&sink, s, now);
            sink(EditorEvent::AudioError { id: s.id.clone(), message: msg });
            return;
        }
        if s.shared.ended.swap(false, Ordering::AcqRel) {
            Self::stop_session(&sink, s, now);
            return;
        }
        s.transport.tick(now);
        let clipped = s.shared.clipped.load(Ordering::Relaxed);
        if clipped != s.clip_seen {
            s.clip_seen = clipped;
            s.clip_until = Some(now + CLIP_HOLD);
        }
        s.clipping = s.clip_until.is_some_and(|u| now < u);
        Self::publish(&sink, s, now);
    }
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
