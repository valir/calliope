//! The import job orchestrator (plan section 2.3): one job at a time, run on its own thread.
//!
//! ```text
//! url:   download (or resume) -> preparing (probe, FLAC, 15 min) -> ready(metadata)
//! file:  preparing (probe, FLAC, 15 min)                          -> ready(metadata)
//! ready/failed(server|save) --start_extraction--> uploading -> queued -> working
//!        -> receiving -> saving -> saved          (the temp folder is removed)
//! ```
//!
//! Cancel: during a download the `.part` is kept (back to the source page); during preparing
//! the partial output is removed (source page; a local-file job's folder is removed too); during
//! the extraction the server job is deleted, the staging folder removed and the job is back in
//! `ready` (the edit pane) with its audio and metadata, so Extract can be retried. A failed
//! extraction also keeps the job (`failed`, retriable); a failed download or preparation ends it.
//!
//! Data safety: user source files are only read; every temp/staging removal goes through
//! `ImportTmp::remove_job_dir` / `Staging::abandon` (Calliope-marked folders only); a track
//! becomes visible by one rename inside `Staging::commit`, run under the repository lock.
//! No Tauri here: the Tauri layer gives it a sink for events and a [`RepoLock`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use calliope_lib::stems_api::JobState;
use calliope_lib::stems_client::{ClientError, StemsClient, STALL_TIMEOUT};
use serde::Serialize;

use crate::download::{self, DownloadError};
use crate::import_tmp::{self, ImportTmp};
use crate::media::{self, Cancel, MediaError, Probe};
use crate::repository::{self, RepoStatus, Repository, Staging, TrackRecord, ORIGINAL_NAME};
use crate::tools::{ToolName, Tools};
use crate::track_meta::{self, StemEntry, TrackEdits, TrackMeta, TrackType};

pub const BUSY_MESSAGE: &str = "An import is already running";
pub const NOT_CONFIGURED_MESSAGE: &str =
    "The edge-AI server address is not set. Check Settings > Stem extraction.";
const ROOT_MISSING_MESSAGE: &str = "The track repository folder is missing. Check Settings > Track repository.";
/// At most this many progress events per second reach the sink.
pub const MAX_EVENTS_PER_SECOND: u32 = 10;
const MAX_INFO_BYTES: u64 = 8 << 20;

// ---- public data ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Downloading,
    Preparing,
    Ready,
    Uploading,
    Queued,
    Working,
    Receiving,
    Saving,
    Saved,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Download,
    Prepare,
    Server,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackTo {
    Source,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SourceKind {
    #[serde(rename = "url")]
    Url,
    #[serde(rename = "audio-file")]
    AudioFile,
    #[serde(rename = "video-file")]
    VideoFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Audio,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Source {
    pub kind: SourceKind,
    /// The URL or the bare file name.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobError {
    pub stage: Stage,
    pub message: String,
    pub http_status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JobSnapshot {
    pub job: String,
    pub source: Source,
    pub phase: Phase,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub sent: u64,
    pub stems_done: u32,
    pub stems_total: u32,
    pub progress: Option<f64>,
    pub duration_s: Option<f64>,
    pub metadata: Option<TrackEdits>,
    pub error: Option<JobError>,
    pub track: Option<TrackRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)] // short-lived values, serialised at once
pub enum ImportEvent {
    Downloading { downloaded: u64, total: Option<u64> },
    Preparing,
    Ready { metadata: TrackEdits, duration_s: Option<f64> },
    Uploading { sent: u64, total: u64 },
    Queued,
    Working { progress: Option<f64> },
    Receiving { done: u32, total: u32 },
    Saving,
    Saved { track: TrackRecord },
    Failed { stage: Stage, message: String, http_status: Option<u16> },
    Cancelled { back_to: BackTo },
}

impl ImportEvent {
    /// Progress updates are throttled; phase changes are always delivered.
    #[allow(dead_code)] // the throttle tests use it
    fn is_progress(&self) -> bool {
        matches!(
            self,
            ImportEvent::Downloading { .. }
                | ImportEvent::Uploading { .. }
                | ImportEvent::Working { .. }
                | ImportEvent::Receiving { .. }
        )
    }
}

pub type Sink = Arc<dyn Fn(ImportEvent) + Send + Sync>;

/// Runs `f` while holding the repository lock (`RepoState`'s mutex in the app).
pub trait RepoLock: Send + Sync {
    fn run(&self, f: &mut dyn FnMut());
}

impl<F: Fn(&mut dyn FnMut()) + Send + Sync> RepoLock for F {
    fn run(&self, f: &mut dyn FnMut()) {
        self(f)
    }
}

fn locked<T>(lock: &dyn RepoLock, f: impl FnOnce() -> T) -> T {
    let mut f = Some(f);
    let mut out = None;
    lock.run(&mut || {
        if let Some(f) = f.take() {
            out = Some(f());
        }
    });
    out.expect("the repository lock did not run the closure")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UrlPrepStatus {
    Invalid,
    Ready,
    Partial,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UrlPrep {
    pub status: UrlPrepStatus,
    pub message: String,
    pub partial_bytes: u64,
}

// ---- throttle ----

/// Lets at most `MAX_EVENTS_PER_SECOND` progress events through.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    pub fn allow(&mut self, now: Instant) -> bool {
        let gap = Duration::from_millis(1000 / MAX_EVENTS_PER_SECOND as u64);
        match self.last {
            Some(t) if now.saturating_duration_since(t) < gap => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

// ---- state ----

struct Job {
    snap: JobSnapshot,
    dir: PathBuf,
    audio: Option<PathBuf>,
    root: PathBuf,
    #[allow(dead_code)]
    kind: SourceKind,
    /// A thread is working on it.
    running: bool,
    /// Finished for good (saved, cancelled to the source page, failed before `ready`).
    over: bool,
    cancel: Option<Cancel>,
    handle: Option<JoinHandle<()>>,
}

struct Inner {
    job: Mutex<Option<Job>>,
    sink: Mutex<Option<Sink>>,
    throttle: Mutex<Throttle>,
    tools: Mutex<Tools>,
    /// Looks the tools up again (the app re-reads PATH, so a tool installed while Calliope runs
    /// is found without a restart). `None` keeps the tools given to `new` (tests).
    rediscover: Option<Box<dyn Fn() -> Tools + Send + Sync>>,
    lock: Arc<dyn RepoLock>,
    poll: Duration,
}

impl Inner {
    fn current_tools(&self) -> Tools {
        if let Some(f) = &self.rediscover {
            let fresh = f();
            *guard(&self.tools) = fresh;
        }
        guard(&self.tools).clone()
    }
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct ImportState {
    inner: Arc<Inner>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn check_tools(tools: &Tools, needed: &[ToolName]) -> Result<(), String> {
    for &t in needed {
        let st = tools.check_one(t);
        if !st.usable() {
            return Err(st.message.unwrap_or_else(|| t.missing_message()));
        }
    }
    Ok(())
}

/// The root must exist and be an `ok` or `empty` repository; an empty one gets its layout.
fn check_root(root: &Path, lock: &dyn RepoLock) -> Result<(), String> {
    match repository::status(root) {
        RepoStatus::Ok => Ok(()),
        RepoStatus::Empty => locked(lock, || Repository::new(root).ensure_layout()),
        _ => Err(ROOT_MISSING_MESSAGE.to_string()),
    }
}

fn new_snapshot(job: String, kind: SourceKind, label: String, phase: Phase) -> JobSnapshot {
    JobSnapshot {
        job,
        source: Source { kind, label },
        phase,
        downloaded: 0,
        total: None,
        sent: 0,
        stems_done: 0,
        stems_total: 0,
        progress: None,
        duration_s: None,
        metadata: None,
        error: None,
        track: None,
    }
}

/// Builds the strictly validated metadata of a stem track (also used with a placeholder stem
/// list to validate the user's edits before anything starts).
fn build_meta(id: &str, edits: &TrackEdits, stems: &[String], model: &str, keep: bool) -> Result<TrackMeta, String> {
    let now = track_meta::now_rfc3339();
    let mut meta = TrackMeta {
        schema_version: track_meta::CURRENT_SCHEMA,
        id: id.to_string(),
        track_type: TrackType::Stem,
        band: String::new(),
        album: String::new(),
        title: "x".into(),
        composers: Vec::new(),
        year: None,
        source_url: None,
        copyright: None,
        audio: None,
        original: keep.then(|| ORIGINAL_NAME.to_string()),
        stems: stems.iter().map(|n| StemEntry { name: n.clone(), file: format!("stems/{n}.flac") }).collect(),
        stem_model: Some(model.to_string()),
        tablatures: Vec::new(),
        imported: now.clone(),
        modified: now,
        extra: serde_json::Map::new(),
    };
    track_meta::apply_edits(&mut meta, edits.clone())?;
    track_meta::validate_for_write(&meta)?;
    Ok(meta)
}

impl ImportState {
    /// `poll` is the interval at which the edge-AI job is polled (1 s in the app).
    pub fn new(tools: Tools, lock: Arc<dyn RepoLock>, poll: Duration) -> Self {
        ImportState {
            inner: Arc::new(Inner {
                job: Mutex::new(None),
                sink: Mutex::new(None),
                throttle: Mutex::new(Throttle::default()),
                tools: Mutex::new(tools),
                rediscover: None,
                lock,
                poll,
            }),
        }
    }

    /// Makes every `tools()` call and every import start look the tools up again with `f`.
    pub fn with_rediscovery(mut self, f: impl Fn() -> Tools + Send + Sync + 'static) -> Self {
        Arc::get_mut(&mut self.inner).expect("with_rediscovery before sharing").rediscover = Some(Box::new(f));
        self
    }

    /// The current tools, looked up again when rediscovery is set.
    pub fn tools(&self) -> Tools {
        self.inner.current_tools()
    }

    pub fn snapshot(&self) -> Option<JobSnapshot> {
        guard(&self.inner.job).as_ref().map(|j| j.snap.clone())
    }

    /// Sends later events to `sink` (a reloaded webview attaches a new channel).
    pub fn replace_sink(&self, sink: Sink) {
        *guard(&self.inner.sink) = Some(sink);
    }

    /// A job exists that is not over (running, or waiting in the edit pane). Changing the
    /// repository root is refused while this is true.
    pub fn is_active(&self) -> bool {
        guard(&self.inner.job).as_ref().is_some_and(|j| !j.over)
    }

    pub fn is_running(&self) -> bool {
        guard(&self.inner.job).as_ref().is_some_and(|j| j.running)
    }

    /// The state of a URL for the resume prompt (nothing is created or changed).
    pub fn prepare_url(&self, root: &Path, input: &str) -> UrlPrep {
        let prep = |status, message: &str, partial_bytes| UrlPrep { status, message: message.to_string(), partial_bytes };
        let url = match download::normalise(input) {
            Ok(u) => u,
            Err(m) => return prep(UrlPrepStatus::Invalid, &m, 0),
        };
        if self.is_active() {
            return prep(UrlPrepStatus::Busy, BUSY_MESSAGE, 0);
        }
        let partial = ImportTmp::peek(root).and_then(|t| t.find_partial(&url)).filter(|p| p.is_resumable());
        match partial {
            Some(p) => {
                let bytes = std::fs::read_dir(&p.dir)
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| e.file_name().to_string_lossy().starts_with("download."))
                            .filter_map(|e| e.metadata().ok())
                            .map(|m| m.len())
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                prep(UrlPrepStatus::Partial, "Incomplete download file from the same URL found", bytes)
            }
            None => prep(UrlPrepStatus::Ready, "", 0),
        }
    }

    /// Starts a URL import. `resume` continues an earlier partial download; otherwise the URL's
    /// folder is cleared first (Start Over). Errors that happen before anything starts (bad URL,
    /// missing tool, repository problems, a job already active) are returned as text.
    pub fn start_url(&self, root: &Path, input: &str, resume: bool, sink: Sink) -> Result<JobSnapshot, String> {
        let url = download::normalise(input)?;
        check_tools(&self.inner.current_tools(), &[ToolName::YtDlp, ToolName::Ffmpeg, ToolName::Ffprobe])?;
        let mut slot = guard(&self.inner.job);
        if slot.as_ref().is_some_and(|j| !j.over) {
            return Err(BUSY_MESSAGE.into());
        }
        check_root(root, &*self.inner.lock)?;
        let tmp = ImportTmp::open(root)?;
        let created = track_meta::now_rfc3339();
        let dir = tmp.prepare_url_dir(&url, &created)?;
        tmp.clean_stale(now_secs(), Some(&dir));
        locked(&*self.inner.lock, || Repository::new(root).clean_stale_staging(None));
        if !resume {
            tmp.clear_for_start_over(&url, &created)?;
        }
        let id = track_meta::new_id();
        let snap = new_snapshot(id.clone(), SourceKind::Url, url.clone(), Phase::Downloading);
        self.launch(&mut slot, snap, SourceKind::Url, dir.clone(), root, tmp.clone(), sink, move |ctx| {
            let r = run_url(&ctx, &dir, &url, resume);
            finish_prepare(&ctx, r, false);
        })
    }

    /// Starts importing a local audio or video file picked by the user (read only).
    pub fn start_file(&self, root: &Path, kind: FileKind, path: &Path, sink: Sink) -> Result<JobSnapshot, String> {
        if !path.is_absolute() {
            return Err("the selected file path must be absolute".into());
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if name.is_empty() {
            return Err("the selected file has no usable name".into());
        }
        check_tools(&self.inner.current_tools(), &[ToolName::Ffmpeg, ToolName::Ffprobe])?;
        let mut slot = guard(&self.inner.job);
        if slot.as_ref().is_some_and(|j| !j.over) {
            return Err(BUSY_MESSAGE.into());
        }
        check_root(root, &*self.inner.lock)?;
        let tmp = ImportTmp::open(root)?;
        tmp.clean_stale(now_secs(), None);
        locked(&*self.inner.lock, || Repository::new(root).clean_stale_staging(None));
        let (skind, kname) = match kind {
            FileKind::Audio => (SourceKind::AudioFile, "audio-file"),
            FileKind::Video => (SourceKind::VideoFile, "video-file"),
        };
        let dir = tmp.new_file_dir(kname, &name, &track_meta::now_rfc3339())?;
        let id = track_meta::new_id();
        let snap = new_snapshot(id, skind, name.clone(), Phase::Preparing);
        let path = path.to_path_buf();
        let d2 = dir.clone();
        self.launch(&mut slot, snap, skind, dir, root, tmp, sink, move |ctx| {
            let r = run_file(&ctx, &d2, &path, &name);
            finish_prepare(&ctx, r, true);
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn launch(
        &self,
        slot: &mut Option<Job>,
        snap: JobSnapshot,
        kind: SourceKind,
        dir: PathBuf,
        root: &Path,
        tmp: ImportTmp,
        sink: Sink,
        body: impl FnOnce(Ctx) + Send + 'static,
    ) -> Result<JobSnapshot, String> {
        let cancel = Cancel::new();
        let ctx = Ctx { inner: self.inner.clone(), id: snap.job.clone(), cancel: cancel.clone(), root: root.to_path_buf(), tmp };
        *guard(&self.inner.sink) = Some(sink);
        let out = snap.clone();
        eprintln!("calliope: import job={} phase={:?}", snap.job, snap.phase);
        let handle = std::thread::Builder::new()
            .name("import-job".into())
            .spawn(move || body(ctx))
            .map_err(|e| format!("cannot start the import: {e}"))?;
        *slot = Some(Job {
            snap,
            dir,
            audio: None,
            root: root.to_path_buf(),
            kind,
            running: true,
            over: false,
            cancel: Some(cancel),
            handle: Some(handle),
        });
        Ok(out)
    }

    /// Starts the extraction of a `ready` (or retriable `failed`) job. `edits` are validated
    /// strictly first; nothing starts on an error.
    pub fn start_extraction(
        &self,
        job: &str,
        edits: TrackEdits,
        edge_ai_url: Option<&str>,
        keep_original: bool,
    ) -> Result<JobSnapshot, String> {
        build_meta("validation", &edits, &["vocals".to_string()], "model", keep_original)?;
        let url = edge_ai_url.map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| NOT_CONFIGURED_MESSAGE.to_string())?;
        let url = url.to_string();
        let mut slot = guard(&self.inner.job);
        let j = slot.as_mut().filter(|j| j.snap.job == job).ok_or("Unknown import job")?;
        let retriable = j.snap.phase == Phase::Failed && !j.over;
        if j.running || j.over || !(j.snap.phase == Phase::Ready || retriable) {
            return Err("This import is not ready for extraction".into());
        }
        let audio = j.audio.clone().filter(|a| a.is_file()).ok_or("The prepared audio is missing; start the import again")?;
        // all fallible work first: an early error must leave the job untouched
        let tmp = ImportTmp::open(&j.root)?;
        let cancel = Cancel::new();
        j.cancel = Some(cancel.clone());
        j.running = true;
        j.snap.error = None;
        j.snap.metadata = Some(edits.clone());
        j.snap.phase = Phase::Uploading;
        j.snap.sent = 0;
        j.snap.total = None;
        j.snap.progress = None;
        j.snap.stems_done = 0;
        j.snap.stems_total = 0;
        let out = j.snap.clone();
        let ctx = Ctx { inner: self.inner.clone(), id: j.snap.job.clone(), cancel, root: j.root.clone(), tmp };
        let dir = j.dir.clone();
        eprintln!("calliope: import job={} phase=uploading", job);
        let spawned = std::thread::Builder::new().name("import-job".into()).spawn(move || {
            ctx.emit(ImportEvent::Uploading { sent: 0, total: 0 });
            let mut staging: Option<Staging> = None;
            let mut server_job: Option<String> = None;
            let r = run_extract(&ctx, &dir, &audio, &edits, &url, keep_original, &mut staging, &mut server_job);
            // cleanup first, then publish the outcome
            if let Some(s) = staging.take() {
                if let Err(e) = s.abandon() {
                    eprintln!("calliope: import cleanup: {e}");
                }
            }
            if let Some(id) = server_job.take() {
                StemsClient::new(&url).delete(&id);
            }
            finish_extract(&ctx, r);
        });
        match spawned {
            Ok(h) => {
                j.handle = Some(h);
                Ok(out)
            }
            Err(e) => {
                j.running = false;
                j.snap.phase = Phase::Ready;
                Err(format!("cannot start the extraction: {e}"))
            }
        }
    }

    /// Asks the running job to stop; the outcome arrives as an event. Not running: no change.
    pub fn cancel(&self, job: &str) -> Result<JobSnapshot, String> {
        let slot = guard(&self.inner.job);
        let j = slot.as_ref().filter(|j| j.snap.job == job).ok_or("Unknown import job")?;
        if j.running {
            if let Some(c) = &j.cancel {
                c.cancel();
            }
        }
        Ok(j.snap.clone())
    }

    /// Removes the job's temp folder and forgets the job (the edit pane's Cancel, "Discard").
    pub fn discard(&self, job: &str) -> Result<(), String> {
        let mut slot = guard(&self.inner.job);
        let j = slot.as_ref().filter(|j| j.snap.job == job).ok_or("Unknown import job")?;
        if j.running {
            return Err("The import is still running; cancel it first".into());
        }
        if j.dir.exists() {
            ImportTmp::open(&j.root)?.remove_job_dir(&j.dir)?;
        }
        *slot = None;
        Ok(())
    }

    /// App exit: cancels the running job (child processes are killed, the server job is
    /// deleted best effort) and waits up to `timeout` for the thread. True if nothing is left
    /// running.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let handle = {
            let mut slot = guard(&self.inner.job);
            let Some(j) = slot.as_mut().filter(|j| j.running) else { return true };
            if let Some(c) = &j.cancel {
                c.cancel();
            }
            j.handle.take()
        };
        let deadline = Instant::now() + timeout;
        let Some(handle) = handle else { return !self.is_running() };
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = handle.join();
        true
    }
}

// ---- the thread side ----

#[derive(Clone)]
struct Ctx {
    inner: Arc<Inner>,
    id: String,
    cancel: Cancel,
    root: PathBuf,
    tmp: ImportTmp,
}

enum Stop {
    Cancelled,
    Failed(JobError),
}

type R<T> = Result<T, Stop>;

fn fail<T>(stage: Stage, message: impl Into<String>) -> R<T> {
    Err(Stop::Failed(JobError { stage, message: message.into(), http_status: None }))
}

impl Ctx {
    fn update(&self, f: impl FnOnce(&mut Job)) {
        if let Some(j) = guard(&self.inner.job).as_mut().filter(|j| j.snap.job == self.id) {
            f(j);
        }
    }

    fn emit(&self, ev: ImportEvent) {
        let sink = guard(&self.inner.sink).clone();
        if let Some(s) = sink {
            s(ev);
        }
    }

    fn progress(&self, ev: ImportEvent) {
        if guard(&self.inner.throttle).allow(Instant::now()) {
            self.emit(ev);
        }
    }

    fn phase(&self, phase: Phase, ev: ImportEvent) {
        self.update(|j| j.snap.phase = phase);
        eprintln!("calliope: import job={} phase={:?}", self.id, phase);
        self.emit(ev);
    }

    fn check_cancel(&self) -> R<()> {
        if self.cancel.is_cancelled() {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    }

    fn tool(&self, t: ToolName) -> R<PathBuf> {
        match guard(&self.inner.tools).path_of(t) {
            Some(p) => Ok(p.to_path_buf()),
            None => fail(Stage::Prepare, t.missing_message()),
        }
    }
}

fn media_stop(e: MediaError, rewrite: Option<&str>) -> Stop {
    match e {
        MediaError::Cancelled => Stop::Cancelled,
        MediaError::Failed(m) => {
            let m = match rewrite {
                Some(name) => m.replacen(&format!("Selected file {name}"), "The downloaded file", 1),
                None => m,
            };
            Stop::Failed(JobError { stage: Stage::Prepare, message: m, http_status: None })
        }
    }
}

fn read_info(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() || m.len() > MAX_INFO_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// For URL imports the info values win and file tags fill the gaps.
fn merge_url_edits(info: Option<TrackEdits>, probe: &Probe, url: &str) -> TrackEdits {
    let tags = media::edits_from_tags(probe, "");
    let tag_title = probe.tags.get("title").map(|t| media::clean_text(t, 200)).filter(|t| !t.is_empty());
    match info {
        Some(mut e) => {
            if e.band.is_empty() {
                e.band = tags.band;
            }
            if e.album.is_empty() {
                e.album = tags.album;
            }
            if e.composers.is_empty() {
                e.composers = tags.composers;
            }
            e.year = e.year.or(tags.year);
            e.copyright = e.copyright.or(tags.copyright);
            e
        }
        None => TrackEdits {
            title: tag_title.unwrap_or_else(|| media::clean_text(url, 200)),
            source_url: Some(url.to_string()),
            ..tags
        },
    }
}

fn set_ready(ctx: &Ctx, audio: PathBuf, metadata: TrackEdits, duration_s: Option<f64>) {
    ctx.update(|j| {
        j.audio = Some(audio);
        j.snap.metadata = Some(metadata.clone());
        j.snap.duration_s = duration_s;
        j.snap.phase = Phase::Ready;
        j.running = false;
        j.cancel = None;
    });
    eprintln!("calliope: import job={} phase=Ready", ctx.id);
    ctx.emit(ImportEvent::Ready { metadata, duration_s });
}

fn run_url(ctx: &Ctx, dir: &Path, url: &str, resume: bool) -> R<()> {
    let ffprobe = ctx.tool(ToolName::Ffprobe)?;
    let ffmpeg = ctx.tool(ToolName::Ffmpeg)?;
    let partial = ctx.tmp.find_partial(url);
    let has_audio = partial.as_ref().is_some_and(|p| p.has_audio);
    let has_download = partial.as_ref().is_some_and(|p| p.has_download);
    ctx.check_cancel()?;
    let (download_file, info) = if has_audio || has_download {
        (download::find_download(dir), Some(dir.join(download::INFO_JSON)).filter(|p| p.is_file()))
    } else {
        let ytdlp = ctx.tool(ToolName::YtDlp)?;
        let c = ctx.clone();
        let dl = download::run_download(&ytdlp, dir, url, resume, &ctx.cancel, move |d, t| {
            c.update(|j| {
                j.snap.downloaded = d;
                j.snap.total = t;
            });
            c.progress(ImportEvent::Downloading { downloaded: d, total: t });
        })
        .map_err(|e| match e {
            DownloadError::Cancelled => Stop::Cancelled,
            DownloadError::Failed { message, http_status } => {
                Stop::Failed(JobError { stage: Stage::Download, message, http_status })
            }
        })?;
        (Some(dl.file), dl.info)
    };
    ctx.check_cancel()?;
    ctx.phase(Phase::Preparing, ImportEvent::Preparing);
    let audio = dir.join(import_tmp::AUDIO_FLAC);
    let src = download_file.clone().unwrap_or_else(|| audio.clone());
    let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("download").to_string();
    let probe = media::probe(&ffprobe, &src, &ctx.cancel).map_err(|e| media_stop(e, Some(&name)))?;
    media::check_duration(&probe, &name).map_err(|e| media_stop(e, Some(&name)))?;
    if !has_audio {
        media::to_flac(&ffmpeg, &src, &probe, dir, &ctx.cancel).map_err(|e| media_stop(e, Some(&name)))?;
    }
    ctx.check_cancel()?;
    let info_edits = info.as_deref().and_then(read_info).and_then(|t| download::edits_from_info(&t, url));
    let edits = merge_url_edits(info_edits, &probe, url);
    set_ready(ctx, audio, edits, probe.duration_s);
    Ok(())
}

fn run_file(ctx: &Ctx, dir: &Path, path: &Path, name: &str) -> R<()> {
    let ffprobe = ctx.tool(ToolName::Ffprobe)?;
    let ffmpeg = ctx.tool(ToolName::Ffmpeg)?;
    ctx.check_cancel()?;
    let probe = media::probe(&ffprobe, path, &ctx.cancel).map_err(|e| media_stop(e, None))?;
    media::check_duration(&probe, name).map_err(|e| media_stop(e, None))?;
    let audio = media::to_flac(&ffmpeg, path, &probe, dir, &ctx.cancel).map_err(|e| media_stop(e, None))?;
    ctx.check_cancel()?;
    set_ready(ctx, audio, media::edits_from_tags(&probe, name), probe.duration_s);
    Ok(())
}

/// Outcome of a preparation thread that did not reach `ready`: the job is over. A local-file
/// job's folder is removed (nothing worth keeping); a URL job's folder stays (resume).
fn finish_prepare(ctx: &Ctx, r: R<()>, remove_dir: bool) {
    let Err(stop) = r else { return };
    if remove_dir {
        let dir = guard(&ctx.inner.job).as_ref().filter(|j| j.snap.job == ctx.id).map(|j| j.dir.clone());
        if let Some(d) = dir {
            if let Err(e) = ctx.tmp.remove_job_dir(&d) {
                eprintln!("calliope: import cleanup: {e}");
            }
        }
    }
    match stop {
        Stop::Cancelled => {
            ctx.update(|j| {
                j.snap.phase = Phase::Cancelled;
                j.running = false;
                j.over = true;
                j.cancel = None;
            });
            eprintln!("calliope: import job={} phase=Cancelled", ctx.id);
            ctx.emit(ImportEvent::Cancelled { back_to: BackTo::Source });
        }
        Stop::Failed(e) => {
            ctx.update(|j| {
                j.snap.phase = Phase::Failed;
                j.snap.error = Some(e.clone());
                j.running = false;
                j.over = true;
                j.cancel = None;
            });
            eprintln!("calliope: import job={} phase=Failed stage={:?}", ctx.id, e.stage);
            ctx.emit(ImportEvent::Failed { stage: e.stage, message: e.message, http_status: e.http_status });
        }
    }
}

fn client_stop(e: ClientError) -> Stop {
    match e {
        ClientError::Cancelled => Stop::Cancelled,
        ClientError::Rejected { status, ref message } => {
            Stop::Failed(JobError { stage: Stage::Server, message: format!("The edge-AI server said {status}: {message}"), http_status: Some(status) })
        }
        other => Stop::Failed(JobError { stage: Stage::Server, message: other.to_string(), http_status: None }),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_extract(
    ctx: &Ctx,
    dir: &Path,
    audio: &Path,
    edits: &TrackEdits,
    edge_url: &str,
    keep_original: bool,
    staging: &mut Option<Staging>,
    server_job: &mut Option<String>,
) -> R<TrackRecord> {
    let client = StemsClient::new(edge_url);
    ctx.check_cancel()?;
    let health = client.health().map_err(client_stop)?;
    let model = health.default_model.clone();
    ctx.check_cancel()?;
    let c = ctx.clone();
    let id = client
        .submit(audio, ctx.cancel.flag(), move |sent, total| {
            c.update(|j| {
                j.snap.sent = sent;
                j.snap.total = Some(total);
            });
            c.progress(ImportEvent::Uploading { sent, total });
        })
        .map_err(client_stop)?;
    *server_job = Some(id.clone());
    ctx.check_cancel()?;
    ctx.phase(Phase::Queued, ImportEvent::Queued);
    let c = ctx.clone();
    let mut last: Option<JobState> = None;
    let st = client
        .wait_final(&id, ctx.cancel.flag(), ctx.inner.poll, STALL_TIMEOUT, |st| {
            if last != Some(st.state) {
                last = Some(st.state);
                if st.state == JobState::Running {
                    c.update(|j| j.snap.progress = st.progress);
                    c.phase(Phase::Working, ImportEvent::Working { progress: st.progress });
                }
            } else if st.state == JobState::Running {
                c.update(|j| j.snap.progress = st.progress);
                c.progress(ImportEvent::Working { progress: st.progress });
            }
        })
        .map_err(client_stop)?;
    match st.state {
        JobState::Done => {}
        JobState::Failed => {
            let m = st.error.map(|m| media::clean_text(&m, 300)).filter(|m| !m.is_empty()).unwrap_or_else(|| "The edge-AI server could not separate the track".into());
            return fail(Stage::Server, m);
        }
        _ => return fail(Stage::Server, "The job was cancelled on the edge-AI server"),
    }
    let stems = st.stems.unwrap_or_default();
    if stems.is_empty() {
        return fail(Stage::Server, "The edge-AI server returned no stems");
    }
    // validates the final metadata before any download
    build_meta("validation", edits, &stems, &model, keep_original).map_err(|m| Stop::Failed(JobError { stage: Stage::Save, message: m, http_status: None }))?;
    let new_id = track_meta::new_id();
    let save_err = |m: String| Stop::Failed(JobError { stage: Stage::Save, message: m, http_status: None });
    let root = ctx.root.clone();
    *staging = Some(
        locked(&*ctx.inner.lock, || Repository::new(&root).begin_staged_track(&new_id)).map_err(save_err)?,
    );
    let total = stems.len() as u32;
    ctx.update(|j| {
        j.snap.stems_total = total;
        j.snap.stems_done = 0;
    });
    ctx.phase(Phase::Receiving, ImportEvent::Receiving { done: 0, total });
    for (i, name) in stems.iter().enumerate() {
        ctx.check_cancel()?;
        let part = staging.as_ref().expect("staging").stem_part_path(name).map_err(save_err)?;
        client.fetch_stem(&id, name, &part, ctx.cancel.flag()).map_err(client_stop)?;
        staging.as_ref().expect("staging").finish_stem(name).map_err(save_err)?;
        let done = i as u32 + 1;
        ctx.update(|j| j.snap.stems_done = done);
        ctx.progress(ImportEvent::Receiving { done, total });
    }
    ctx.check_cancel()?;
    ctx.phase(Phase::Saving, ImportEvent::Saving);
    if keep_original {
        staging.as_mut().expect("staging").adopt_original(audio).map_err(save_err)?;
    }
    let meta = build_meta(&new_id, edits, &stems, &model, keep_original).map_err(save_err)?;
    let record = locked(&*ctx.inner.lock, || staging.as_ref().expect("staging").commit(&meta)).map_err(save_err)?;
    // the track exists now: nothing to abandon any more
    *staging = None;
    if let Some(sid) = server_job.take() {
        client.delete(&sid);
    }
    if let Err(e) = ctx.tmp.remove_job_dir(dir) {
        eprintln!("calliope: import cleanup: {e}");
    }
    Ok(record)
}

fn finish_extract(ctx: &Ctx, r: R<TrackRecord>) {
    match r {
        Ok(track) => {
            ctx.update(|j| {
                j.snap.phase = Phase::Saved;
                j.snap.track = Some(track.clone());
                j.snap.error = None;
                j.audio = None;
                j.running = false;
                j.over = true;
                j.cancel = None;
            });
            eprintln!("calliope: import job={} phase=Saved", ctx.id);
            ctx.emit(ImportEvent::Saved { track });
        }
        Err(Stop::Cancelled) => {
            ctx.update(|j| {
                j.snap.phase = Phase::Ready;
                j.snap.sent = 0;
                j.snap.progress = None;
                j.snap.stems_done = 0;
                j.snap.stems_total = 0;
                j.running = false;
                j.cancel = None;
            });
            eprintln!("calliope: import job={} phase=Ready (extraction cancelled)", ctx.id);
            ctx.emit(ImportEvent::Cancelled { back_to: BackTo::Edit });
        }
        Err(Stop::Failed(e)) => {
            ctx.update(|j| {
                j.snap.phase = Phase::Failed;
                j.snap.error = Some(e.clone());
                j.running = false;
                j.cancel = None;
            });
            eprintln!("calliope: import job={} phase=Failed stage={:?}", ctx.id, e.stage);
            ctx.emit(ImportEvent::Failed { stage: e.stage, message: e.message, http_status: e.http_status });
        }
    }
}


#[cfg(test)]
#[path = "import_job_tests.rs"]
mod tests;
