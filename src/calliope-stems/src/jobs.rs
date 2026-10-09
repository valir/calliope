//! Job state (in memory), the single worker thread, cancellation and the janitor.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use calliope_lib::process::{self, CancelHandle};
use calliope_lib::flac_peak;
use calliope_lib::stems_api::{JobState, JobStatus, StemPeak};

use crate::config::Config;
use crate::separator::{parse_progress, truncate, validate_output};
use crate::{log, workdir};

struct Job {
    state: JobState,
    progress: Option<f64>,
    stems: Option<Vec<String>>,
    stem_peaks: Option<Vec<StemPeak>>,
    error: Option<String>,
    dir: PathBuf,
    started: Option<Instant>,
    finished: Option<Instant>,
    cancel: Option<CancelHandle>,
}

#[derive(Default)]
struct State {
    jobs: HashMap<String, Job>,
    queue: VecDeque<String>,
    /// Uploads in progress; they hold a place in the queue.
    reserved: usize,
    /// The job whose separator the worker is handling.
    current: Option<String>,
    stop: bool,
}

impl State {
    fn active(&self) -> usize {
        self.reserved
            + self.jobs.values().filter(|j| matches!(j.state, JobState::Queued | JobState::Running)).count()
    }
}

pub struct Manager {
    cfg: Arc<Config>,
    state: Mutex<State>,
    /// Wakes the worker.
    work: Condvar,
    /// Signals that the worker finished a job.
    idle: Condvar,
}

/// A place in the queue, held while a body is uploaded.
pub struct Slot {
    mgr: Arc<Manager>,
    armed: bool,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.armed {
            self.mgr.lock().reserved -= 1;
        }
    }
}

pub enum StemLookup {
    Found(PathBuf),
    UnknownJob,
    NotDone,
    UnknownStem,
}

/// Longest wait for a killed separator to be reaped before its job folder is removed.
const CANCEL_WAIT: Duration = Duration::from_secs(10);

impl Manager {
    pub fn new(cfg: Arc<Config>) -> Arc<Manager> {
        Arc::new(Manager { cfg, state: Mutex::new(State::default()), work: Condvar::new(), idle: Condvar::new() })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a place in the queue (queue limit + the running job), or `None` when it is full.
    pub fn reserve(self: &Arc<Self>) -> Option<Slot> {
        let mut st = self.lock();
        if st.stop || st.active() > self.cfg.queue {
            return None;
        }
        st.reserved += 1;
        Some(Slot { mgr: self.clone(), armed: true })
    }

    /// Registers an uploaded job as queued and wakes the worker.
    pub fn enqueue(&self, mut slot: Slot, id: String, dir: PathBuf) {
        let mut st = self.lock();
        slot.armed = false;
        st.reserved -= 1;
        st.jobs.insert(
            id.clone(),
            Job {
                state: JobState::Queued,
                progress: None,
                stems: None,
                stem_peaks: None,
                error: None,
                dir,
                started: None,
                finished: None,
                cancel: None,
            },
        );
        st.queue.push_back(id.clone());
        drop(st);
        log(format_args!("job id={id} state=queued"));
        self.work.notify_all();
    }

    pub fn busy(&self) -> bool {
        self.lock().active() > 0
    }

    pub fn status(&self, id: &str) -> Option<JobStatus> {
        let st = self.lock();
        let j = st.jobs.get(id)?;
        Some(JobStatus {
            job: id.to_string(),
            state: j.state,
            progress: if j.state == JobState::Done { Some(1.0) } else { j.progress },
            stems: j.stems.clone(),
            error: j.error.clone(),
            stem_peaks: j.stem_peaks.clone(),
        })
    }

    pub fn stem_path(&self, id: &str, name: &str) -> StemLookup {
        let st = self.lock();
        let Some(j) = st.jobs.get(id) else { return StemLookup::UnknownJob };
        if j.state != JobState::Done {
            return StemLookup::NotDone;
        }
        if j.stems.as_ref().is_some_and(|s| s.iter().any(|n| n == name)) {
            StemLookup::Found(j.dir.join("out").join(format!("{name}.flac")))
        } else {
            StemLookup::UnknownStem
        }
    }

    /// `DELETE`: cancels a queued or running job (the job stays visible as `cancelled`), or
    /// forgets a finished one. Files are deleted either way. False when the job is unknown.
    pub fn delete(&self, id: &str) -> bool {
        let mut st = self.lock();
        let Some(job) = st.jobs.get_mut(id) else { return false };
        match job.state {
            JobState::Queued => {
                job.state = JobState::Cancelled;
                job.finished = Some(Instant::now());
                st.queue.retain(|q| q != id);
            }
            JobState::Running => {
                job.state = JobState::Cancelled;
                job.finished = Some(Instant::now());
                if let Some(h) = &job.cancel {
                    if !h.is_finished() {
                        h.cancel();
                    }
                }
                let deadline = Instant::now() + CANCEL_WAIT;
                while st.current.as_deref() == Some(id) {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    st = self.idle.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
                }
            }
            _ => {
                st.jobs.remove(id);
            }
        }
        drop(st);
        if let Err(e) = workdir::remove_job_dir(&self.cfg.work_dir, id) {
            log(format_args!("job id={id} error=\"cannot remove files: {e}\""));
        }
        log(format_args!("job id={id} deleted"));
        true
    }

    /// Janitor: forgets finished jobs older than the retention and deletes their files.
    /// Returns how many.
    pub fn sweep(&self) -> usize {
        let now = Instant::now();
        let expired: Vec<String> = {
            let mut st = self.lock();
            let ids: Vec<String> = st
                .jobs
                .iter()
                .filter(|(_, j)| {
                    j.state.is_final() && j.finished.is_some_and(|f| now.duration_since(f) >= self.cfg.retention)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &ids {
                st.jobs.remove(id);
            }
            ids
        };
        for id in &expired {
            let _ = workdir::remove_job_dir(&self.cfg.work_dir, id);
        }
        if !expired.is_empty() {
            log(format_args!("cleanup removed={}", expired.len()));
        }
        expired.len()
    }

    /// Janitor thread body.
    pub fn run_janitor(self: Arc<Self>) {
        loop {
            std::thread::sleep(self.cfg.janitor_interval);
            if self.lock().stop {
                return;
            }
            self.sweep();
        }
    }

    /// Stops the worker: kills a running separator, refuses new jobs.
    pub fn shutdown(&self) {
        let mut st = self.lock();
        st.stop = true;
        if let Some(id) = st.current.clone() {
            if let Some(h) = st.jobs.get(&id).and_then(|j| j.cancel.as_ref()) {
                if !h.is_finished() {
                    h.cancel();
                }
            }
        }
        drop(st);
        self.work.notify_all();
    }

    /// Waits until the worker has finished its current job (after `shutdown`).
    pub fn wait_idle(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut st = self.lock();
        while st.current.is_some() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            st = self.idle.wait_timeout(st, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The worker thread body: one job at a time, in order.
    pub fn run_worker(self: Arc<Self>) {
        loop {
            let id = {
                let mut st = self.lock();
                loop {
                    if st.stop {
                        return;
                    }
                    match st.queue.pop_front() {
                        Some(id) if st.jobs.get(&id).is_some_and(|j| j.state == JobState::Queued) => break id,
                        Some(_) => continue,
                        None => st = self.work.wait(st).unwrap_or_else(|e| e.into_inner()),
                    }
                }
            };
            self.process(&id);
        }
    }

    fn fail_locked(&self, st: &mut State, id: &str, msg: String) {
        if let Some(j) = st.jobs.get_mut(id) {
            j.state = JobState::Failed;
            j.error = Some(msg.clone());
            j.finished = Some(Instant::now());
            j.cancel = None;
        }
        log(format_args!("job id={id} state=failed error=\"{}\"", truncate(&msg, 300)));
    }

    fn process(self: &Arc<Self>, id: &str) {
        let cfg = &self.cfg;
        {
            let mut st = self.lock();
            let Some(job) = st.jobs.get_mut(id) else { return };
            if job.state != JobState::Queued {
                return;
            }
            job.state = JobState::Running;
            job.started = Some(Instant::now());
            let dir = job.dir.clone();
            let out = dir.join("out");
            let tail = Arc::new(Mutex::new(String::new()));
            let args = [dir.join("input.flac").into_os_string(), out.clone().into_os_string(), cfg.model.clone().into()];
            let spawned = std::fs::create_dir_all(&out).map_err(|e| e.to_string()).and_then(|()| {
                let (m1, m2) = (self.clone(), self.clone());
                let (id1, id2) = (id.to_string(), id.to_string());
                let (t1, t2) = (tail.clone(), tail.clone());
                process::spawn(
                    &cfg.separator,
                    &args,
                    Some(&dir),
                    move |line| m1.on_line(&id1, &t1, line),
                    move |line| m2.on_line(&id2, &t2, line),
                )
                .map_err(|e| e.to_string())
            });
            match spawned {
                Ok(running) => {
                    let handle = running.cancel_handle();
                    let job = st.jobs.get_mut(id).expect("job present");
                    job.cancel = Some(handle);
                    st.current = Some(id.to_string());
                    drop(st);
                    log(format_args!("job id={id} state=running"));
                    self.supervise(id, running, dir, tail);
                }
                Err(e) => {
                    self.fail_locked(&mut st, id, format!("could not start the separator: {e}"));
                    drop(st);
                    let _ = workdir::remove_job_dir(&cfg.work_dir, id);
                }
            }
        }
    }

    fn supervise(self: &Arc<Self>, id: &str, running: process::Running, dir: PathBuf, tail: Arc<Mutex<String>>) {
        let handle = running.cancel_handle();
        let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (disarm, armed) = mpsc::channel::<()>();
        let timer = {
            let (timed_out, timeout) = (timed_out.clone(), self.cfg.separator_timeout);
            std::thread::spawn(move || {
                if let Err(RecvTimeoutError::Timeout) = armed.recv_timeout(timeout) {
                    timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                    handle.cancel();
                }
            })
        };
        let status = running.wait();
        drop(disarm);
        let _ = timer.join();

        let mut outcome: Result<Vec<String>, String> = if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
            Err(format!("the separator timed out after {} minutes", self.cfg.separator_timeout.as_secs() / 60))
        } else {
            match status {
                Err(e) => Err(format!("could not wait for the separator: {e}")),
                Ok(s) if s.success() => validate_output(&dir.join("out")),
                Ok(s) => {
                    let last = tail.lock().map(|t| t.clone()).unwrap_or_default();
                    let how = match s.code() {
                        Some(c) => format!("exit status {c}"),
                        None => "killed by a signal".to_string(),
                    };
                    Err(if last.is_empty() {
                        format!("the separator failed ({how})")
                    } else {
                        format!("the separator failed ({how}): {last}")
                    })
                }
            }
        };

        // Measure the stems outside the lock; cancel and shutdown are checked between stems.
        let mut stem_peaks = None;
        if let (true, Ok(stems)) = (self.cfg.stem_peaks, &outcome) {
            match self.measure_stems(id, &dir.join("out"), stems) {
                Some(peaks) => stem_peaks = Some(peaks),
                None => {
                    // Interrupted: a cancelled job takes the cancelled path below; a stopping
                    // server just gives the job up.
                    outcome = Err("the server is stopping".to_string());
                }
            }
        }

        let mut st = self.lock();
        let cancelled = st.jobs.get(id).is_none_or(|j| j.state == JobState::Cancelled);
        let mut remove_files = false;
        if !cancelled {
            match outcome {
                Ok(stems) => {
                    let j = st.jobs.get_mut(id).expect("job present");
                    let secs = j.started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
                    j.state = JobState::Done;
                    j.progress = Some(1.0);
                    j.stems = Some(stems);
                    j.stem_peaks = stem_peaks;
                    j.finished = Some(Instant::now());
                    j.cancel = None;
                    log(format_args!("job id={id} state=done duration_s={secs:.1}"));
                }
                Err(msg) => {
                    self.fail_locked(&mut st, id, msg);
                    remove_files = true;
                }
            }
        } else {
            log(format_args!("job id={id} state=cancelled"));
        }
        st.current = None;
        drop(st);
        self.idle.notify_all();
        if remove_files {
            let _ = workdir::remove_job_dir(&self.cfg.work_dir, id);
        }
    }

    /// True when the job was cancelled or deleted, or the server is stopping.
    fn interrupted(&self, id: &str) -> bool {
        let st = self.lock();
        st.stop || st.jobs.get(id).is_none_or(|j| j.state == JobState::Cancelled)
    }

    /// Full-scan peak of every stem, in order. A stem that cannot be decoded gets no entry.
    /// `None` when cancelled or stopped part-way.
    fn measure_stems(&self, id: &str, out: &std::path::Path, stems: &[String]) -> Option<Vec<StemPeak>> {
        let t0 = Instant::now();
        let mut peaks = Vec::new();
        for name in stems {
            if self.interrupted(id) {
                return None;
            }
            match flac_peak::scan(&out.join(format!("{name}.flac")), None) {
                Ok(sc) => {
                    let peak_dbfs = flac_peak::to_dbfs(sc.peak, sc.bits).map(|d| (d * 100.0).round() / 100.0);
                    let shown = peak_dbfs.map_or("-inf".to_string(), |d| format!("{d:.1}"));
                    log(format_args!(
                        "job id={id} stem={name} peak={} bits={} peak_dbfs={shown}",
                        sc.peak, sc.bits
                    ));
                    peaks.push(StemPeak { name: name.clone(), peak: sc.peak, bits: sc.bits, peak_dbfs });
                }
                Err(e) => log(format_args!("job id={id} stem={name} peak=unknown error=\"{}\"", truncate(&e, 200))),
            }
        }
        log(format_args!(
            "job id={id} measured={}/{} ms={}",
            peaks.len(),
            stems.len(),
            t0.elapsed().as_millis()
        ));
        Some(peaks)
    }

    fn on_line(&self, id: &str, tail: &Mutex<String>, line: String) {
        if let Some(p) = parse_progress(&line) {
            if let Some(j) = self.lock().jobs.get_mut(id) {
                if j.state == JobState::Running {
                    j.progress = Some(p);
                }
            }
            return;
        }
        log(format_args!("separator id={id} line={}", truncate(&line, 300)));
        if !line.trim().is_empty() {
            if let Ok(mut t) = tail.lock() {
                *t = truncate(line.trim(), 200);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(work: &std::path::Path, queue: usize, retention: Duration) -> Arc<Config> {
        Arc::new(Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            work_dir: work.to_path_buf(),
            separator: PathBuf::from("/nonexistent"),
            model: "m".into(),
            max_upload_bytes: 1 << 20,
            max_duration_s: 900,
            queue,
            separator_timeout: Duration::from_secs(60),
            retention,
            janitor_interval: Duration::from_secs(600),
            stem_peaks: true,
        })
    }

    fn add(m: &Manager, id: &str, state: JobState, age: Duration) {
        let dir = workdir::create_job_dir(&m.cfg.work_dir, id).unwrap();
        m.lock().jobs.insert(
            id.to_string(),
            Job {
                state,
                progress: None,
                stems: None,
                stem_peaks: None,
                error: None,
                dir,
                started: None,
                finished: state.is_final().then(|| Instant::now() - age),
                cancel: None,
            },
        );
    }

    #[test]
    fn retention_removes_old_finished_jobs_only() {
        let tmp = tempfile::tempdir().unwrap();
        workdir::ensure(tmp.path()).unwrap();
        let m = Manager::new(cfg(tmp.path(), 2, Duration::from_secs(3600)));
        add(&m, "aa01", JobState::Done, Duration::from_secs(7200));
        add(&m, "aa02", JobState::Failed, Duration::from_secs(10));
        add(&m, "aa03", JobState::Queued, Duration::ZERO);
        add(&m, "aa04", JobState::Cancelled, Duration::from_secs(4000));
        assert_eq!(m.sweep(), 2);
        assert!(m.status("aa01").is_none() && m.status("aa04").is_none());
        assert!(m.status("aa02").is_some() && m.status("aa03").is_some());
        let jobs = workdir::jobs_root(tmp.path());
        assert!(!jobs.join("aa01").exists() && jobs.join("aa02").exists() && jobs.join("aa03").exists());
    }

    #[test]
    fn zero_retention_removes_finished_jobs_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        workdir::ensure(tmp.path()).unwrap();
        let m = Manager::new(cfg(tmp.path(), 2, Duration::ZERO));
        add(&m, "bb01", JobState::Done, Duration::ZERO);
        assert_eq!(m.sweep(), 1);
    }

    #[test]
    fn queue_limit_counts_running_waiting_and_uploading() {
        let tmp = tempfile::tempdir().unwrap();
        workdir::ensure(tmp.path()).unwrap();
        let m = Manager::new(cfg(tmp.path(), 1, Duration::from_secs(60)));
        let a = m.reserve().expect("first");
        let b = m.reserve().expect("second");
        assert!(m.reserve().is_none(), "queue 1 allows one running + one waiting");
        drop(b);
        let c = m.reserve().expect("a released slot is reusable");
        drop((a, c));
        assert!(!m.busy());
    }

    #[test]
    fn delete_semantics() {
        let tmp = tempfile::tempdir().unwrap();
        workdir::ensure(tmp.path()).unwrap();
        let m = Manager::new(cfg(tmp.path(), 2, Duration::from_secs(60)));
        add(&m, "cc01", JobState::Queued, Duration::ZERO);
        add(&m, "cc02", JobState::Done, Duration::ZERO);
        assert!(m.delete("cc01"));
        assert_eq!(m.status("cc01").unwrap().state, JobState::Cancelled);
        assert!(!workdir::jobs_root(tmp.path()).join("cc01").exists());
        assert!(m.delete("cc02"));
        assert!(m.status("cc02").is_none());
        assert!(!m.delete("cc02"));
        assert!(m.delete("cc01") && m.status("cc01").is_none());
    }

    #[test]
    fn stem_lookup() {
        let tmp = tempfile::tempdir().unwrap();
        workdir::ensure(tmp.path()).unwrap();
        let m = Manager::new(cfg(tmp.path(), 2, Duration::from_secs(60)));
        add(&m, "dd01", JobState::Queued, Duration::ZERO);
        add(&m, "dd02", JobState::Done, Duration::ZERO);
        m.lock().jobs.get_mut("dd02").unwrap().stems = Some(vec!["vocals".into()]);
        assert!(matches!(m.stem_path("dd01", "vocals"), StemLookup::NotDone));
        assert!(matches!(m.stem_path("dd02", "vocals"), StemLookup::Found(_)));
        assert!(matches!(m.stem_path("dd02", "drums"), StemLookup::UnknownStem));
        assert!(matches!(m.stem_path("zz", "vocals"), StemLookup::UnknownJob));
    }
}
