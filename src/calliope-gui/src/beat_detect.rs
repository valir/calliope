//! The beat detection job of the Editor (plan 2.8): renders the job's input (the drums, bass and
//! guitar stems at unity gain, whatever the Editor's checks, volumes and solo say; `other` when
//! there is no guitar; all stems when none of them exists; see
//! `editor::detect_stem_indices`) into a `.calliope-beats-*.part` file, uploads it to the edge-AI
//! server, waits for the tracker, turns the beats into a tempo map and stores it through the `TempoStore`.
//! The job runs on its own thread and never touches the manager; it leaves its outcome in
//! `DetectShared`, from which `EditorManager::tick` emits the one final event.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use calliope_lib::stems_api::JobState;
use calliope_lib::stems_client::{ClientError, StemsClient};
use serde::Serialize;

use crate::backing_render::{self, RenderInput, Throttle};
use crate::editor::{EditorEvent, EventSink};
use crate::media;
use crate::tempo_detect;
use crate::tempo_map::TempoMap;
use crate::tempo_store::TempoStore;

/// Minimum time between progress events of one phase.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
pub const NO_BEATS_TASK: &str = "The edge-AI server cannot detect beats yet; update calliope-stems on the server (see its README)";
pub const STALLED: &str = "The edge-AI server stopped answering; try again";
pub const TOO_LONG: &str = "Tracks longer than 15 minutes can't be analysed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectPhase {
    Checking,
    Mixing,
    Uploading,
    Waiting,
    Detecting,
    Applying,
}

impl DetectPhase {
    pub fn name(self) -> &'static str {
        match self {
            DetectPhase::Checking => "checking",
            DetectPhase::Mixing => "mixing",
            DetectPhase::Uploading => "uploading",
            DetectPhase::Waiting => "waiting",
            DetectPhase::Detecting => "detecting",
            DetectPhase::Applying => "applying",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct DetectState {
    pub phase: DetectPhase,
    pub progress: Option<f32>,
}

/// What the app hands to `EditorManager::start_detect`.
#[derive(Clone)]
pub struct DetectParams {
    /// The edge-AI server URL from the settings (empty = not set).
    pub url: String,
    pub store: Arc<dyn TempoStore>,
    /// Status poll interval (`ipc::IMPORT_POLL` in the app).
    pub poll: Duration,
    /// No change for this long ends the wait (`STALL_TIMEOUT` in the app).
    pub stall: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DetectOutcome {
    Done {
        tempo: TempoMap,
        beats: usize,
        replaced: bool,
    },
    Failed(String),
    Cancelled,
}

/// State shared between the job thread and the manager.
pub struct DetectShared {
    pub cancel: AtomicBool,
    state: Mutex<DetectState>,
    finished: AtomicBool,
    outcome: Mutex<Option<DetectOutcome>>,
}

impl DetectShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: AtomicBool::new(false),
            state: Mutex::new(DetectState {
                phase: DetectPhase::Checking,
                progress: None,
            }),
            finished: AtomicBool::new(false),
            outcome: Mutex::new(None),
        })
    }

    pub fn state(&self) -> DetectState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn outcome(&self) -> Option<DetectOutcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Everything the job needs, copied out of the session.
pub struct DetectJob {
    pub id: String,
    pub dir: PathBuf,
    pub input: RenderInput,
    pub duration_s: f64,
    pub params: DetectParams,
}

/// Removes the part file on every path (also a panic).
struct PartGuard(PathBuf);

impl Drop for PartGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

enum Stop {
    Cancelled,
    Failed(String),
}

type R<T> = Result<T, Stop>;

fn client_stop(e: ClientError, url: &str) -> Stop {
    match e {
        ClientError::Cancelled => Stop::Cancelled,
        ClientError::Unreachable(_) => {
            Stop::Failed(format!("Cannot reach the edge-AI server at {url}"))
        }
        ClientError::Stalled => Stop::Failed(STALLED.into()),
        ClientError::Rejected {
            status: 413,
            message,
        } => Stop::Failed(format!("The server refused the mix: {message}")),
        other => Stop::Failed(other.to_string()),
    }
}

/// Starts the job on its own thread. After `shared` shows it finished, the outcome is set.
pub fn spawn(job: DetectJob, shared: Arc<DetectShared>, sink: EventSink) -> JoinHandle<()> {
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
            Ok((tempo, beats, replaced)) => DetectOutcome::Done {
                tempo,
                beats,
                replaced,
            },
            Err(Stop::Cancelled) => DetectOutcome::Cancelled,
            Err(Stop::Failed(m)) => DetectOutcome::Failed(m),
        };
        drop(_guard);
        *shared.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        shared.finished.store(true, Ordering::Release);
    })
}

struct Reporter<'a> {
    id: &'a str,
    shared: &'a DetectShared,
    sink: &'a EventSink,
    throttle: Throttle,
}

impl Reporter<'_> {
    fn emit(&self, state: DetectState) {
        *self.shared.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
        (self.sink)(EditorEvent::Detect {
            id: self.id.to_string(),
            state,
        });
    }

    fn phase(&mut self, phase: DetectPhase, progress: Option<f32>) {
        eprintln!(
            "calliope: editor detect id={} phase={}",
            self.id,
            phase.name()
        );
        self.throttle = Throttle::new(PROGRESS_INTERVAL);
        self.throttle.allow(Instant::now());
        self.emit(DetectState { phase, progress });
    }

    /// A progress update inside a phase, at most every `PROGRESS_INTERVAL`.
    fn progress(&mut self, phase: DetectPhase, progress: Option<f32>) {
        if self.throttle.allow(Instant::now()) {
            self.emit(DetectState { phase, progress });
        }
    }
}

fn run(
    job: &DetectJob,
    shared: &DetectShared,
    sink: &EventSink,
    part: &Path,
) -> R<(TempoMap, usize, bool)> {
    let p = &job.params;
    let mut server_job: Option<String> = None;
    let client = StemsClient::new(&p.url);
    let result = run_inner(job, shared, sink, part, &client, &mut server_job);
    if let Some(sid) = server_job {
        client.delete(&sid);
    }
    result
}

fn run_inner(
    job: &DetectJob,
    shared: &DetectShared,
    sink: &EventSink,
    part: &Path,
    client: &StemsClient,
    server_job: &mut Option<String>,
) -> R<(TempoMap, usize, bool)> {
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
        id: &job.id,
        shared,
        sink,
        throttle: Throttle::new(PROGRESS_INTERVAL),
    };

    rep.phase(DetectPhase::Checking, None);
    let health = client.health().map_err(|e| client_stop(e, url))?;
    if !StemsClient::supports_beats(&health) {
        return Err(Stop::Failed(NO_BEATS_TASK.into()));
    }
    check_cancel()?;

    rep.phase(DetectPhase::Mixing, Some(0.0));
    backing_render::render_flac(&job.input, part, cancel, &mut |f| {
        rep.progress(DetectPhase::Mixing, Some(f))
    })
    .map_err(|m| {
        if m == backing_render::CANCELLED {
            Stop::Cancelled
        } else {
            Stop::Failed(m)
        }
    })?;
    check_cancel()?;

    rep.phase(DetectPhase::Uploading, Some(0.0));
    let sent = client.submit_beats(part, cancel, |sent, total| {
        rep.progress(
            DetectPhase::Uploading,
            Some(sent as f32 / total.max(1) as f32),
        );
    });
    let _ = fs::remove_file(part);
    let id = sent.map_err(|e| client_stop(e, url))?;
    *server_job = Some(id.clone());
    check_cancel()?;

    rep.phase(DetectPhase::Waiting, None);
    let mut last: Option<JobState> = None;
    let st = client
        .wait_final(&id, cancel, p.poll, p.stall, |st| {
            if st.state == JobState::Running {
                let progress = st.progress.map(|x| x as f32);
                if last != Some(JobState::Running) {
                    rep.phase(DetectPhase::Detecting, progress);
                } else {
                    rep.progress(DetectPhase::Detecting, progress);
                }
            }
            last = Some(st.state);
        })
        .map_err(|e| client_stop(e, url))?;
    match st.state {
        JobState::Done => {}
        JobState::Failed => {
            let m = st
                .error
                .map(|m| media::clean_text(&m, 300))
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "no reason given".into());
            return Err(Stop::Failed(format!(
                "Beat detection failed on the server: {m}"
            )));
        }
        _ => {
            return Err(Stop::Failed(
                "The job was cancelled on the edge-AI server".into(),
            ))
        }
    }

    // From here a cancel is too late: the job ends `Detected`.
    check_cancel()?;
    rep.phase(DetectPhase::Applying, None);
    let beats = client
        .fetch_beats(&id, job.duration_s)
        .map_err(|e| client_stop(e, url))?;
    let det = tempo_detect::detect(&beats.beats, &beats.downbeats, 4).map_err(Stop::Failed)?;
    eprintln!(
        "calliope: editor detected id={} beats={} downbeats={} bar_beats={} sigma_ms={:.1} segments={} bpm={} starts_ms={}",
        job.id,
        beats.beats.len(),
        beats.downbeats.len(),
        det.bar_beats,
        det.sigma_ms,
        det.map.segments.len(),
        join(det.map.segments.iter().map(|s| format!("{}", s.bpm))),
        join(det.map.segments.iter().map(|s| format!("{:.0}", s.start_ms))),
    );
    let replaced = AtomicBool::new(false);
    let new = det.map;
    let tempo = p
        .store
        .replace(&job.id, &job.dir, &|old| {
            replaced.store(!old.is_empty(), Ordering::Relaxed);
            Ok(new.with_play_settings_of(old))
        })
        .map_err(Stop::Failed)?;
    Ok((tempo, beats.beats.len(), replaced.load(Ordering::Relaxed)))
}

fn join(it: impl Iterator<Item = String>) -> String {
    it.collect::<Vec<_>>().join(",")
}

#[cfg(test)]
#[path = "beat_detect_tests.rs"]
mod tests;
