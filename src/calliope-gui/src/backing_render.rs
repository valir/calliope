//! Offline render of the checked stems into a FLAC backing and the save job (plan 2.4, 2.6).
//! The render uses the same `mixer` functions as playback, at full stem precision.

#![allow(dead_code)] // used by the IPC layer (task 10) and the tests

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use flacenc::component::BitRepr;
use flacenc::error::{SourceError, SourceErrorReason, Verify};
use flacenc::source::{Fill, Source};
use serde_json::Value;

use crate::editor::{EditorEvent, EventSink, JobKind, VariantTarget};
use crate::import_job::{locked, RepoLock};
use crate::loudness::{self, Meter, Purpose};
use crate::mixer;
use crate::repository::{BackingSave, Repository};
use crate::stem_audio::StemReader;
use crate::track_meta::{StemEntry, TrackMeta};

/// Error text of a cancelled render.
pub const CANCELLED: &str = "cancelled";
/// Minimum time between `saving` events.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// One stem to mix: the file and its linear gain (see `mixer::gain_factor`).
#[derive(Debug, Clone)]
pub struct RenderStem {
    pub path: PathBuf,
    pub gain: f32,
}

#[derive(Debug, Clone)]
pub struct RenderInput {
    /// Only the checked stems.
    pub stems: Vec<RenderStem>,
    pub sample_rate: u32,
    /// 16 or 24.
    pub bits: u32,
    /// Length of the output (the longest stem of the track).
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderStats {
    pub clipped_samples: u64,
    /// The STREAMINFO MD5 of the written audio.
    pub md5: [u8; 16],
}

struct MixSource<'a> {
    readers: Vec<(StemReader, f32)>,
    scratch: Vec<Vec<f32>>,
    mixed: Vec<f32>,
    pcm: Vec<i32>,
    bits: u32,
    rate: u32,
    total: u64,
    done: u64,
    clipped: u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(f32),
    error: Option<String>,
    cancelled: bool,
}

impl Source for MixSource<'_> {
    fn channels(&self) -> usize {
        2
    }
    fn bits_per_sample(&self) -> usize {
        self.bits as usize
    }
    fn sample_rate(&self) -> usize {
        self.rate as usize
    }
    fn len_hint(&self) -> Option<usize> {
        Some(self.total as usize)
    }
    fn read_samples<F: Fill>(
        &mut self,
        block_size: usize,
        dest: &mut F,
    ) -> Result<usize, SourceError> {
        if self.cancel.load(Ordering::Relaxed) {
            self.cancelled = true;
            return Err(SourceError::by_reason(SourceErrorReason::IO(None)));
        }
        let n = (self.total - self.done).min(block_size as u64) as usize;
        if n == 0 {
            return Ok(0);
        }
        for (reader, scratch) in self
            .readers
            .iter_mut()
            .map(|(r, _)| r)
            .zip(self.scratch.iter_mut())
        {
            scratch.resize(n * 2, 0.0);
            if let Err(e) = reader.read(scratch, n) {
                self.error = Some(e);
                return Err(SourceError::by_reason(SourceErrorReason::IO(None)));
            }
        }
        let gains: Vec<f32> = self.readers.iter().map(|(_, g)| *g).collect();
        let inputs: Vec<&[f32]> = self.scratch.iter().map(|s| &s[..n * 2]).collect();
        self.mixed.resize(n * 2, 0.0);
        self.clipped += u64::from(mixer::mix(&inputs, &gains, &mut self.mixed));
        self.pcm.clear();
        self.pcm
            .extend(self.mixed.iter().map(|&x| mixer::quantise(x, self.bits)));
        dest.fill_interleaved(&self.pcm)?;
        self.done += n as u64;
        (self.progress)((self.done as f32 / self.total as f32).min(0.999));
        Ok(n)
    }
}

/// Mixes, encodes and writes the FLAC to `part` (`create_new`, fsynced). `progress` gets
/// every block (below 1.0) and a final 1.0 once the file is written. The part file is
/// removed on every failure and on cancel (`Err(CANCELLED)`).
pub fn render_flac(
    input: &RenderInput,
    part: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<RenderStats, String> {
    let result = render_inner(input, part, cancel, progress);
    if result.is_err() {
        let _ = fs::remove_file(part);
    }
    result
}

/// Opens the stems that are audible (gain not 0), checking that they still match the session.
fn open_readers(input: &RenderInput) -> Result<Vec<(StemReader, f32)>, String> {
    let mut readers = Vec::new();
    for s in &input.stems {
        if s.gain != 0.0 {
            let reader = StemReader::open(&s.path)?;
            // The file may have been replaced since the session was opened.
            if reader.sample_rate() != input.sample_rate {
                return Err(format!(
                    "Stem {} changed on disk: it has {} Hz, the session uses {} Hz. Reopen the track.",
                    reader.label(),
                    reader.sample_rate(),
                    input.sample_rate
                ));
            }
            if reader.bits() > 16 && input.bits == 16 {
                return Err(format!(
                    "Stem {} changed on disk: it has {} bits per sample, the session uses 16. Reopen the track.",
                    reader.label(),
                    reader.bits()
                ));
            }
            readers.push((reader, s.gain));
        }
    }
    Ok(readers)
}

/// Frames per block of the measuring pass.
const MEASURE_BLOCK: usize = 4096;

/// Pass 1 of a save and the measuring step of a normalize: the loudness and true peak of the
/// unclipped mix of `input` (plan 2.4). `progress` gets `done / total` per block (below 1.0);
/// a set `cancel` returns `Err(CANCELLED)`.
pub fn measure_mix(
    input: &RenderInput,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<loudness::Measure, String> {
    if input.frames == 0 {
        return Err("The stems are empty".into());
    }
    let mut readers = open_readers(input)?;
    let mut meter = Meter::new(input.sample_rate)?;
    let gains: Vec<f32> = readers.iter().map(|(_, g)| *g).collect();
    let mut scratch: Vec<Vec<f32>> = vec![Vec::new(); readers.len()];
    let mut mixed = Vec::new();
    let mut done = 0u64;
    while done < input.frames {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let n = (input.frames - done).min(MEASURE_BLOCK as u64) as usize;
        for ((reader, _), buf) in readers.iter_mut().zip(scratch.iter_mut()) {
            buf.resize(n * 2, 0.0);
            reader.read(buf, n)?;
        }
        let inputs: Vec<&[f32]> = scratch.iter().map(|b| &b[..n * 2]).collect();
        mixed.resize(n * 2, 0.0);
        mixer::sum(&inputs, &gains, &mut mixed);
        meter.add(&mixed)?;
        done += n as u64;
        progress((done as f32 / input.frames as f32).min(0.999));
    }
    Ok(meter.finish())
}

fn render_inner(
    input: &RenderInput,
    part: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<RenderStats, String> {
    if input.frames == 0 {
        return Err("The stems are empty".into());
    }
    if !matches!(input.bits, 16 | 24) {
        return Err(format!("unsupported output depth {}", input.bits));
    }
    let readers = open_readers(input)?;
    let scratch = vec![Vec::new(); readers.len()];
    let mut src = MixSource {
        readers,
        scratch,
        mixed: Vec::new(),
        pcm: Vec::new(),
        bits: input.bits,
        rate: input.sample_rate,
        total: input.frames,
        done: 0,
        clipped: 0,
        cancel,
        progress,
        error: None,
        cancelled: false,
    };
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| format!("encoder: {e:?}"))?;
    let stream = flacenc::encode_with_fixed_block_size(&config, &mut src, config.block_size);
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            return Err(if src.cancelled {
                CANCELLED.into()
            } else if let Some(msg) = src.error.take() {
                msg
            } else {
                format!("Cannot encode the backing: {e}")
            });
        }
    };
    let clipped_samples = src.clipped;
    let md5 = *stream.stream_info().md5_digest();
    let progress = src.progress;
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| format!("Cannot encode the backing: {e:?}"))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)
        .map_err(|e| format!("Cannot create {}: {e}", part.display()))?;
    file.write_all(sink.as_slice())
        .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
    file.sync_all()
        .map_err(|e| format!("Cannot write {}: {e}", part.display()))?;
    progress(1.0);
    Ok(RenderStats {
        clipped_samples,
        md5,
    })
}

/// Lets a progress event through at most every `interval`.
pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) >= self.interval)
        {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

// ------------------------------------------------------------------- save job

/// State shared between the job thread and the manager.
pub struct SaveShared {
    pub cancel: AtomicBool,
    progress: AtomicU32,
    pub(crate) finished: AtomicBool,
    /// The variant that was written; `None` after a failure or cancel.
    outcome: Mutex<Option<VariantTarget>>,
}

impl SaveShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: AtomicBool::new(false),
            progress: AtomicU32::new(0f32.to_bits()),
            finished: AtomicBool::new(false),
            outcome: Mutex::new(None),
        })
    }

    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn outcome(&self) -> Option<VariantTarget> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Runs `f` while no Player session holds the track (plan 2.8).
pub trait PlayerGate: Send + Sync {
    /// An open session is flushed (when writable) and closed first, and its `closed` event makes
    /// the Player view reopen it. An error closing it refuses `f`.
    fn without_player(
        &self,
        id: &str,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String>;
}

/// The default gate: there is no Player (tests); runs `f` directly.
pub struct NoPlayer;

impl PlayerGate for NoPlayer {
    fn without_player(
        &self,
        _id: &str,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        f()
    }
}

/// `"none"` or the value with two decimals (log lines).
pub(crate) fn fmt_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "none".to_string(), |v| format!("{v:.2}"))
}

/// The progress callback of a job: stores the fraction, sends a throttled `saving` event and
/// sleeps `pace` per call (test knob).
pub(crate) fn reporter<'a>(
    id: &'a str,
    job: JobKind,
    shared: &'a SaveShared,
    sink: &'a EventSink,
    pace: Option<Duration>,
) -> impl FnMut(f32) + 'a {
    let mut throttle = Throttle::new(PROGRESS_INTERVAL);
    move |fraction: f32| {
        if let Some(d) = pace.filter(|_| fraction < 1.0) {
            std::thread::sleep(d);
        }
        shared.progress.store(fraction.to_bits(), Ordering::Relaxed);
        if fraction >= 1.0 || throttle.allow(Instant::now()) {
            sink(EditorEvent::Saving {
                id: id.to_string(),
                progress: fraction,
                job,
            });
        }
    }
}

pub struct SaveParams {
    pub id: String,
    /// The session's target variant id.
    pub variant: String,
    pub dir: PathBuf,
    /// The session's stems, for the repository's "same stems" check.
    pub stems: Vec<StemEntry>,
    pub input: RenderInput,
    pub mix: Value,
    pub repo: Repository,
    pub lock: Arc<dyn RepoLock>,
    pub gate: Arc<dyn PlayerGate>,
    /// Test knob: slept per progress call (every block of both passes).
    pub pace: Option<Duration>,
}

/// Starts the save job on its own thread. Exactly one final event is sent (`saved`,
/// `save-failed` or `save-cancelled`), after `shared` shows the job as finished.
pub fn spawn(params: SaveParams, shared: Arc<SaveShared>, sink: EventSink) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let id = params.id.clone();
        let part = params
            .dir
            .join(format!(".calliope-backing-{}.part", uuid::Uuid::now_v7()));
        // A panic must not leave the job "running" forever or the part file behind.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&params, &shared, &sink, &part)
        }))
        .unwrap_or_else(|_| {
            Err((
                Some(part.clone()),
                "The save stopped unexpectedly".to_string(),
            ))
        });
        let part_cleanup = |p: &Path| {
            let _ = fs::remove_file(p);
        };
        let event = match result {
            Ok(ev) => ev,
            Err((part, msg)) => {
                if let Some(p) = part {
                    part_cleanup(&p);
                }
                if msg == CANCELLED {
                    EditorEvent::SaveCancelled {
                        id,
                        job: JobKind::Save,
                    }
                } else {
                    eprintln!("calliope: editor save failed id={id}: {msg}");
                    EditorEvent::SaveFailed {
                        id,
                        message: msg,
                        job: JobKind::Save,
                    }
                }
            }
        };
        shared.finished.store(true, Ordering::Release);
        sink(event);
    })
}

type JobError = (Option<PathBuf>, String);

fn run(
    p: &SaveParams,
    shared: &SaveShared,
    sink: &EventSink,
    part: &Path,
) -> Result<EditorEvent, JobError> {
    let mut report = reporter(&p.id, JobKind::Save, shared, sink, p.pace);
    let fail = |msg: String| (Some(part.to_path_buf()), msg);
    // Pass 1: measure the unclipped mix, then decide the gain.
    let m =
        measure_mix(&p.input, &shared.cancel, &mut |f| report(f * 0.4)).map_err(|e| (None, e))?;
    let d = loudness::decide(m, Purpose::Save);
    eprintln!(
        "calliope: editor measured id={} job=save lufs={} tp={}",
        p.id,
        fmt_opt(m.lufs),
        fmt_opt(m.true_peak)
    );
    // Pass 2: render with the gain folded into the stem gains.
    let g = loudness::db_to_gain(d.gain_db);
    let mut input = p.input.clone();
    for s in &mut input.stems {
        s.gain *= g;
    }
    let stats = render_flac(&input, part, &shared.cancel, &mut |f| report(0.4 + f * 0.6))
        .map_err(|e| (None, e))?;
    if shared.cancel.load(Ordering::Relaxed) {
        return Err(fail(CANCELLED.into()));
    }
    let mut saved = None;
    p.gate
        .without_player(&p.id, &mut || {
            locked(&*p.lock, || {
                saved = Some(p.repo.save_backing(BackingSave {
                    id: &p.id,
                    variant: Some(&p.variant),
                    stems: &p.stems,
                    part,
                    mix: p.mix.clone(),
                    sample_rate: p.input.sample_rate,
                    bits: p.input.bits,
                    loudness: Some(loudness::record_of(m, d, stats.md5)),
                }));
            });
            Ok(())
        })
        .map_err(fail)?;
    let result = saved
        .expect("the repository lock ran the closure")
        .map_err(fail)?;
    let res = loudness::result_of(m, d);
    let target = written_variant(&p.dir, &p.variant);
    let file = target.file.clone();
    *shared.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(target);
    eprintln!(
        "calliope: editor saved id={} file={file} bits={} clipped={} outcome={} lufs={} tp={} gain={:.2}",
        p.id,
        p.input.bits,
        stats.clipped_samples,
        res.outcome.as_str(),
        fmt_opt(res.lufs),
        fmt_opt(res.true_peak),
        res.gain_db
    );
    Ok(EditorEvent::Saved {
        id: p.id.clone(),
        file,
        clipped_samples: stats.clipped_samples,
        loudness: res,
        track: result.track,
        warnings: result.warnings,
    })
}

/// The variant the repository wrote: the requested id if listed, else the newest entry.
fn written_variant(dir: &Path, wanted: &str) -> VariantTarget {
    let meta: Option<TrackMeta> = fs::read(dir.join("track.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let found = meta.and_then(|m| {
        let i = m
            .backings
            .iter()
            .position(|b| b.id == wanted)
            .or(m.backings.len().checked_sub(1))?;
        Some(m.backings[i].clone())
    });
    match found {
        Some(b) => VariantTarget {
            id: b.id,
            name: b.name,
            file: b.file,
            exists: true,
        },
        None => VariantTarget {
            id: wanted.into(),
            name: "Backing".into(),
            file: format!("backings/{wanted}.flac"),
            exists: true,
        },
    }
}

#[cfg(test)]
#[path = "backing_render_tests.rs"]
mod tests;
