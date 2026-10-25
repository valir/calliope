//! The Player (plan 2.7): the real-time `Engine` owned by the audio callback and the
//! `PlayerManager` that opens a track, drives the engine through a bounded command channel,
//! and persists `player.json` (debounced, plus a periodic position save while playing).
//! The engine also mixes the metronome and count-in clicks and repeats loops (plans 2.4, 2.6).

use std::collections::VecDeque;
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
use crate::speed::{self, Training};
use crate::stretch::Stretcher;
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
pub(crate) const TRAINING_RUNNING: &str = "Stop the speed training first";
pub(crate) const LOOP_INCOMPLETE: &str = "Set the loop end first";

// ---------------------------------------------------------------------- engine

/// A beat of the grid: (source frame, first beat of a bar). Every beat clicks in the same tone.
#[doc(hidden)]
pub type Beat = (u64, bool);

/// The current complete loop, in source frames.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopSpec {
    pub start: u64,
    pub end: u64,
    /// Loop On: playback reaching the end jumps back to the start.
    pub repeat: bool,
    /// Count-in before every repeat (the loop's own Count-In): the song BPM at the loop start times 1000.
    /// The engine spaces the clicks by `60 / (bpm × speed)` of the coming pass.
    pub count_in_bpm_milli: Option<u32>,
    /// Song frames from the loop start to the next beat (0 on a beat): the count-in is spaced on the grid.
    pub count_in_lead: u64,
    /// Speed training of the loop (settings with `on`; only given when it is on and the loop repeats).
    pub training: Option<Training>,
}

/// Commands from the manager to the callback.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// Start sound at the engine position (restarts from 0 at the end of the track), after
    /// the count-in when given.
    Play {
        count_in: Option<CountIn>,
    },
    /// Jump to the loop start and play it; with Loop On it repeats, otherwise it stops at the end.
    PlayLoop {
        count_in: Option<CountIn>,
    },
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
    /// The track speed in percent (25..=200).
    SetSpeed(u32),
    /// The playback floor in frames (the first beat when it is set, else 0): where "restart at the end" goes.
    SetStart(u64),
}

/// Status written by the callback, read by the manager (atomics only).
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct Shared {
    /// Position in frames (the song frame being heard). The manager also stores here when no output is open.
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
    /// The speed heard now, in percent.
    pub speed_pct: AtomicU64,
    /// 0 = no training run, else the pass heard now plus one.
    pub pass: AtomicU64,
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

/// Output frames the stretch path processes at most at once.
const SUB_BLOCK: usize = 256;
/// Capacity of the timeline and the marks (no allocation after construction).
const TIMELINE: usize = 64;
const MARKS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Today's per-frame code (bit-exact at 100 %).
    Bypass,
    /// Through the time-stretcher.
    Stretch,
}

/// The heard song position is linear in the output frame between entries: `u + (out - entry.out) * speed`,
/// in the unwrapped feed coordinate (the feed keeps counting past a loop end; see `EventKind::Wrap`).
#[derive(Clone, Copy)]
struct Entry {
    out: u64,
    u: f64,
    speed: f64,
}

#[derive(Clone, Copy)]
enum EventKind {
    /// The feed wrapped here: the heard position goes back by `len`.
    /// `pass`/`pct`: the pass that starts and its speed (pass 0: no training run).
    Wrap { len: u64, pass: u32, pct: u32 },
    /// Loop end with count-in (`pass`/`pct` as for `Wrap`).
    CountIn {
        start: u64,
        bpm_milli: u32,
        lead: u64,
        pass: u32,
        pct: u32,
    },
    /// A loop played once ended.
    Once { start: u64 },
    /// The track ended.
    Track,
}

/// Something the feed met at unwrapped position `u`; it happens when the heard position gets there.
#[derive(Clone, Copy)]
struct FeedEvent {
    u: f64,
    kind: EventKind,
}

/// A speed-training run (2.6). `pass` is the pass the feed is in, `heard` the one being heard (0-based).
#[derive(Clone, Copy)]
struct Run {
    pass: u32,
    heard: u32,
    /// Speed of the pass heard, in percent.
    pct: u32,
    /// The loop the run belongs to.
    start: u64,
    end: u64,
}

/// Stereo f32 frames from song frame `from` (mono duplicated; zeros outside the track).
fn frame_lr(pcm: &Pcm, f: u64) -> (f32, f32) {
    if f >= pcm.frames {
        return (0.0, 0.0);
    }
    let ch = pcm.channels.max(1) as usize;
    let base = f as usize * ch;
    let l = f32::from(pcm.samples[base]) / 32768.0;
    let r = if ch >= 2 {
        f32::from(pcm.samples[base + 1]) / 32768.0
    } else {
        l
    };
    (l, r)
}

fn fill_stereo(pcm: &Pcm, from: u64, buf: &mut [f32]) {
    for (k, fr) in buf.chunks_exact_mut(2).enumerate() {
        let (l, r) = frame_lr(pcm, from + k as u64);
        fr[0] = l;
        fr[1] = r;
    }
}

/// Pure render state; never allocates or locks.
#[doc(hidden)]
pub struct Engine {
    pcm: Arc<Pcm>,
    pos: u64,
    /// The playback floor (`Cmd::SetStart`).
    start: u64,
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
    // stretch mode
    mode: Mode,
    /// Track speed in percent, and the ratio fed now.
    base_pct: u32,
    speed: f64,
    stretch: Stretcher,
    l_in: u64,
    l_out: u64,
    /// Input to the stretcher (and the alignment pre-roll).
    scratch: Vec<f32>,
    /// Stretcher output not yet heard.
    stage: Vec<f32>,
    stage_len: usize,
    stage_at: usize,
    /// Next song frame to feed, and the fraction of a frame fed ahead of it.
    fpos: u64,
    acc: f64,
    /// Sum of the loop lengths the feed has wrapped / the heard side has passed.
    woff: u64,
    hoff: u64,
    /// The feed ended (loop end with count-in, loop played once, track end): zeros follow.
    feed_zero: bool,
    feed_fade_left: u32,
    /// Output frames heard in stretch mode.
    out_count: u64,
    timeline: VecDeque<Entry>,
    events: VecDeque<FeedEvent>,
    /// Crossfade from the bypass path (still at `xpos`) after a speed change away from 100 %.
    xfade_left: u32,
    xpos: u64,
    /// The crossfade runs the other way: stretch -> bypass after a speed change back to 100 %.
    to_bypass: bool,
    run: Option<Run>,
}

impl Engine {
    pub fn new(pcm: Arc<Pcm>, pos: u64, shared: Arc<Shared>) -> Self {
        let fade_len = (pcm.rate * FADE_MS / 1000).max(1);
        let pos = pos.min(pcm.frames);
        let click = Click::new(pcm.rate);
        let stretch = Stretcher::new(pcm.rate);
        let scratch = vec![0.0; 2 * stretch.pre_roll_frames().max(2 * SUB_BLOCK + 8)];
        shared.speed_pct.store(100, Ordering::Release);
        shared.pass.store(0, Ordering::Release);
        Engine {
            pcm,
            pos,
            start: 0,
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
            mode: Mode::Bypass,
            base_pct: 100,
            speed: 1.0,
            l_in: stretch.input_latency() as u64,
            l_out: stretch.output_latency() as u64,
            stretch,
            scratch,
            stage: vec![0.0; 2 * SUB_BLOCK],
            stage_len: 0,
            stage_at: 0,
            fpos: 0,
            acc: 0.0,
            woff: 0,
            hoff: 0,
            feed_zero: false,
            feed_fade_left: 0,
            out_count: 0,
            timeline: VecDeque::with_capacity(TIMELINE),
            events: VecDeque::with_capacity(MARKS),
            xfade_left: 0,
            to_bypass: false,
            xpos: 0,
            run: None,
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

    /// A discontinuity: chooses the path for the audio that follows from song frame `target`
    /// (`pos` is already there). Exactly 100 % is the bypass; anything else aligns the stretcher.
    fn enter(&mut self, target: u64) {
        self.xfade_left = 0;
        self.to_bypass = false;
        if let Some(r) = self.run.as_mut() {
            // a run restarts the feed in the pass being heard
            r.pass = r.heard;
            if let Some(t) = self.lp.and_then(|l| l.training) {
                self.speed = f64::from(t.pass_speed(r.pass)) / 100.0;
            }
        }
        if self.base_pct == 100 && self.run.is_none() {
            self.mode = Mode::Bypass;
            return;
        }
        let pcm = &*self.pcm;
        // a repeating loop the target is inside: the audio stops at its end (with the fade-out)
        let wrapping = self
            .wrapping()
            .filter(|l| target < l.end && l.end <= pcm.frames);
        let fade = u64::from(self.fade_len);
        let fed = self.stretch.align(
            |f, b| {
                fill_stereo(pcm, f, b);
                if let Some(l) = wrapping {
                    for (k, fr) in b.chunks_exact_mut(2).enumerate() {
                        let song = f + k as u64;
                        if song + fade >= l.end {
                            let g = if song >= l.end {
                                0.0
                            } else {
                                (l.end - song) as f32 / fade as f32
                            };
                            fr[0] *= g;
                            fr[1] *= g;
                        }
                    }
                }
            },
            target,
            self.speed,
            &mut self.scratch,
        );
        self.mode = Mode::Stretch;
        self.fpos = fed;
        self.acc = 0.0;
        self.woff = 0;
        self.hoff = 0;
        self.feed_zero = false;
        self.feed_fade_left = 0;
        self.stage_len = 0;
        self.stage_at = 0;
        self.events.clear();
        self.timeline.clear();
        self.timeline.push_back(Entry {
            out: self.out_count,
            u: target as f64,
            speed: self.speed,
        });
        self.beat_dirty = true;
        self.voice = None;
        if let Some(l) = wrapping.filter(|l| fed >= l.end) {
            // the alignment already fed past the loop end: the end is met now
            self.loop_end(Some(l), l.end as f64, fed - l.end);
        } else if fed >= self.pcm.frames {
            self.push_event(FeedEvent {
                u: self.pcm.frames as f64,
                kind: EventKind::Track,
            });
            self.feed_zero = true;
        }
    }

    /// The feed met the end of loop `l` (or of the track) at unwrapped position `at`, `over` frames short of
    /// where the feed is now.
    fn loop_end(&mut self, l: Option<LoopSpec>, at: f64, over: u64) {
        match l {
            Some(l) if l.repeat && l.count_in_bpm_milli.is_none() => {
                let (pass, pct) = self.coming_pass(&l);
                self.push_event(FeedEvent {
                    u: at,
                    kind: EventKind::Wrap {
                        len: l.end - l.start,
                        pass,
                        pct,
                    },
                });
                if let Some(r) = self.run.as_mut().filter(|_| pass > 0) {
                    r.pass += 1;
                    self.speed = f64::from(pct) / 100.0; // the new speed applies from the loop start
                }
                self.woff += l.end - l.start;
                self.fpos = l.start + over;
                self.feed_fade_left = self.fade_len;
            }
            Some(l) if l.repeat => {
                let bpm_milli = l.count_in_bpm_milli.unwrap_or(0);
                let (pass, pct) = self.coming_pass(&l);
                let lead = l.count_in_lead;
                self.push_event(FeedEvent {
                    u: at,
                    kind: EventKind::CountIn {
                        start: l.start,
                        bpm_milli,
                        lead,
                        pass,
                        pct,
                    },
                });
                self.feed_zero = true;
            }
            Some(l) => {
                self.push_event(FeedEvent {
                    u: at,
                    kind: EventKind::Once { start: l.start },
                });
                self.feed_zero = true;
            }
            None => {
                self.push_event(FeedEvent {
                    u: at,
                    kind: EventKind::Track,
                });
                self.feed_zero = true;
            }
        }
    }

    /// The pass after the one the feed is in and its speed (pass 0: no run), read from the loop's current plan.
    fn coming_pass(&self, l: &LoopSpec) -> (u32, u32) {
        match (self.run, l.training) {
            (Some(r), Some(t)) => (r.pass + 2, t.pass_speed(r.pass + 1)),
            _ => (0, 0),
        }
    }

    /// The run is over: the track speed again (the ratio changes; the mode stays until the next discontinuity).
    fn end_run(&mut self) {
        if self.run.take().is_some() {
            self.speed = f64::from(self.base_pct) / 100.0;
        }
    }

    fn push_event(&mut self, m: FeedEvent) {
        debug_assert!(self.events.len() < MARKS, "feed events overflow");
        if self.events.len() < MARKS {
            self.events.push_back(m);
        }
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
        self.enter(self.pos);
    }

    pub fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { count_in } => {
                self.end_run();
                if self.pos >= self.pcm.frames {
                    self.pos = self.start.min(self.pcm.frames);
                }
                self.begin(count_in);
            }
            Cmd::PlayLoop { count_in } => {
                self.end_run(); // a new PlayLoop restarts at pass 0
                if let Some(l) = self.lp {
                    self.pos = l.start.min(self.pcm.frames);
                    self.loop_play = true;
                    if let Some(t) = l.training.filter(|_| l.repeat) {
                        let pct = t.pass_speed(0);
                        self.run = Some(Run {
                            pass: 0,
                            heard: 0,
                            pct,
                            start: l.start,
                            end: l.end,
                        });
                        self.speed = f64::from(pct) / 100.0;
                    }
                } else if self.pos >= self.pcm.frames {
                    self.pos = self.start.min(self.pcm.frames);
                }
                self.begin(count_in);
            }
            Cmd::Pause => {
                self.end_run();
                self.playing = false;
                self.audible = false;
                self.count = None;
                self.loop_play = false;
                self.voice = None;
            }
            Cmd::Seek(f) => {
                self.leave_run(f);
                self.move_to(f);
                self.count = None;
                if self.playing {
                    self.audible = false;
                }
            }
            Cmd::Jump(f) => {
                self.leave_run(f);
                self.move_to(f);
                self.count = None;
                if self.playing && self.audible {
                    self.start_fade();
                    self.enter(self.pos);
                }
            }
            Cmd::Resume => {
                if self.playing {
                    self.audible = true;
                    self.start_fade();
                    self.enter(self.pos);
                }
            }
            Cmd::SetLoop(l) => {
                self.lp = l;
                if l.is_none() {
                    self.loop_play = false;
                }
                // the run goes on only for the same loop that still repeats with training; changed settings
                // are read at the next wrap
                if let Some(r) = self.run {
                    if !l.is_some_and(|l| {
                        l.repeat && l.training.is_some() && l.start == r.start && l.end == r.end
                    }) {
                        self.end_run();
                    }
                }
            }
            Cmd::SetGrid(g) => {
                let old = std::mem::replace(&mut self.grid, g);
                self.beat_dirty = true;
                if let Some(t) = &self.trash {
                    let _ = t.try_send(old); // when full, freed here (rare)
                }
            }
            Cmd::SetStart(f) => self.start = f,
            Cmd::SetMetronome(on) => {
                self.metronome = on;
                self.beat_dirty = true;
            }
            Cmd::SetSpeed(pct) => {
                self.base_pct = pct;
                if self.run.is_some() {
                    return; // a run plays its own speeds; the track speed is back when it ends
                }
                if self.to_bypass {
                    // a change during the way back to the bypass: stay in the stretch path
                    self.to_bypass = false;
                    self.xfade_left = 0;
                }
                self.speed = f64::from(pct) / 100.0;
                if self.playing && self.count.is_some() {
                    self.enter(self.pos); // before the music starts: align for the new speed
                } else if self.playing && self.audible && self.mode == Mode::Bypass && pct != 100 {
                    // bypass -> stretch: align here and crossfade the two paths
                    let at = self.pos;
                    self.enter(at);
                    self.xpos = at;
                    self.xfade_left = self.fade_len;
                    self.fade_left = 0;
                } else if self.playing
                    && self.audible
                    && self.mode == Mode::Stretch
                    && pct == 100
                    && self.xfade_left == 0
                {
                    // stretch -> 100 %: back to the bypass path, crossfaded from the heard position
                    self.xpos = self.pos;
                    self.xfade_left = self.fade_len;
                    self.to_bypass = true;
                }
                // stretch -> another speed: only the feed ratio changes (`self.speed`)
            }
        }
    }

    /// A move to song frame `f` outside the loop of a run ends it.
    fn leave_run(&mut self, f: u64) {
        if self.run.is_some_and(|r| f < r.start || f >= r.end) {
            self.end_run();
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

    /// The count-in of a loop wrap, spaced for the coming pass; `lead` song frames before the next beat.
    fn wrap_count(&self, bpm_milli: u32, lead: u64, speed: f64) -> CountState {
        let plan = count_in_plan(f64::from(bpm_milli) / 1000.0 * speed, self.pcm.rate);
        CountState {
            plan: plan.lead_in((lead as f64 / speed).round() as u64),
            at: 0,
        }
    }

    /// The loop the feed wraps at, when it is in play.
    fn wrapping(&self) -> Option<LoopSpec> {
        self.lp.filter(|l| l.repeat || self.loop_play)
    }

    /// Feeds the stretcher one sub-block and leaves its output in the stage.
    fn fill_stage(&mut self) {
        let s = self.speed;
        let frames = self.pcm.frames;
        let wraps = self.wrapping();
        // where the feed has to stop: a loop end the feed is before, else the track end
        let event = if self.feed_zero || self.fpos >= frames {
            None
        } else {
            match wraps.filter(|l| self.fpos < l.end && l.end <= frames) {
                Some(l) => Some((l.end, Some(l))),
                None => Some((frames, None)),
            }
        };
        let u0 = (self.fpos + self.woff) as f64 + self.acc - self.l_in as f64;
        let changed = self.timeline.back().is_none_or(|e| e.speed != s);
        debug_assert!(
            !changed || self.timeline.len() < TIMELINE,
            "speed timeline overflow"
        );
        if changed && self.timeline.len() < TIMELINE {
            // `out` ignores the (at most SUB_BLOCK - 1, ~6 ms) stage frames not yet heard, so a new speed is
            // shown that much early at worst; the position it moves is below the display's resolution.
            self.timeline.push_back(Entry {
                out: self.out_count + self.l_out,
                u: u0,
                speed: s,
            });
        }
        let exact = self.acc + SUB_BLOCK as f64 * s;
        let mut m = exact as usize;
        let mut n = SUB_BLOCK;
        let mut hit = false;
        if let Some((e, _)) = event {
            let r = (e - self.fpos) as usize;
            if m >= r {
                // cut here so the event falls on the block boundary
                m = r;
                n = (((m as f64 - self.acc) / s).round() as usize).clamp(1, SUB_BLOCK);
                hit = true;
            }
        }
        // the source, with the 3 ms fades at the loop edges
        let input = &mut self.scratch[..2 * m];
        if self.feed_zero {
            input.fill(0.0);
        } else {
            let pcm = &*self.pcm;
            let mut ffl = self.feed_fade_left;
            let fade = self.fade_len;
            for (k, fr) in input.chunks_exact_mut(2).enumerate() {
                let f = self.fpos + k as u64;
                let (l, r) = frame_lr(pcm, f);
                let mut g = 1.0f32;
                if ffl > 0 {
                    g = (fade - ffl) as f32 / fade as f32;
                    ffl -= 1;
                }
                if let Some(lp) = wraps {
                    if f < lp.end && f >= lp.start && lp.end - f <= u64::from(fade) {
                        g *= (lp.end - f) as f32 / fade as f32;
                    }
                }
                fr[0] = l * g;
                fr[1] = r * g;
            }
            self.feed_fade_left = ffl;
        }
        self.stretch
            .process(&self.scratch[..2 * m], &mut self.stage[..2 * n]);
        self.stage_len = n;
        self.stage_at = 0;
        self.fpos += m as u64;
        if !hit {
            self.acc = exact - m as f64;
            return;
        }
        self.acc = 0.0;
        let at = (self.fpos + self.woff) as f64;
        self.loop_end(event.and_then(|e| e.1), at, 0);
    }

    /// The heard position entered pass `pass` (1-based, 0 = none) of the run, at `pct` %.
    fn heard_pass(&mut self, pass: u32, pct: u32) {
        if let Some(r) = self.run.as_mut().filter(|_| pass > 0) {
            r.heard = pass - 1;
            r.pct = pct;
        }
    }

    /// The heard position reached a mark. True when it stopped or restarted the audio (no frame this time).
    fn reached(&mut self, kind: EventKind) -> bool {
        match kind {
            EventKind::Wrap { len, pass, pct } => {
                self.heard_pass(pass, pct);
                self.hoff += len;
                self.beat_dirty = true;
                self.shared.wraps.fetch_add(1, Ordering::AcqRel);
                false
            }
            EventKind::CountIn {
                start,
                bpm_milli,
                lead,
                pass,
                pct,
            } => {
                self.heard_pass(pass, pct);
                self.shared.wraps.fetch_add(1, Ordering::AcqRel);
                self.pos = start;
                self.beat_dirty = true;
                let speed = if pass > 0 {
                    f64::from(pct) / 100.0
                } else {
                    self.speed
                };
                self.count = Some(self.wrap_count(bpm_milli, lead, speed));
                self.enter(start);
                true
            }
            EventKind::Once { start } => {
                self.pos = start; // played once: back to the loop start
                self.playing = false;
                self.audible = false;
                self.loop_play = false;
                self.voice = None;
                self.shared.loop_stopped.store(true, Ordering::Release);
                true
            }
            EventKind::Track => {
                self.end_run();
                self.pos = self.pcm.frames;
                self.playing = false;
                self.audible = false;
                self.loop_play = false;
                self.shared.ended.store(true, Ordering::Release);
                true
            }
        }
    }

    /// One output frame of the stretch path into `out[i]`; false when the frame was spent on an event.
    fn stretch_frame(&mut self, out: &mut [f32], i: usize) -> bool {
        if self.stage_at >= self.stage_len {
            self.fill_stage();
        }
        let o = self.out_count;
        while self.timeline.len() > 1 && self.timeline[1].out <= o {
            self.timeline.pop_front();
        }
        let e = self.timeline[0];
        let u = e.u + o.saturating_sub(e.out) as f64 * e.speed;
        while let Some(mk) = self.events.front().copied() {
            if u < mk.u {
                break;
            }
            self.events.pop_front();
            if self.reached(mk.kind) {
                return false;
            }
        }
        let song = (u - self.hoff as f64).max(0.0);
        if self.metronome {
            if self.beat_dirty {
                self.beat_idx = self.grid.partition_point(|b| (b.0 as f64) < song);
                self.beat_dirty = false;
            }
            while let Some(&(f, _)) = self.grid.get(self.beat_idx) {
                if f as f64 > song {
                    break;
                }
                self.voice = Some(Voice { at: 0 });
                self.beat_idx += 1;
            }
        }
        let (mut l, mut r) = (
            self.stage[2 * self.stage_at],
            self.stage[2 * self.stage_at + 1],
        );
        self.stage_at += 1;
        self.out_count += 1;
        if self.xfade_left > 0 {
            let mut w = (self.fade_len - self.xfade_left) as f32 / self.fade_len as f32;
            if self.to_bypass {
                w = 1.0 - w; // the stretch path fades out
            }
            let (bl, br) = frame_lr(&self.pcm, self.xpos);
            l = l * w + bl * (1.0 - w);
            r = r * w + br * (1.0 - w);
            self.xfade_left -= 1;
            self.xpos += 1;
        }
        let g = if self.fade_left > 0 {
            let g = (self.fade_len - self.fade_left) as f32 / self.fade_len as f32;
            self.fade_left -= 1;
            g
        } else {
            1.0
        };
        out[2 * i] = l * g;
        out[2 * i + 1] = r * g;
        self.mix_voice(out, i);
        out[2 * i] = out[2 * i].clamp(-1.0, 1.0);
        out[2 * i + 1] = out[2 * i + 1].clamp(-1.0, 1.0);
        self.pos = (song as u64).min(self.pcm.frames);
        if self.to_bypass {
            // the heard position follows the bypass side, so it never goes back
            self.pos = self.xpos.min(self.pcm.frames);
            if self.xfade_left == 0 {
                self.to_bypass = false;
                self.mode = Mode::Bypass;
                self.beat_dirty = true;
                self.events.clear();
                self.timeline.clear();
                // a loop end crossed during the return fade is handled here (the bypass check is exact)
                let crossed = self
                    .wrapping()
                    .filter(|l| self.pos >= l.end && l.end <= self.pcm.frames);
                if let Some(l) = crossed {
                    self.bypass_loop_end(l);
                } else if self.pos >= self.pcm.frames {
                    self.playing = false;
                    self.audible = false;
                    self.loop_play = false;
                    self.shared.ended.store(true, Ordering::Release);
                }
            }
        }
        true
    }

    /// The bypass path reached the end of the wrapping loop `l`: wrap (with its count-in rule) or stop.
    fn bypass_loop_end(&mut self, l: LoopSpec) {
        if l.repeat {
            self.pos = l.start;
            self.beat_dirty = true;
            self.shared.wraps.fetch_add(1, Ordering::AcqRel);
            self.count = l
                .count_in_bpm_milli
                .map(|b| self.wrap_count(b, l.count_in_lead, self.speed));
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
            if self.mode == Mode::Stretch {
                if self.stretch_frame(out, i) {
                    i += 1;
                }
                continue;
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
            let r = if ch >= 2 {
                f32::from(self.pcm.samples[base + 1]) / 32768.0
            } else {
                l
            };
            let mut g = if self.fade_left > 0 {
                let g = (self.fade_len - self.fade_left) as f32 / self.fade_len as f32;
                self.fade_left -= 1;
                g
            } else {
                1.0
            };
            let wraps = self.wrapping();
            if let Some(l) = wraps {
                if self.pos < l.end
                    && self.pos >= l.start
                    && l.end - self.pos <= u64::from(self.fade_len)
                {
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
                self.bypass_loop_end(l);
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
        let heard = match self.timeline.front() {
            _ if self.playing && self.run.is_some() => self.run.map_or(0, |r| u64::from(r.pct)),
            Some(e) if self.mode == Mode::Stretch && self.playing && self.audible => {
                (e.speed * 100.0).round() as u64
            }
            _ => u64::from(self.base_pct),
        };
        self.shared.speed_pct.store(heard, Ordering::Release);
        let pass = self
            .run
            .filter(|_| self.playing)
            .map_or(0, |r| u64::from(r.heard) + 1);
        self.shared.pass.store(pass, Ordering::Release);
        self.shared
            .counting_in
            .store(self.count.is_some(), Ordering::Release);
        self.shared
            .loop_playing
            .store(self.playing && self.loop_play, Ordering::Release);
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
    /// The song BPM of the section under the playhead (0 = none), not scaled by the speed.
    pub bpm: f64,
    /// `bpm × speed_pct / 100` (0 = none).
    pub played_bpm: f64,
    /// The speed heard now (the pass speed during a training run).
    pub speed_pct: u32,
    pub tempo_index: Option<usize>,
    /// The running speed training, else none.
    pub training: Option<TrainingProgress>,
}

/// The running training as the Loops pane shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TrainingProgress {
    #[serde(rename = "loop")]
    pub loop_index: usize,
    pub speed_pct: u32,
    pub repeat: u32,
    pub of: Option<u32>,
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
    /// The training settings (the defaults when the file has none).
    pub training: Training,
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
    /// The track speed in percent.
    pub speed_pct: u32,
    pub loops: Vec<PlayerLoop>,
    pub current_loop: Option<usize>,
    pub loop_message: Option<String>,
    pub transport: PlayerTransport,
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

impl Loaded {
    /// Whether the automatic search for the first beat may run on this track (a tempo whose first beat is
    /// at exactly 0.0 and not set yet, and a writable state).
    pub fn wants_first_beat(&self) -> bool {
        !self.state.read_only && self.state.tempo.needs_first_beat()
    }

    /// Key of the decoded source.
    pub fn source_key(&self) -> &str {
        &self.source
    }

    pub fn pcm(&self) -> &Pcm {
        &self.pcm
    }
}

/// Who asked for the first beat to be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstBeatVia {
    /// The automatic run at open (re-checks `needs_first_beat`).
    Auto,
    /// The Find start button (always moves it).
    Button,
}

/// What "Find start" needs to analyse, taken under the lock.
#[derive(Clone)]
pub struct FirstBeatInput {
    /// Key of the analysis source (the track's backing).
    pub key: String,
    /// Its absolute file.
    pub file: PathBuf,
    /// Its PCM when the Player has it loaded (no second decode).
    pub pcm: Option<Arc<Pcm>>,
    pub ffmpeg: Option<PathBuf>,
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
            sink(PlayerEvent::Loading {
                id: id.clone(),
                progress: p,
            });
        }
    }
}

/// Revision of `<dir>/player.json`: the FNV-1a hash of its bytes; none when it can't be read.
fn revision_of(dir: &std::path::Path) -> Option<String> {
    std::fs::read(dir.join(player_state::PLAYER_FILE))
        .ok()
        .map(|b| crate::fsutil::fnv1a64_hex(&b))
}

/// Reads `player.json`, picks the source and decodes it (long; call without the manager lock).
pub(crate) fn load(
    spec: &OpenSpec,
    ticket: &OpenTicket,
    sink: &EventSink,
) -> Result<Loaded, String> {
    // The bytes are read first, so the state is at least as new as the revision.
    let revision = revision_of(&spec.dir);
    let (mut state, note, _writable) = player_state::load(&spec.dir);
    let keys: Vec<&str> = spec.sources.iter().map(|s| s.key.as_str()).collect();
    let source = player_state::choose_source(state.source.as_deref(), &keys)
        .ok_or_else(|| format!("{} has no audio to play.", spec.title))?;
    let entry = spec
        .sources
        .iter()
        .find(|s| s.key == source)
        .expect("chosen from the list");
    let mut progress = progress_sink(&spec.id, sink);
    let pcm = player_source::decode(
        &spec.dir.join(&entry.file),
        spec.ffmpeg.as_deref(),
        &ticket.cancel,
        &mut progress,
    )?;
    if ticket.cancel.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    let peaks = player_source::peaks(&pcm, PEAK_BUCKETS);
    state.source = Some(source.clone());
    Ok(Loaded {
        state,
        note,
        source,
        pcm,
        peaks,
        revision,
    })
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
        let mut engine =
            Engine::new(self.pcm.clone(), self.start, self.shared.clone()).with_trash(trash_tx);
        let handle = self.backend.open(
            self.pcm.rate,
            Box::new(move |out| {
                while let Ok(c) = rx.try_recv() {
                    engine.apply(c);
                }
                engine.render(out);
            }),
        )?;
        Ok(OpenedOutput {
            shared: self.shared,
            output: Output { handle, tx, trash },
        })
    }
}

#[derive(PartialEq, Clone, Copy)]
struct EventKey {
    playing: bool,
    resume_pending: bool,
    counting_in: bool,
    loop_playing: bool,
    speed_pct: u64,
    pass: u64,
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
    /// `Shared.pass` last seen by `tick`, and the loop it ran in (for the log).
    last_pass: u64,
    last_pass_loop: Option<usize>,
    /// The loop (start, end) in ms the click grid was last computed for (10.6).
    grid_wrap: Option<(f64, f64)>,
    /// A loop Play with training started a run the callback has not shown yet (`Shared.pass` is still 0):
    /// the speed is refused until then (the run is seen, or this deadline passes).
    run_until: Option<Instant>,
}

impl Session {
    fn pos(&self) -> u64 {
        self.shared.position.load(Ordering::Acquire)
    }

    /// The playback floor in frames: the set first beat (nearest frame), else 0.
    fn floor(&self) -> u64 {
        self.frame_of_ms_f(self.state.tempo.play_start_ms())
    }

    /// Stores a position clamped to `floor..=frames` and returns the frame actually set.
    fn set_pos(&self, frames: u64) -> u64 {
        let f = frames.min(self.pcm.frames).max(self.floor());
        self.shared.position.store(f, Ordering::Release);
        f
    }

    /// After a tempo change: a position below the new floor moves to it (a jump while playing).
    fn clamp_to_floor(&self) {
        let floor = self.floor();
        if self.pos() < floor {
            self.shared.position.store(floor, Ordering::Release);
            self.send(if self.transport.shows_playing() {
                Cmd::Jump(floor)
            } else {
                Cmd::Seek(floor)
            });
        }
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
        (((ms * f64::from(self.pcm.rate) / 1000.0) - 1e-6)
            .ceil()
            .max(0.0) as u64)
            .min(self.pcm.frames)
    }

    fn boundary_ms(&self, b: Boundary) -> f64 {
        match b {
            Boundary::Ms { ms } => ms,
            Boundary::Tick { tick } => self.state.tempo.ms_at(tick),
        }
    }

    /// The count-in for a start at `ms`, on the beat grid at `pct`; none without a tempo.
    fn plan_at(&self, ms: f64, pct: u32) -> Option<CountIn> {
        let bpm = self.state.tempo.bpm_at(ms) * f64::from(pct) / 100.0;
        let lead = (self.beat_lead(ms) as f64 * 100.0 / f64::from(pct.max(1))).round() as u64;
        (!self.state.tempo.is_empty() && bpm > 0.0)
            .then(|| count_in_plan(bpm, self.pcm.rate).lead_in(lead))
    }

    /// Song frames from a start at `ms` to the next beat of the grid (0 on a beat or without a tempo).
    fn beat_lead(&self, ms: f64) -> u64 {
        let bpm = self.state.tempo.bpm_at(ms);
        if bpm <= 0.0 {
            return 0;
        }
        let half_frame = 500.0 / f64::from(self.pcm.rate.max(1));
        let beat = 60000.0 / bpm;
        match self
            .state
            .tempo
            .beats_in(ms - half_frame, ms + 2.0 * beat)
            .first()
        {
            Some(&(g, _)) => self.frame_of_ms_f(g).saturating_sub(self.frame_of_ms_f(ms)),
            None => 0,
        }
    }

    /// Logs a count-in; Play adds the index of the section that counted in.
    fn log_count_in(&self, plan: &Option<CountIn>, section: Option<usize>) {
        if let Some(p) = plan {
            let section = section.map_or_else(String::new, |i| format!(" section={i}"));
            eprintln!(
                "calliope: player count-in clicks={} beat_ms={}{section}",
                p.clicks.len(),
                self.ms_of(p.beat)
            );
        }
    }

    fn grid(&self) -> Arc<Vec<Beat>> {
        let rate = f64::from(self.pcm.rate);
        let dur_ms = self.pcm.frames as f64 * 1000.0 / rate;
        let clicks = section_clicks(&self.state.tempo, dur_ms, self.wrap_ms());
        let mut frames: Vec<u64> = clicks
            .into_iter()
            .map(|ms| (ms * rate / 1000.0).round() as u64)
            .collect();
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
        let start = self
            .frame_of_ms_f(self.boundary_ms(l.start))
            .max(self.floor());
        if end <= start {
            return None;
        }
        let count_in_bpm_milli = (l.count_in && !self.state.tempo.is_empty())
            .then(|| (self.state.tempo.bpm_at(self.boundary_ms(l.start)) * 1000.0).round() as u32)
            .filter(|&b| b > 0);
        let count_in_lead = self.beat_lead(self.boundary_ms(l.start));
        Some(LoopSpec {
            start,
            end,
            repeat: l.repeat,
            count_in_bpm_milli,
            count_in_lead,
            training: l.training.filter(|t| t.on && l.repeat),
        })
    }

    fn sync_loop(&mut self) {
        self.send(Cmd::SetLoop(self.loop_spec()));
        if self.wrap_ms() != self.grid_wrap {
            self.sync_grid();
        }
    }

    fn sync_grid(&mut self) {
        self.grid_wrap = self.wrap_ms();
        self.send(Cmd::SetStart(self.floor()));
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
            speed_pct: self.shared.speed_pct.load(Ordering::Acquire),
            pass: self.shared.pass.load(Ordering::Acquire),
        }
    }

    /// The speed heard now: the engine's while an output is open, else the track speed.
    fn heard_pct(&self) -> u32 {
        if self.output.is_some() {
            self.shared.speed_pct.load(Ordering::Acquire) as u32
        } else {
            self.state.speed_pct()
        }
    }

    /// The training run being heard (`Shared.pass` is 0 without one) with its loop.
    fn training_progress(&self) -> Option<TrainingProgress> {
        let pass = self.shared.pass.load(Ordering::Acquire);
        let loop_index = self.loop_index()?;
        let t = self.state.current()?.training?;
        let p = t.progress(u32::try_from(pass.checked_sub(1)?).ok()?);
        Some(TrainingProgress {
            loop_index,
            speed_pct: p.speed_pct,
            repeat: p.repeat,
            of: p.of,
        })
    }

    fn transport_state(&self) -> PlayerTransport {
        let ms = self.ms_of(self.pos());
        // The section lookup uses the exact frame time: the whole-ms `position_ms` floors below a fractional
        // section start the Start jump just landed on (a tolerance absorbs float noise on whole-frame times).
        let exact = self.pos_ms_f() + 1e-6;
        let bpm = self.state.tempo.bpm_at(exact);
        let speed_pct = self.heard_pct();
        PlayerTransport {
            playing: self.transport.shows_playing(),
            counting_in: self.shared.counting_in.load(Ordering::Acquire),
            resume_pending: self.transport.resume_pending(),
            position_ms: ms,
            loop_playing: self.shared.loop_playing.load(Ordering::Acquire),
            bpm,
            played_bpm: bpm * f64::from(speed_pct) / 100.0,
            speed_pct,
            tempo_index: self.state.tempo.segment_at(exact),
            training: self.training_progress(),
        }
    }

    fn snapshot(&self) -> PlayerSnapshot {
        let st = &self.state;
        let ms_of = |b: Boundary| match b {
            Boundary::Ms { ms } => ms,
            Boundary::Tick { tick } => st.tempo.ms_at(tick),
        };
        let loops: Vec<PlayerLoop> = st
            .loops
            .iter()
            .map(|l: &LoopRec| PlayerLoop {
                id: l.id.clone(),
                start: l.start,
                end: l.end,
                start_ms: ms_of(l.start),
                end_ms: l.end.map(ms_of),
                repeat: l.repeat,
                count_in: l.count_in,
                training: l.training.unwrap_or_default(),
            })
            .collect();
        let current_loop = st
            .current_loop
            .as_ref()
            .and_then(|c| st.loops.iter().position(|l| &l.id == c));
        PlayerSnapshot {
            id: self.id.clone(),
            title: self.title.clone(),
            sources: self
                .sources
                .iter()
                .map(|s| SourceEntry {
                    key: s.key.clone(),
                    label: s.label.clone(),
                })
                .collect(),
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
            speed_pct: st.speed_pct(),
            loops,
            current_loop,
            loop_message: self.loop_message.clone(),
            transport: self.transport_state(),
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
    let due = s
        .last_emit
        .is_none_or(|t| now.saturating_duration_since(t) >= EVENT_INTERVAL);
    if key_changed || (key.playing && due) || (!key.playing && moved) {
        if key_changed
            && (
                key.playing,
                key.resume_pending,
                key.counting_in,
                key.loop_playing,
            ) != (
                s.last_key.playing,
                s.last_key.resume_pending,
                s.last_key.counting_in,
                s.last_key.loop_playing,
            )
        {
            eprintln!(
                "calliope: player transport playing={} counting_in={} loop={} position_ms={}",
                state.playing,
                state.counting_in,
                if state.loop_playing {
                    s.loop_index().map_or("none".to_string(), |n| n.to_string())
                } else {
                    "none".into()
                },
                state.position_ms
            );
        }
        s.last_key = key;
        s.last_position = state.position_ms;
        s.last_emit = Some(now);
        sink(PlayerEvent::Transport {
            id: s.id.clone(),
            state: state.clone(),
        });
    }
    state
}

/// A change of the track speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedAction {
    /// -1 / +1 per press: the next multiple of 5 in that direction.
    Step(i32),
    Reset,
    /// A typed percentage (25..=200 or refused).
    Set(i64),
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
    Training(Training),
    Play,
    Stop,
}

/// One edit of the tempo sections (the Player's Sections group and waveform).
#[derive(Debug, Clone, PartialEq)]
pub enum SectionEdit {
    Bpm {
        index: usize,
        at: f64,
        bpm: f64,
    },
    Scale {
        index: usize,
        at: f64,
        factor: Scale,
    },
    Add {
        position: f64,
        bpm: f64,
    },
    Remove {
        index: usize,
        at: f64,
    },
    Move {
        index: usize,
        at: f64,
        steps: i64,
    },
    Flags {
        index: usize,
        at: f64,
        metronome: bool,
        count_in: bool,
    },
    Name {
        index: usize,
        at: f64,
        name: String,
    },
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
        Self {
            backend,
            sink,
            session: None,
            pending: None,
            generation: 0,
            trash: Vec::new(),
            defer_drops: false,
        }
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
        let ticket = OpenTicket {
            gen: self.generation,
            cancel: Arc::new(AtomicBool::new(false)),
        };
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
    pub fn finish_open(
        &mut self,
        ticket: &OpenTicket,
        spec: &OpenSpec,
        loaded: Loaded,
    ) -> Result<PlayerSnapshot, String> {
        if ticket.gen != self.generation || ticket.cancel.load(Ordering::Relaxed) {
            return Err(SUPERSEDED.into());
        }
        self.pending = None;
        let Loaded {
            mut state,
            mut note,
            source,
            pcm,
            peaks,
            revision,
        } = loaded;
        if revision_of(&spec.dir) != revision {
            // `player.json` was written (the Editor's tempo map) after it was read: use the new
            // content, so that write is not lost. The decoded source stays.
            let (fresh, fresh_note, _) = player_state::load(&spec.dir);
            eprintln!(
                "calliope: player reread id={} (player.json changed while opening)",
                spec.id
            );
            state = fresh;
            note = fresh_note;
        }
        let pcm = Arc::new(pcm);
        let mut frames = state.position_ms.saturating_mul(u64::from(pcm.rate)) / 1000;
        if frames >= pcm.frames {
            frames = 0; // past the end gives 0:00.0
        }
        let floor = (state.tempo.play_start_ms() * f64::from(pcm.rate) / 1000.0)
            .round()
            .max(0.0) as u64;
        frames = frames.max(floor.min(pcm.frames));
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
            last_key: EventKey {
                playing: false,
                resume_pending: false,
                counting_in: false,
                loop_playing: false,
                speed_pct: 0,
                pass: 0,
            },
            last_position: 0,
            last_emit: None,
            loop_message: None,
            last_wraps: 0,
            last_pass: 0,
            last_pass_loop: None,
            grid_wrap: None,
            run_until: None,
        };
        session.state.position_ms = session.ms_of(frames); // what is on disk or equal to it
        let snap = session.snapshot();
        eprintln!(
            "calliope: player open id={} source={} duration_ms={} rate={} position_ms={}",
            snap.id, source, snap.duration_ms, snap.sample_rate, snap.transport.position_ms
        );
        let old = self
            .session
            .replace(session)
            .and_then(|s| s.output)
            .map(|o| o.handle);
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
        self.session
            .as_ref()
            .map(|s| (s.id.clone(), s.transport.shows_playing()))
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
                s.send(Cmd::SetSpeed(s.state.speed_pct()));
                s.send(Cmd::Seek(s.pos())); // the position may have moved since the request
                s.sync_grid();
                s.sync_loop();
            }
            _ => self.retire(Some(o.output.handle)),
        }
    }

    /// Step 2 of Play: starts. Nothing changes if the device failed to open.
    pub fn play_with(
        &mut self,
        id: &str,
        opened: Option<OpenedOutput>,
        now: Instant,
    ) -> Result<PlayerTransport, String> {
        self.session(id)?;
        self.install(opened);
        let sink = self.sink.clone();
        let s = self.session(id)?;
        if s.output.is_none() {
            return Err(OUTPUT_LOST.into());
        }
        if s.pos() >= s.pcm.frames {
            let f = s.set_pos(0);
            s.send(Cmd::Seek(f));
        }
        // A count-in precedes a start from paused or stopped, not a play that is already running,
        // when the section under the playhead has Count In.
        let pos_ms = s.pos_ms_f();
        let plan = if s.transport.shows_playing() || !s.state.tempo.count_in_at(pos_ms) {
            None
        } else {
            s.plan_at(pos_ms, s.state.speed_pct())
        };
        s.log_count_in(&plan, s.state.tempo.segment_at(pos_ms));
        s.shared
            .counting_in
            .store(plan.is_some(), Ordering::Release);
        s.run_until = None; // a plain Play ends a run
        s.transport.play();
        s.send(Cmd::Play { count_in: plan });
        s.next_pos_save = Some(now + POSITION_SAVE);
        Ok(publish(&sink, s, now))
    }

    #[allow(dead_code)] // whole-call form, used by the tests
    pub fn play(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let opened = self
            .play_request(id)?
            .map(OutputRequest::open)
            .transpose()?;
        self.play_with(id, opened, now)
    }

    pub fn pause(&mut self, id: &str, now: Instant) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        s.run_until = None;
        s.transport.pause();
        s.halt();
        s.next_pos_save = None;
        s.mark_dirty(now);
        Ok(publish(&sink, s, now))
    }

    /// Moves to `position_ms` (clamped); while playing the sound resumes 0.3 s later.
    pub fn seek(
        &mut self,
        id: &str,
        position_ms: u64,
        now: Instant,
    ) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let f = s.set_pos(s.frames_of(position_ms));
        s.transport.seek(now);
        s.send(Cmd::Seek(f));
        s.mark_dirty(now);
        eprintln!("calliope: player seek position_ms={}", s.ms_of(f));
        Ok(publish(&sink, s, now))
    }

    pub fn nudge(
        &mut self,
        id: &str,
        delta_ms: i64,
        now: Instant,
    ) -> Result<PlayerTransport, String> {
        let s = self.session(id)?;
        let cur = s.ms_of(s.pos()) as i64;
        self.seek(id, cur.saturating_add(delta_ms).max(0) as u64, now)
    }

    /// Rew/FF: an instant jump (no scrub silence), clamped.
    pub fn skip(
        &mut self,
        id: &str,
        delta_ms: i64,
        now: Instant,
    ) -> Result<PlayerTransport, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let cur = s.ms_of(s.pos()) as i64;
        let f = s.set_pos(s.frames_of(cur.saturating_add(delta_ms).max(0) as u64));
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
    /// kept, re-snaps the loops and uploads the new grid.
    pub fn set_bpm(&mut self, id: &str, bpm: f64, now: Instant) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        let mut map: TempoMap = s.state.tempo.clone();
        map.set_bpm_at(s.pos_ms_f(), bpm)?;
        s.state.apply_tempo_change(map);
        s.loop_message = None;
        s.clamp_to_floor();
        s.sync_grid();
        s.sync_loop();
        s.mark_dirty(now);
        eprintln!(
            "calliope: player bpm value={bpm} position_ms={}",
            s.ms_of(s.pos())
        );
        Ok(s.snapshot())
    }

    /// Edits the tempo sections of the open session without pausing or moving the playback:
    /// loops keep their time, the engine gets the new grid and the file is saved after the debounce.
    /// A refusal changes nothing.
    pub fn edit_sections(
        &mut self,
        id: &str,
        edit: SectionEdit,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
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
                (
                    "bpm",
                    index,
                    old.set_section_bpm(index, bpm)?,
                    String::new(),
                )
            }
            SectionEdit::Scale { index, at, factor } => {
                guard(index, at)?;
                (
                    "scale",
                    index,
                    old.scale_section(index, factor)?,
                    String::new(),
                )
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
                let (map, applied) = if index == 0 {
                    old.move_first_beat(steps, end_ms)?
                } else {
                    old.move_change(index, steps, end_ms)?
                };
                (
                    "move",
                    index,
                    map,
                    format!(" steps={steps} applied={applied}"),
                )
            }
            SectionEdit::Flags {
                index,
                at,
                metronome,
                count_in,
            } => {
                guard(index, at)?;
                (
                    "flags",
                    index,
                    old.set_section_flags(index, metronome, count_in)?,
                    String::new(),
                )
            }
            SectionEdit::Name { index, at, name } => {
                guard(index, at)?;
                (
                    "name",
                    index,
                    old.set_section_name(index, &name)?,
                    String::new(),
                )
            }
        };
        s.state.apply_tempo_change(map);
        s.loop_message = None;
        s.clamp_to_floor();
        s.sync_grid();
        s.sync_loop();
        s.mark_dirty(now);
        let segs = &s.state.tempo.segments;
        let bpms: Vec<String> = segs.iter().map(|g| g.bpm.to_string()).collect();
        let flag = |f: fn(&Segment) -> bool| {
            segs.iter()
                .map(|g| if f(g) { "1" } else { "0" })
                .collect::<Vec<_>>()
                .join(",")
        };
        eprintln!(
            "calliope: player section {kind} id={id} index={index} segments={} bpm={}{extra} metronome={} count_in={}",
            bpms.len(),
            bpms.join(","),
            flag(|g| g.meta.metronome),
            flag(|g| g.meta.count_in)
        );
        Ok(s.snapshot())
    }

    /// What the detector needs for the open track; refused without a tempo or while the source is switching.
    pub fn first_beat_input(&self, id: &str) -> Result<FirstBeatInput, String> {
        let s = match self.session.as_ref() {
            Some(s) if s.id == id => s,
            _ => return Err(format!("no player session for {id}")),
        };
        if s.state.tempo.is_empty() {
            return Err("Type a BPM first".into());
        }
        if s.switching.is_some() {
            return Err("The source is still loading".into());
        }
        let loaded = s.state.source.clone().unwrap_or_default();
        let src = crate::first_beat::analysis_source(&s.sources, &loaded)
            .ok_or_else(|| "There is no audio to analyse".to_string())?;
        Ok(FirstBeatInput {
            key: src.key.clone(),
            file: s.dir.join(&src.file),
            pcm: (src.key == loaded).then(|| s.pcm.clone()),
            ffmpeg: s.ffmpeg.clone(),
        })
    }

    /// Puts the first beat at `ms` (the found start) and marks it set; the grid, loops, floor and the saved file
    /// follow, playback is not interrupted. `Auto` re-checks `needs_first_beat()` and otherwise changes nothing.
    pub fn place_first_beat(
        &mut self,
        id: &str,
        ms: f64,
        via: FirstBeatVia,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        if via == FirstBeatVia::Auto && (s.state.read_only || !s.state.tempo.needs_first_beat()) {
            let reason = if s.state.read_only {
                "read-only"
            } else if s.state.tempo.first_beat_set() {
                "set"
            } else {
                "not-at-zero"
            };
            eprintln!("calliope: player first-beat skipped id={id} reason={reason}");
            return Ok(s.snapshot());
        }
        let end_ms = s.pcm.frames as f64 * 1000.0 / f64::from(s.pcm.rate.max(1));
        let map = s.state.tempo.place_first_beat(ms, end_ms)?;
        let start = map.segments[0].start_ms;
        s.state.apply_tempo_change(map);
        s.loop_message = None;
        s.clamp_to_floor();
        s.sync_grid();
        s.sync_loop();
        s.mark_dirty(now);
        let loaded = s.state.source.clone().unwrap_or_default();
        let source = crate::first_beat::analysis_source(&s.sources, &loaded)
            .map_or(loaded.clone(), |x| x.key.clone());
        eprintln!(
            "calliope: player first-beat id={id} via={} source={source} found_ms={ms:.1} start_ms={start:.3} clamped={}",
            if via == FirstBeatVia::Auto {
                "auto"
            } else {
                "button"
            },
            start != ms
        );
        Ok(s.snapshot())
    }

    /// Start / End of a section: an instant jump like loop Prev/Next; playback goes on, a pre-roll is cancelled.
    pub fn jump_section(
        &mut self,
        id: &str,
        index: usize,
        at: f64,
        edge: Edge,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        match s.state.tempo.segments.get(index) {
            Some(g) if (g.start_ms - at).abs() < 1e-6 => {}
            _ => return Err("The tempo sections changed; try again".to_string()),
        }
        let end_ms = s.pcm.frames as f64 * 1000.0 / f64::from(s.pcm.rate.max(1));
        let ms = s.state.tempo.section_jump_ms(index, edge, end_ms)?;
        // Start lands on the first frame of the section and End on its last frame: either way the
        // playhead stays in the section (the lookup would otherwise pick the previous or next one).
        let first =
            s.frame_at_or_after_ms(s.state.tempo.section_jump_ms(index, Edge::Start, end_ms)?);
        let f = if edge == Edge::Start {
            first
        } else {
            s.frame_at_or_after_ms(ms).saturating_sub(1).max(first)
        };
        let f = s.set_pos(f);
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

    /// The speed buttons and the typed speed; refused during a training run.
    pub fn speed(
        &mut self,
        id: &str,
        action: SpeedAction,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        if s.shared.pass.load(Ordering::Acquire) > 0 || s.run_until.is_some_and(|t| now < t) {
            return Err(TRAINING_RUNNING.into());
        }
        let pct = match action {
            SpeedAction::Step(steps) => speed::step_pct(s.state.speed_pct(), steps),
            SpeedAction::Reset => 100,
            SpeedAction::Set(p) => speed::check_pct(p)?,
        };
        s.state.set_speed_pct(pct);
        s.send(Cmd::SetSpeed(pct));
        s.mark_dirty(now);
        eprintln!("calliope: player speed id={id} percent={pct}");
        Ok(s.snapshot())
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
    pub fn loop_play_with(
        &mut self,
        id: &str,
        opened: Option<OpenedOutput>,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        self.playable_loop(id)?;
        self.session(id)?;
        self.install(opened);
        let sink = self.sink.clone();
        let s = self.session(id)?;
        if s.output.is_none() {
            return Err(OUTPUT_LOST.into());
        }
        let l = s.state.current().expect("checked");
        let (start_ms, loop_count_in) = (
            s.boundary_ms(l.start).max(s.state.tempo.play_start_ms()),
            l.count_in,
        );
        let first_pct = l
            .training
            .filter(|t| t.on && l.repeat)
            .map_or(s.state.speed_pct(), |t| t.pass_speed(0));
        let start = s.frame_of_ms_f(start_ms);
        let plan = if loop_count_in {
            s.plan_at(start_ms, first_pct)
        } else {
            None
        };
        s.loop_message = None;
        s.set_pos(start);
        s.log_count_in(&plan, None);
        s.shared
            .counting_in
            .store(plan.is_some(), Ordering::Release);
        s.shared.loop_playing.store(true, Ordering::Release);
        s.run_until = l
            .training
            .filter(|t| t.on && l.repeat)
            .map(|_| now + Duration::from_secs(1));
        s.transport.play();
        s.send(Cmd::PlayLoop { count_in: plan });
        s.next_pos_save = Some(now + POSITION_SAVE);
        publish(&sink, s, now);
        Ok(s.snapshot())
    }

    /// One Loops-section action (all but loop Play, which needs the device: `loop_play_with`).
    pub fn loop_action(
        &mut self,
        id: &str,
        action: LoopAction,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        if action == LoopAction::Play {
            let opened = self
                .loop_play_request(id)?
                .map(OutputRequest::open)
                .transpose()?;
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
                let cur = s
                    .loop_index()
                    .map_or(if delta >= 0 { -1 } else { n }, |i| i as i64);
                let to = (cur + i64::from(delta)).clamp(0, n - 1) as usize;
                let id = s.state.loops[to].id.clone();
                s.state.select(&id)?;
                let l = &s.state.loops[to];
                let f = s.frame_of_ms_f(s.boundary_ms(l.start));
                let f = s.set_pos(f);
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
            LoopAction::Flags { repeat, count_in } => {
                s.state.flags(Some(repeat), Some(count_in))?
            }
            LoopAction::Training(t) => s.state.set_training(t)?,
            LoopAction::Stop => {
                let l = s.state.current().ok_or("There is no loop selected")?;
                let f = s.frame_of_ms_f(s.boundary_ms(l.start));
                s.transport.stop();
                s.halt();
                let f = s.set_pos(f);
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
    pub fn begin_select(
        &mut self,
        id: &str,
        key: &str,
        now: Instant,
    ) -> Result<Option<SelectJob>, String> {
        let sink = self.sink.clone();
        let s = self.session(id)?;
        let entry = s
            .sources
            .iter()
            .find(|x| x.key == key)
            .cloned()
            .ok_or_else(|| format!("unknown source {key}"))?;
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
        if let Some(s) = self
            .session
            .as_mut()
            .filter(|s| s.id == job.id && s.switching == Some(job.token))
        {
            s.switching = None;
        }
    }

    /// Step 3 (under the lock): installs the decoded source at the same position in time.
    pub fn finish_select(
        &mut self,
        job: &SelectJob,
        loaded: LoadedSource,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        let s = match self.session.as_mut() {
            Some(s)
                if s.id == job.id
                    && s.switching == Some(job.token)
                    && !job.cancel.load(Ordering::Relaxed) =>
            {
                s
            }
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
        let f = s.set_pos(f);
        s.state.source = Some(job.key.clone());
        s.switching = None;
        s.select_cancel = None;
        s.mark_dirty(now);
        eprintln!(
            "calliope: player open id={} source={} duration_ms={} rate={} position_ms={}",
            s.id,
            job.key,
            s.ms_of(s.pcm.frames),
            s.pcm.rate,
            s.ms_of(f)
        );
        Ok(s.snapshot())
    }

    /// Whole source change in one call; the IPC layer splits it to keep the manager unlocked.
    #[allow(dead_code)] // whole-call form, used by the tests
    pub fn select_source(
        &mut self,
        id: &str,
        key: &str,
        now: Instant,
    ) -> Result<PlayerSnapshot, String> {
        let Some(job) = self.begin_select(id, key, now)? else {
            return self.snapshot(id);
        };
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
            let pass = s.shared.pass.load(Ordering::Acquire);
            if pass > 0 || ended || loop_stopped {
                s.run_until = None; // `Shared.pass` tells from here on
            }
            if pass != s.last_pass {
                let loop_index = s.loop_index().or(s.last_pass_loop);
                let at = loop_index.map_or("none".to_string(), |n| n.to_string());
                if pass > 0 {
                    s.last_pass_loop = loop_index;
                    if let Some(p) = s.training_progress() {
                        let of = p.of.map_or("end".to_string(), |n| n.to_string());
                        eprintln!(
                            "calliope: player training loop={at} pass={pass} speed={} repeat={}/{of}",
                            p.speed_pct, p.repeat
                        );
                    }
                } else {
                    let at = s.last_pass_loop.map_or(at, |n| n.to_string());
                    eprintln!(
                        "calliope: player training end loop={at} speed={}",
                        s.state.speed_pct()
                    );
                }
                s.last_pass = pass;
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
                    let f = s.set_pos(0);
                    s.send(Cmd::Seek(f));
                    save_session(&sink, s);
                } else {
                    s.mark_dirty(now);
                }
                if let Some(message) = failed {
                    retired = s.output.take().map(|o| o.handle);
                    sink(PlayerEvent::AudioError {
                        id: s.id.clone(),
                        message,
                    });
                }
            } else if s.transport.tick(now) {
                s.send(Cmd::Resume);
            }
            if s.transport.shows_playing() && s.next_pos_save.is_some_and(|t| now >= t) {
                s.next_pos_save = Some(now + POSITION_SAVE);
                save_session(&sink, s);
            }
            if s.dirty_at
                .is_some_and(|t| now.saturating_duration_since(t) >= SAVE_DEBOUNCE)
            {
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
    let pcm = player_source::decode(
        &job.dir.join(&job.file),
        job.ffmpeg.as_deref(),
        &job.cancel,
        &mut progress,
    )?;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(SUPERSEDED.into());
    }
    let peaks = player_source::peaks(&pcm, PEAK_BUCKETS);
    Ok(LoadedSource { pcm, peaks })
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
