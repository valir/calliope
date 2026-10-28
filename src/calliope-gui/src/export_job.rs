//! The export job (plan section 2.6): one export at a time on its own thread. `plan` checks the
//! destination and makes a single-use token; `start` re-checks, creates the missing RC-5 folders
//! and the part file, and runs `export_audio::run`; the part file is renamed to the target at the
//! end, so a cancel or a failure leaves nothing behind. No Tauri.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use calliope_lib::process;
use serde::{Deserialize, Serialize};

use crate::backing_render::Throttle;
use crate::export_audio::{self, count_in_plan, CountInPlan, Output, Pipeline, CANCELLED};
pub use crate::export_rules::ExportKind;
use crate::export_rules::{self, has_encoder, StreamInfo, NO_TEMPO_REASON};
use crate::fsutil::sync_dir;
use crate::repository::{inside_root, TrackRecord};
use crate::tempo_map::TempoMap;
use crate::tools::Tools;
use crate::wav::WavWriter;

pub const BUSY_MESSAGE: &str = "An export is running";
pub const STALE_MESSAGE: &str = "This export is no longer ready; press the button again";
pub const TOOLS_MESSAGE: &str = "Install ffmpeg to export (ffmpeg and ffprobe are needed)";
/// Progress events per second at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// What the user asked for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TargetReq {
    Rc5 {
        memory: String,
        /// The folder the frontend shows as "To:"; the dialog is skipped only if it is the one
        /// Rust remembers (and it is still a usable RC-5 folder).
        #[serde(default)]
        reuse: Option<String>,
    },
    Mp3,
    Ogg,
}

impl TargetReq {
    pub fn kind(&self) -> ExportKind {
        match self {
            TargetReq::Rc5 { .. } => ExportKind::Rc5,
            TargetReq::Mp3 => ExportKind::Mp3,
            TargetReq::Ogg => ExportKind::Ogg,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportPlan {
    pub token: String,
    /// The file that will be written (for display; the frontend never sends it back).
    pub path: String,
    pub exists: bool,
    /// The folder the plan writes into (for display).
    pub base: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportPhase {
    Writing,
    Finishing,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportSnapshot {
    pub job: String,
    pub id: String,
    pub title: String,
    pub target: ExportKind,
    pub path: String,
    pub phase: ExportPhase,
    pub progress: f32,
    pub message: Option<String>,
    pub clipped: u32,
    /// Why the export info wasn't (fully) saved; `None` until done, and when all went well.
    pub info_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum ExportEvent {
    Writing {
        progress: f32,
    },
    Finishing,
    Done {
        path: String,
        clipped: u32,
        root: String,
        records: Vec<TrackRecord>,
        info_error: Option<String>,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

pub type Sink = Arc<dyn Fn(ExportEvent) + Send + Sync>;

/// What `on_done` is told about a finished export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub id: String,
    pub kind: ExportKind,
    pub memory: Option<u8>,
    pub path: PathBuf,
}

/// What `on_done` saved: the changed records (for the Library), and what went wrong.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Recorded {
    pub root: String,
    pub records: Vec<TrackRecord>,
    pub info_error: Option<String>,
}

/// Runs on the export thread after the file is in place and before `done` is published.
pub type OnDone = Box<dyn FnOnce(&Finished) -> Recorded + Send>;

/// A pending plan, as `ExportState::planned` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRef {
    pub id: String,
    pub track_dir: PathBuf,
    pub repo_root: PathBuf,
    pub kind: ExportKind,
    pub memory: Option<u8>,
}

pub struct PlanInput<'a> {
    pub track: &'a TrackRecord,
    /// Absolute path of the source file.
    pub source: PathBuf,
    pub kind: ExportKind,
    /// The RC-5 memory (RC-5 only).
    pub memory: Option<u8>,
    /// The folder picked by the user: the RC-5's root, or the MP3/OGG folder.
    pub chosen: &'a Path,
    pub repo_root: &'a Path,
    /// The track's directory; `start` reads the tempo map from it.
    pub track_dir: PathBuf,
    pub tempo: &'a TempoMap,
}

/// A checked destination waiting for `start`.
struct Planned {
    token: String,
    id: String,
    title: String,
    tags: Vec<(&'static str, String)>,
    track_dir: PathBuf,
    source: PathBuf,
    kind: ExportKind,
    memory: Option<u8>,
    base: PathBuf,
    repo_root: PathBuf,
    target: PathBuf,
    rate: u32,
    channels: u32,
    duration_s: f64,
}

struct Job {
    snap: ExportSnapshot,
    cancel: Arc<AtomicBool>,
    sink: Option<Sink>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Inner {
    planned: Option<Planned>,
    job: Option<Job>,
    next_token: u64,
    next_job: u64,
    /// The base folder of the last successful RC-5 export (or accepted by "Change...").
    rc5_quick: Option<PathBuf>,
}

impl Inner {
    fn running(&self) -> bool {
        self.job
            .as_ref()
            .is_some_and(|j| matches!(j.snap.phase, ExportPhase::Writing | ExportPhase::Finishing))
    }
}

pub struct ExportState {
    tools: Box<dyn Fn() -> Tools + Send + Sync>,
    pace: Option<f64>,
    inner: Arc<Mutex<Inner>>,
    part_counter: AtomicU64,
}

fn refuse<T>(kind: ExportKind, reason: &str, message: impl Into<String>) -> Result<T, String> {
    eprintln!(
        "calliope: export refused target={} reason={reason}",
        kind.as_str()
    );
    Err(message.into())
}

fn lock(m: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn last_line(last: &Arc<Mutex<String>>) -> impl FnMut(String) + Send + 'static {
    let last = last.clone();
    move |l| {
        if !l.trim().is_empty() {
            if let Ok(mut s) = last.lock() {
                *s = l;
            }
        }
    }
}

/// Runs `program` to the end and returns its stdout lines joined, or its last stderr line.
fn capture(program: &Path, args: &[std::ffi::OsString]) -> Result<String, String> {
    let out = Arc::new(Mutex::new(Vec::<String>::new()));
    let err = Arc::new(Mutex::new(String::new()));
    let o = out.clone();
    let running = process::spawn(
        program,
        args,
        None,
        move |l| {
            if let Ok(mut v) = o.lock() {
                v.push(l);
            }
        },
        last_line(&err),
    )
    .map_err(|e| e.to_string())?;
    let status = running.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let e = err.lock().map(|s| s.clone()).unwrap_or_default();
        return Err(if e.is_empty() {
            format!("{} failed", program.display())
        } else {
            e
        });
    }
    let v = out.lock().map(|v| v.join("\n")).unwrap_or_default();
    Ok(v)
}

fn probe(ffprobe: &Path, src: &Path) -> Result<StreamInfo, String> {
    let name = src.file_name().map_or_else(
        || src.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut file = std::ffi::OsString::from("file:");
    file.push(src.as_os_str());
    let mut args: Vec<std::ffi::OsString> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=sample_rate,channels,duration:format=duration",
        "-of",
        "json",
    ]
    .iter()
    .map(Into::into)
    .collect();
    args.push(file);
    let json = capture(ffprobe, &args).map_err(|e| format!("Cannot read {name}: {e}"))?;
    export_rules::parse_stream_info(&json).ok_or_else(|| format!("Cannot read the audio of {name}"))
}

/// The checks of an RC-5 base folder, in order: it is a folder, it holds `ROLAND`, and it is not
/// inside the repository. `Err((reason, message))`.
pub fn check_rc5_base(base: &Path, repo_root: &Path) -> Result<(), (&'static str, String)> {
    if !base.is_dir() {
        return Err((
            "not-folder",
            format!("{} is not an existing folder", base.display()),
        ));
    }
    if !base.join("ROLAND").is_dir() {
        return Err((
            "no-roland",
            format!(
                "No ROLAND folder in {}: is the RC-5 mounted?",
                base.display()
            ),
        ));
    }
    if inside_root(repo_root, base) {
        return Err((
            "inside-repo",
            "Cannot export into the repository folder".into(),
        ));
    }
    Ok(())
}

/// The destination checks of plan 2.5, in order. `Err((reason, message))`.
fn check_destination(
    kind: ExportKind,
    base: &Path,
    repo_root: &Path,
    target: &Path,
) -> Result<(), (&'static str, String)> {
    if kind == ExportKind::Rc5 {
        check_rc5_base(base, repo_root)?;
    } else {
        if !base.is_dir() {
            return Err((
                "not-folder",
                format!("{} is not an existing folder", base.display()),
            ));
        }
        if inside_root(repo_root, base) {
            return Err((
                "inside-repo",
                "Cannot export into the repository folder".into(),
            ));
        }
    }
    if kind == ExportKind::Rc5 {
        let dirs = [
            base.join("ROLAND").join("WAVE"),
            target.parent().map(Path::to_path_buf).unwrap_or_default(),
        ];
        for d in dirs {
            if d.exists() && !d.is_dir() {
                return Err(("not-folder", format!("{} is not a folder", d.display())));
            }
        }
    }
    if target.is_dir() {
        return Err(("is-folder", format!("{} is a folder", target.display())));
    }
    Ok(())
}

fn target_of(kind: ExportKind, base: &Path, memory: Option<u8>, track: &TrackRecord) -> PathBuf {
    match kind {
        ExportKind::Rc5 => export_rules::rc5_target(base, memory.unwrap_or(1)),
        ExportKind::Mp3 => base.join(export_rules::file_name(
            &track.band,
            &track.title,
            "mp3",
            &track.id,
        )),
        ExportKind::Ogg => base.join(export_rules::file_name(
            &track.band,
            &track.title,
            "ogg",
            &track.id,
        )),
    }
}

impl ExportState {
    pub fn new(tools: Box<dyn Fn() -> Tools + Send + Sync>) -> Self {
        Self {
            tools,
            pace: None,
            inner: Arc::new(Mutex::new(Inner::default())),
            part_counter: AtomicU64::new(0),
        }
    }

    /// Tests only: the export takes `factor` times the audio's duration.
    pub fn with_pace(mut self, factor: f64) -> Self {
        self.pace = Some(factor);
        self
    }

    /// `ffmpeg` and `ffprobe` (re-discovered now), and for MP3/OGG the encoder. Meant to run
    /// before any folder dialog.
    pub fn require(&self, kind: ExportKind) -> Result<(PathBuf, PathBuf), String> {
        let t = (self.tools)();
        let (Some(ffmpeg), Some(ffprobe)) = (t.ffmpeg, t.ffprobe) else {
            return refuse(kind, "tools", TOOLS_MESSAGE);
        };
        let (name, what) = match kind {
            ExportKind::Rc5 => return Ok((ffmpeg, ffprobe)),
            ExportKind::Mp3 => ("libmp3lame", "MP3 encoder (libmp3lame)"),
            ExportKind::Ogg => ("libvorbis", "Ogg Vorbis encoder (libvorbis)"),
        };
        let listing =
            capture(&ffmpeg, &["-hide_banner".into(), "-encoders".into()]).unwrap_or_default();
        if !has_encoder(&listing, name) {
            return refuse(
                kind,
                "encoder",
                format!("Your ffmpeg has no {what}: install an ffmpeg build that includes it"),
            );
        }
        Ok((ffmpeg, ffprobe))
    }

    /// Checks the chosen folder and makes a single-use plan (replacing an older one).
    pub fn plan(&self, p: PlanInput) -> Result<ExportPlan, String> {
        let kind = p.kind;
        if lock(&self.inner).running() {
            return refuse(kind, "busy", BUSY_MESSAGE);
        }
        if p.tempo.is_empty() {
            return refuse(kind, "no-tempo", NO_TEMPO_REASON);
        }
        let (_ffmpeg, ffprobe) = self.require_tools(kind)?;
        let info = probe(&ffprobe, &p.source).map_err(|e| {
            eprintln!("calliope: export failed job=none message={e}");
            e
        })?;
        let target = target_of(kind, p.chosen, p.memory, p.track);
        if let Err((reason, msg)) = check_destination(kind, p.chosen, p.repo_root, &target) {
            return refuse(kind, reason, msg);
        }
        let (rate, channels) = export_rules::output_format(kind, &info);
        let mut g = lock(&self.inner);
        if g.running() {
            drop(g);
            return refuse(kind, "busy", BUSY_MESSAGE);
        }
        g.next_token += 1;
        let token = format!("x{}", g.next_token);
        let plan = ExportPlan {
            token: token.clone(),
            path: target.display().to_string(),
            exists: target.exists(),
            base: p.chosen.to_string_lossy().into_owned(),
        };
        g.planned = Some(Planned {
            token,
            id: p.track.id.clone(),
            title: p.track.title.clone(),
            tags: export_rules::tags(p.track),
            track_dir: p.track_dir,
            source: p.source,
            kind,
            memory: p.memory,
            base: p.chosen.to_path_buf(),
            repo_root: p.repo_root.to_path_buf(),
            target,
            rate,
            channels,
            duration_s: info.duration_s,
        });
        Ok(plan)
    }

    fn require_tools(&self, kind: ExportKind) -> Result<(PathBuf, PathBuf), String> {
        let t = (self.tools)();
        match (t.ffmpeg, t.ffprobe) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => refuse(kind, "tools", TOOLS_MESSAGE),
        }
    }

    /// The RC-5 folder the next export may use without the dialog (in memory only).
    pub fn rc5_quick(&self) -> Option<PathBuf> {
        lock(&self.inner).rc5_quick.clone()
    }

    pub fn set_rc5_quick(&self, dir: PathBuf) {
        lock(&self.inner).rc5_quick = Some(dir);
    }

    /// True while an export runs.
    pub fn is_running(&self) -> bool {
        lock(&self.inner).running()
    }

    /// The track, its directory and repository (as they were at prepare time), kind and RC-5
    /// memory of the pending plan `token`, if it is still valid.
    pub fn planned(&self, token: &str) -> Option<PlannedRef> {
        let g = lock(&self.inner);
        g.planned
            .as_ref()
            .filter(|p| p.token == token)
            .map(|p| PlannedRef {
                id: p.id.clone(),
                track_dir: p.track_dir.clone(),
                repo_root: p.repo_root.clone(),
                kind: p.kind,
                memory: p.memory,
            })
    }

    /// Starts the planned export. `overwrite` must be true when the target exists. `tempo` is the
    /// map as it is now: the count-in is computed again from it.
    pub fn start(
        &self,
        token: &str,
        overwrite: bool,
        tempo: &TempoMap,
        sink: Sink,
        on_done: OnDone,
    ) -> Result<ExportSnapshot, String> {
        let mut g = lock(&self.inner);
        let Some(pl) = g.planned.take().filter(|p| p.token == token) else {
            drop(g);
            eprintln!("calliope: export refused target=none reason=stale");
            return Err(STALE_MESSAGE.into());
        };
        let kind = pl.kind;
        if g.running() {
            drop(g);
            return refuse(kind, "busy", BUSY_MESSAGE);
        }
        let Some(count_in) = count_in_plan(tempo, pl.rate) else {
            drop(g);
            return refuse(kind, "no-tempo", NO_TEMPO_REASON);
        };
        let ffmpeg = match (self.tools)().ffmpeg {
            Some(f) => f,
            None => {
                drop(g);
                return refuse(kind, "tools", TOOLS_MESSAGE);
            }
        };
        if let Err((reason, msg)) = check_destination(kind, &pl.base, &pl.repo_root, &pl.target) {
            drop(g);
            return refuse(kind, reason, msg);
        }
        if pl.target.exists() && !overwrite {
            drop(g);
            return refuse(
                kind,
                "exists",
                format!("{} already exists", pl.target.display()),
            );
        }

        // Folders and the part file; remembering what we created.
        let mut created: Vec<PathBuf> = Vec::new();
        let fail = |created: &[PathBuf], dir: &Path, e: std::io::Error| -> String {
            remove_dirs(created);
            let msg = format!("Cannot write to {}: {e}", dir.display());
            eprintln!("calliope: export failed job=none message={msg}");
            msg
        };
        if kind == ExportKind::Rc5 {
            let wave = pl.base.join("ROLAND").join("WAVE");
            let mem = pl
                .target
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| wave.clone());
            for d in [wave, mem] {
                if !d.exists() {
                    if let Err(e) = std::fs::create_dir(&d) {
                        let parent = d.parent().unwrap_or(&d).to_path_buf();
                        return Err(fail(&created, &parent, e));
                    }
                    created.push(d);
                }
            }
        }
        let dir = pl
            .target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let name = pl
            .target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let n = self.part_counter.fetch_add(1, Ordering::SeqCst);
        let part = dir.join(format!(".{name}.calliope-{}-{n}.part", std::process::id()));
        let file = match OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(f) => f,
            Err(e) => return Err(fail(&created, &dir, e)),
        };

        g.next_job += 1;
        let job_id = format!("e{}", g.next_job);
        let snap = ExportSnapshot {
            job: job_id.clone(),
            id: pl.id.clone(),
            title: pl.title.clone(),
            target: kind,
            path: pl.target.display().to_string(),
            phase: ExportPhase::Writing,
            progress: 0.0,
            message: None,
            clipped: 0,
            info_error: None,
        };
        eprintln!(
            "calliope: export start job={job_id} id={} target={} path={} rate={} channels={} count_in={} first_beat_ms={} lead_frames={} skip_frames={} overwrite={overwrite}",
            pl.id,
            kind.as_str(),
            pl.target.display(),
            pl.rate,
            pl.channels,
            count_in.bpm,
            tempo.segments[0].start_ms,
            count_in.lead_frames,
            count_in.skip_frames,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let work = Work {
            inner: self.inner.clone(),
            job: job_id,
            cancel: cancel.clone(),
            ffmpeg,
            pl,
            count_in,
            created,
            part,
            file,
            overwrite,
            pace: self.pace,
            on_done,
        };
        let thread = std::thread::spawn(move || work.run());
        g.job = Some(Job {
            snap: snap.clone(),
            cancel,
            sink: Some(sink),
            thread: Some(thread),
        });
        Ok(snap)
    }

    /// Cancels the running job `job`; a finished or unknown job changes nothing. Returns the
    /// job's snapshot when `job` is the current one.
    pub fn cancel(&self, job: &str) -> Option<ExportSnapshot> {
        let g = lock(&self.inner);
        let j = g.job.as_ref().filter(|j| j.snap.job == job)?;
        if matches!(j.snap.phase, ExportPhase::Writing | ExportPhase::Finishing) {
            j.cancel.store(true, Ordering::SeqCst);
        }
        Some(j.snap.clone())
    }

    pub fn snapshot(&self) -> Option<ExportSnapshot> {
        lock(&self.inner).job.as_ref().map(|j| j.snap.clone())
    }

    /// Re-attaches: events of the current job go to `sink` from now on.
    pub fn watch(&self, sink: Sink) -> Option<ExportSnapshot> {
        let mut g = lock(&self.inner);
        let j = g.job.as_mut()?;
        j.sink = Some(sink);
        Some(j.snap.clone())
    }

    /// App exit: cancels the running job and waits up to `cap` for its thread (part file removed).
    pub fn shutdown(&self, cap: Duration) -> bool {
        let thread = {
            let mut g = lock(&self.inner);
            let Some(j) = g.job.as_mut() else { return true };
            j.cancel.store(true, Ordering::SeqCst);
            j.thread.take()
        };
        let Some(t) = thread else { return true };
        let end = Instant::now() + cap;
        while !t.is_finished() {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = t.join();
        true
    }
}

/// Removes the folders this job created, deepest first, never recursively.
fn remove_dirs(created: &[PathBuf]) {
    for d in created.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
}

struct Work {
    inner: Arc<Mutex<Inner>>,
    job: String,
    cancel: Arc<AtomicBool>,
    ffmpeg: PathBuf,
    pl: Planned,
    count_in: CountInPlan,
    created: Vec<PathBuf>,
    part: PathBuf,
    file: File,
    overwrite: bool,
    pace: Option<f64>,
    on_done: OnDone,
}

impl Work {
    /// Updates the snapshot of this job and sends `ev` to the current sink.
    fn publish(&self, update: impl FnOnce(&mut ExportSnapshot), ev: ExportEvent) {
        let sink = {
            let mut g = lock(&self.inner);
            match g.job.as_mut().filter(|j| j.snap.job == self.job) {
                Some(j) => {
                    update(&mut j.snap);
                    j.sink.clone()
                }
                None => None,
            }
        };
        if let Some(s) = sink {
            s(ev);
        }
    }

    fn run(mut self) {
        let on_done = std::mem::replace(&mut self.on_done, Box::new(|_| Recorded::default()));
        let result = self.render();
        match result {
            Ok(stats) => {
                let path = self.pl.target.display().to_string();
                eprintln!(
                    "calliope: export done job={} path={path} frames={} clipped={}",
                    self.job, stats.frames, stats.clipped
                );
                if self.pl.kind == ExportKind::Rc5 {
                    lock(&self.inner).rc5_quick = Some(self.pl.base.clone());
                }
                // Before `done` is published, so that whoever reacts to it (the pane reloads the
                // memory suggestion) sees what `on_done` saved.
                let rec = on_done(&Finished {
                    id: self.pl.id.clone(),
                    kind: self.pl.kind,
                    memory: self.pl.memory,
                    path: self.pl.target.clone(),
                });
                let info_error = rec.info_error.clone();
                self.publish(
                    |s| {
                        s.phase = ExportPhase::Done;
                        s.progress = 1.0;
                        s.clipped = stats.clipped;
                        s.info_error = info_error;
                    },
                    ExportEvent::Done {
                        path,
                        clipped: stats.clipped,
                        root: rec.root,
                        records: rec.records,
                        info_error: rec.info_error,
                    },
                );
            }
            Err(msg) => {
                let _ = std::fs::remove_file(&self.part);
                remove_dirs(&self.created);
                if msg == CANCELLED || self.cancel.load(Ordering::SeqCst) {
                    eprintln!("calliope: export cancelled job={}", self.job);
                    self.publish(|s| s.phase = ExportPhase::Cancelled, ExportEvent::Cancelled);
                } else {
                    eprintln!("calliope: export failed job={} message={msg}", self.job);
                    let m = msg.clone();
                    self.publish(
                        |s| {
                            s.phase = ExportPhase::Failed;
                            s.message = Some(m);
                        },
                        ExportEvent::Failed { message: msg },
                    );
                }
            }
        }
    }

    fn render(&self) -> Result<export_audio::RunStats, String> {
        let pl = &self.pl;
        let pipeline = Pipeline {
            ffmpeg: self.ffmpeg.clone(),
            source: pl.source.clone(),
            rate: pl.rate,
            channels: pl.channels,
            plan: self.count_in.clone(),
            part: self.part.clone(),
            pace: self.pace,
        };
        let part = self.part.display().to_string();
        let output = match pl.kind {
            ExportKind::Rc5 => {
                let f = self
                    .file
                    .try_clone()
                    .map_err(|e| format!("Cannot write {part}: {e}"))?;
                Output::Wav(
                    WavWriter::from_file(f, pl.rate)
                        .map_err(|e| format!("Cannot write {part}: {e}"))?,
                )
            }
            kind => Output::Encode {
                ffmpeg: self.ffmpeg.clone(),
                args: export_audio::encoder_args(kind, pl.rate, pl.channels, &pl.tags, &self.part),
            },
        };
        let expected = (self.count_in.lead_frames
            + (pl.duration_s * f64::from(pl.rate)).round() as u64)
            .saturating_sub(self.count_in.skip_frames);
        let mut throttle = Throttle::new(PROGRESS_INTERVAL);
        let stats = export_audio::run(&pipeline, output, &self.cancel, &mut |frames| {
            let p = (frames as f64 / expected.max(1) as f64).min(0.99) as f32;
            if throttle.allow(Instant::now()) {
                self.publish(|s| s.progress = p, ExportEvent::Writing { progress: p });
            }
        })?;

        self.publish(|s| s.phase = ExportPhase::Finishing, ExportEvent::Finishing);
        if pl.kind != ExportKind::Rc5 {
            File::open(&self.part)
                .and_then(|f| f.sync_all())
                .map_err(|e| format!("Cannot write {part}: {e}"))?;
        }
        // A cancel still wins until the rename.
        if self.cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        if !self.overwrite && pl.target.exists() {
            return Err(format!("{} already exists", pl.target.display()));
        }
        std::fs::rename(&self.part, &pl.target)
            .map_err(|e| format!("Cannot write {}: {e}", pl.target.display()))?;
        if let Some(dir) = pl.target.parent() {
            sync_dir(dir);
        }
        Ok(stats)
    }
}

#[cfg(test)]
#[path = "export_job_tests.rs"]
mod tests;
