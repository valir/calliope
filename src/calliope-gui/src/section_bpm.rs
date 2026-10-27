//! Detect BPM of one section (plan gui-section-bpm-detect 2.2-2.4): the piece of up to 3 bars
//! from the playhead to the end of the section, its FLAC encoding from the Player's PCM, the job
//! that sends it to the edge-AI server (`POST /v1/beats`) and turns the beats into one BPM, and
//! the hint for octave errors. The job runs on its own thread and never touches the manager; it
//! leaves its outcome in `BpmShared`, from which `PlayerManager::tick` emits the final event.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use calliope_lib::stems_client::StemsClient;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use serde::Serialize;

use crate::backing_render::Throttle;
use crate::beat_detect::{
    check_beats_server, client_stop, upload_and_wait, DetectPhase, Stop, PROGRESS_INTERVAL, R,
};
use crate::player::{EventSink, PlayerEvent};
use crate::player_source::Pcm;
use crate::tempo_detect;
use crate::tempo_map::TempoMap;

/// Bars of the piece.
pub const PIECE_BARS: f64 = 3.0;
pub const TOO_SHORT: &str = "Less than 1 bar from the playhead to the section's end; move the playhead earlier in the section";
/// A result within this ratio tolerance of 2x or 0.5x the old BPM gets a hint.
pub const HINT_TOL: f64 = 0.08;

/// The piece of section `index` to measure: `(from_ms, to_ms)`. It starts at the playhead when
/// that is inside the section, else at the section's start, and is at most `PIECE_BARS` bars long
/// and never goes past the section's end (the next section's start, or `end_ms`).
pub fn piece(
    map: &TempoMap,
    index: usize,
    playhead_ms: f64,
    end_ms: f64,
) -> Result<(f64, f64), String> {
    let seg = map.segments.get(index).ok_or("No such section")?;
    let bar_ms = map.beats_per_bar as f64 * 60000.0 / seg.bpm;
    let section_end = map.segments.get(index + 1).map_or(end_ms, |s| s.start_ms);
    let from = if seg.start_ms <= playhead_ms && playhead_ms < section_end {
        playhead_ms
    } else {
        seg.start_ms
    };
    let to = (from + PIECE_BARS * bar_ms).min(section_end);
    if to - from < bar_ms - 1.0 {
        return Err(TOO_SHORT.into());
    }
    Ok((from, to))
}

/// The frames `from_f..to_f` of the piece `from_ms..to_ms` in a PCM of `frames` frames.
pub fn piece_frames(rate: u32, frames: u64, from_ms: f64, to_ms: f64) -> (u64, u64) {
    let f = |ms: f64| (ms * rate as f64 / 1000.0).round().max(0.0) as u64;
    let to = f(to_ms).min(frames);
    (f(from_ms).min(to), to)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Hint {
    Double,
    Half,
}

impl Hint {
    pub fn name(self) -> &'static str {
        match self {
            Hint::Double => "double",
            Hint::Half => "half",
        }
    }
}

/// Whether `new` looks like an octave error of `old` (about twice or half).
pub fn hint(old: f64, new: f64) -> Option<Hint> {
    let r = new / old;
    if (r - 2.0).abs() <= 2.0 * HINT_TOL {
        Some(Hint::Double)
    } else if (r - 0.5).abs() <= 0.5 * HINT_TOL {
        Some(Hint::Half)
    } else {
        None
    }
}

/// 16-bit FLAC of the frames `from_f..to_f` of `pcm` (its own rate and channels).
pub fn encode_piece(pcm: &Pcm, from_f: u64, to_f: u64) -> Result<Vec<u8>, String> {
    let ch = pcm.channels as usize;
    let (a, b) = (from_f as usize * ch, to_f as usize * ch);
    let slice = pcm
        .samples
        .get(a..b)
        .filter(|_| a < b)
        .ok_or("The piece is empty")?;
    let samples: Vec<i32> = slice.iter().map(|&s| s as i32).collect();
    let src = flacenc::source::MemSource::from_samples(&samples, ch, 16, pcm.rate as usize);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| format!("encoder: {e:?}"))?;
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size)
        .map_err(|e| format!("Cannot encode the piece: {e}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| format!("Cannot encode the piece: {e:?}"))?;
    Ok(sink.as_slice().to_vec())
}

/// What the UI knows of a running detection.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BpmDetectView {
    pub serial: u64,
    pub index: usize,
    pub at: f64,
    pub from_ms: f64,
    pub to_ms: f64,
    /// The selected source's key.
    pub source: String,
    pub phase: DetectPhase,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BpmOutcome {
    Done { bpm: f64, beats: usize },
    Failed(String),
    Cancelled,
}

/// State shared between the job thread and the manager.
pub struct BpmShared {
    pub cancel: AtomicBool,
    phase: Mutex<DetectPhase>,
    finished: AtomicBool,
    outcome: Mutex<Option<BpmOutcome>>,
}

impl BpmShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: AtomicBool::new(false),
            phase: Mutex::new(DetectPhase::Checking),
            finished: AtomicBool::new(false),
            outcome: Mutex::new(None),
        })
    }

    pub fn phase(&self) -> DetectPhase {
        *self.phase.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn outcome(&self) -> Option<BpmOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Sets the outcome, then the finished flag (the thread, and tests with no thread).
    pub(crate) fn finish(&self, o: BpmOutcome) {
        *self.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(o);
        self.finished.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct BpmParams {
    /// The edge-AI server URL from the settings (empty = not set).
    pub url: String,
    /// Status poll interval (`ipc::IMPORT_POLL` in the app).
    pub poll: Duration,
    /// No change for this long ends the wait (`STALL_TIMEOUT` in the app).
    pub stall: Duration,
}

/// Everything the job needs, copied out of the session.
pub struct BpmJob {
    pub id: String,
    pub dir: PathBuf,
    pub serial: u64,
    pub view: BpmDetectView,
    pub pcm: Arc<Pcm>,
    pub from_f: u64,
    pub to_f: u64,
    pub params: BpmParams,
}

/// Removes the part file on every path (also a panic).
struct PartGuard(PathBuf);

impl Drop for PartGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Reporter<'a> {
    job: &'a BpmJob,
    shared: &'a BpmShared,
    sink: &'a EventSink,
    throttle: Throttle,
    current: Option<DetectPhase>,
}

impl Reporter<'_> {
    fn emit(&self, phase: DetectPhase) {
        *self.shared.phase.lock().unwrap_or_else(|e| e.into_inner()) = phase;
        if !self.shared.cancel.load(Ordering::SeqCst) {
            (self.sink)(PlayerEvent::BpmDetect {
                id: self.job.id.clone(),
                detect: BpmDetectView {
                    phase,
                    ..self.job.view.clone()
                },
            });
        }
    }

    fn phase(&mut self, phase: DetectPhase) {
        eprintln!(
            "calliope: player detect id={} phase={}",
            self.job.id,
            phase.name()
        );
        self.current = Some(phase);
        self.throttle = Throttle::new(PROGRESS_INTERVAL);
        self.throttle.allow(Instant::now());
        self.emit(phase);
    }

    /// A phase when it differs from the current one, else an update at most every
    /// `PROGRESS_INTERVAL`.
    fn on(&mut self, phase: DetectPhase) {
        if self.current != Some(phase) {
            self.phase(phase);
        } else if self.throttle.allow(Instant::now()) {
            self.emit(phase);
        }
    }
}

/// Starts the job on its own thread. After `shared` shows it finished, the outcome is set.
pub fn spawn(job: BpmJob, shared: Arc<BpmShared>, sink: EventSink) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let part = job
            .dir
            .join(format!(".calliope-beats-{}.part", uuid::Uuid::now_v7()));
        let _guard = PartGuard(part.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&job, &shared, &sink, &part)
        }))
        .unwrap_or_else(|_| Err(Stop::Failed("Beat detection stopped unexpectedly".into())));
        let outcome = match result {
            Ok((bpm, beats)) => BpmOutcome::Done { bpm, beats },
            Err(Stop::Cancelled) => BpmOutcome::Cancelled,
            Err(Stop::Failed(m)) => BpmOutcome::Failed(m),
        };
        drop(_guard);
        shared.finish(outcome);
    })
}

fn run(job: &BpmJob, shared: &BpmShared, sink: &EventSink, part: &Path) -> R<(f64, usize)> {
    let mut server_job: Option<String> = None;
    let client = StemsClient::new(&job.params.url);
    let result = run_inner(job, shared, sink, part, &client, &mut server_job);
    if let Some(sid) = server_job {
        client.delete(&sid);
    }
    result
}

fn run_inner(
    job: &BpmJob,
    shared: &BpmShared,
    sink: &EventSink,
    part: &Path,
    client: &StemsClient,
    server_job: &mut Option<String>,
) -> R<(f64, usize)> {
    let p = &job.params;
    let url = p.url.as_str();
    let cancel = &shared.cancel;
    let check_cancel = || {
        if cancel.load(Ordering::SeqCst) {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    };
    let mut rep = Reporter {
        job,
        shared,
        sink,
        throttle: Throttle::new(PROGRESS_INTERVAL),
        current: None,
    };

    rep.phase(DetectPhase::Checking);
    check_beats_server(client, url)?;
    check_cancel()?;

    let bytes = encode_piece(&job.pcm, job.from_f, job.to_f).map_err(Stop::Failed)?;
    let write = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(part)?;
        f.write_all(&bytes)?;
        f.sync_all()
    };
    write().map_err(|e| Stop::Failed(format!("Cannot write {}: {e}", part.display())))?;
    check_cancel()?;

    let id = upload_and_wait(
        client,
        url,
        part,
        cancel,
        p.poll,
        p.stall,
        server_job,
        &mut |phase, _| rep.on(phase),
    )?;

    check_cancel()?;
    let duration_s = (job.to_f - job.from_f) as f64 / job.pcm.rate as f64;
    let beats = client
        .fetch_beats(&id, duration_s)
        .map_err(|e| client_stop(e, url))?;
    let piece = tempo_detect::piece_bpm(&beats.beats).map_err(Stop::Failed)?;
    Ok((piece.bpm, piece.beats))
}

#[cfg(test)]
#[path = "section_bpm_tests.rs"]
mod tests;
