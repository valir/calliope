//! The Detect BPM run inside the Player session (plan gui-section-bpm-detect 2.5): start, cancel
//! and the reaping of the outcome by `tick`. A child module of `player`, so it sees the private
//! session fields. The job thread (`section_bpm`) never touches the manager.

use std::thread::JoinHandle;

use super::*;
use crate::section_bpm::{
    self, BpmDetectView, BpmJob, BpmOutcome, BpmParams, BpmResult, BpmShared, ResultKind,
};

const CHANGED: &str = "The section changed during the detection; its BPM was left as it was";

/// A run in a session. Dropping it cancels the job (it stops at its next check, deletes the
/// server job and removes its part file).
pub(super) struct BpmRun {
    id: String,
    view: BpmDetectView,
    old_bpm: f64,
    shared: Arc<BpmShared>,
    #[allow(dead_code)] // the thread is detached
    handle: Option<JoinHandle<()>>,
}

impl BpmRun {
    /// The running view with the phase the job has reached.
    pub(super) fn view(&self) -> BpmDetectView {
        BpmDetectView {
            phase: self.shared.phase(),
            ..self.view.clone()
        }
    }
}

impl Drop for BpmRun {
    fn drop(&mut self) {
        if !self.shared.cancel.swap(true, Ordering::SeqCst) {
            eprintln!(
                "calliope: player detect cancelled id={} reason=closed",
                self.id
            );
        }
    }
}

fn refuse(id: &str, reason: &str, message: &str) -> Result<PlayerSnapshot, String> {
    eprintln!("calliope: player detect refused id={id} reason={reason}");
    Err(message.to_string())
}

impl PlayerManager {
    /// Starts measuring the BPM of section `index` (whose start is `at`). Every refusal starts
    /// nothing.
    pub fn start_bpm_detect(
        &mut self,
        id: &str,
        index: usize,
        at: f64,
        params: BpmParams,
    ) -> Result<PlayerSnapshot, String> {
        let s = match self.session.as_mut() {
            Some(s) if s.id == id => s,
            _ => return Err(format!("no player session for {id}")),
        };
        if s.state.tempo.is_empty()
            || s.state
                .tempo
                .segments
                .get(index)
                .is_some_and(|g| g.bpm <= 0.0)
        {
            return refuse(id, "no-tempo", "Type a BPM first");
        }
        if s.state.read_only {
            return refuse(
                id,
                "read-only",
                "This track's Player settings were written by a newer Calliope; they can't be changed",
            );
        }
        if s.bpm_detect.is_some() {
            return refuse(id, "busy", "A BPM detection is already running");
        }
        if params.url.trim().is_empty() {
            return refuse(id, "no-server", "Set the edge-AI server in Settings first");
        }
        if s.switching.is_some() {
            return refuse(id, "loading", "The source is still loading");
        }
        let seg = match s.state.tempo.segments.get(index) {
            Some(g) if (g.start_ms - at).abs() < 1e-6 => g.clone(),
            _ => return refuse(id, "stale", "The tempo sections changed; try again"),
        };
        let rate = s.pcm.rate;
        let end_ms = s.pcm.frames as f64 * 1000.0 / f64::from(rate.max(1));
        let (from, to) = match section_bpm::piece(&s.state.tempo, index, s.pos_ms_f(), end_ms) {
            Ok(p) => p,
            Err(m) => return refuse(id, "too-short", &m),
        };
        let (from_f, to_f) = section_bpm::piece_frames(rate, s.pcm.frames, from, to);
        let ms_of = |f: u64| f as f64 * 1000.0 / f64::from(rate.max(1));
        self.bpm_serial += 1;
        let serial = self.bpm_serial;
        let source = s.state.source.clone().unwrap_or_default();
        let view = BpmDetectView {
            serial,
            index,
            at,
            from_ms: ms_of(from_f),
            to_ms: ms_of(to_f),
            source,
            phase: crate::beat_detect::DetectPhase::Checking,
        };
        eprintln!(
            "calliope: player detect id={id} index={index} source={} from_ms={} to_ms={} frames={} bpm={}",
            view.source,
            view.from_ms as u64,
            view.to_ms as u64,
            to_f - from_f,
            seg.bpm
        );
        let shared = BpmShared::new();
        let handle = section_bpm::spawn(
            BpmJob {
                id: id.to_string(),
                dir: s.dir.clone(),
                serial,
                view: view.clone(),
                pcm: s.pcm.clone(),
                from_f,
                to_f,
                beats_per_bar: s.state.tempo.beats_per_bar,
                params,
            },
            shared.clone(),
            self.sink.clone(),
        );
        s.bpm_detect = Some(BpmRun {
            id: id.to_string(),
            view,
            old_bpm: seg.bpm,
            shared,
            handle: Some(handle),
        });
        Ok(s.snapshot())
    }

    /// Cancels the run at once: its result can no longer be applied.
    pub fn cancel_bpm_detect(&mut self, id: &str) -> Result<PlayerSnapshot, String> {
        let s = self.session(id)?;
        if let Some(run) = s.bpm_detect.take() {
            run.shared.cancel.store(true, Ordering::SeqCst);
            eprintln!("calliope: player detect cancelled id={id} reason=button");
        }
        Ok(s.snapshot())
    }

    /// Takes a finished run and applies or reports its outcome (called by `tick`).
    pub(super) fn reap_bpm_detect(&mut self, now: Instant) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        if !s
            .bpm_detect
            .as_ref()
            .is_some_and(|r| r.shared.is_finished())
        {
            return;
        }
        let run = s.bpm_detect.take().expect("checked above");
        run.shared.cancel.store(true, Ordering::SeqCst); // the thread is done: no "closed" log
        let id = s.id.clone();
        let (index, old_bpm) = (run.view.index, run.old_bpm);
        let result = |outcome, new_bpm, hint, message: Option<String>| BpmResult {
            serial: run.view.serial,
            index,
            outcome,
            old_bpm,
            new_bpm,
            from_ms: run.view.from_ms,
            to_ms: run.view.to_ms,
            hint,
            message,
        };
        let emit = |sink: &EventSink, r: BpmResult, snapshot| {
            sink(PlayerEvent::BpmDetected {
                id: id.clone(),
                result: r,
                snapshot,
            })
        };
        let sink = self.sink.clone();
        match run.shared.outcome() {
            Some(BpmOutcome::Done { bpm, beats }) => {
                let hint = section_bpm::hint(old_bpm, bpm);
                let hint_name = hint.map_or("none", section_bpm::Hint::name);
                let unchanged =
                    s.state.tempo.segments.get(index).is_some_and(|g| {
                        (g.start_ms - run.view.at).abs() < 1e-6 && g.bpm == old_bpm
                    });
                let log = |applied: &str| {
                    eprintln!(
                        "calliope: player detected id={id} index={index} beats={beats} old={old_bpm} new={bpm} hint={hint_name} applied={applied}"
                    );
                };
                if !unchanged {
                    log("false reason=changed");
                    let r = result(ResultKind::Dropped, None, None, Some(CHANGED.into()));
                    emit(&sink, r, None);
                    return;
                }
                let edit = SectionEdit::Bpm {
                    index,
                    at: run.view.at,
                    bpm,
                };
                match self.edit_sections(&id, edit, now) {
                    Ok(snap) => {
                        log("true");
                        let r = result(ResultKind::Applied, Some(bpm), hint, None);
                        emit(&sink, r, Some(Box::new(snap)));
                    }
                    Err(m) => {
                        eprintln!("calliope: player detect failed id={id} message={m}");
                        emit(&sink, result(ResultKind::Failed, None, None, Some(m)), None);
                    }
                }
            }
            Some(BpmOutcome::Failed(m)) => {
                eprintln!("calliope: player detect failed id={id} message={m}");
                emit(&sink, result(ResultKind::Failed, None, None, Some(m)), None);
            }
            Some(BpmOutcome::Cancelled) | None => {}
        }
    }

    /// Test hook: registers a run with no thread; the test ends it with `shared.finish(..)`.
    #[cfg(test)]
    pub(crate) fn test_fake_bpm_detect(&mut self, id: &str, index: usize) -> Arc<BpmShared> {
        self.bpm_serial += 1;
        let serial = self.bpm_serial;
        let s = self.session(id).unwrap();
        let seg = s.state.tempo.segments[index].clone();
        let shared = BpmShared::new();
        s.bpm_detect = Some(BpmRun {
            id: id.to_string(),
            view: BpmDetectView {
                serial,
                index,
                at: seg.start_ms,
                from_ms: seg.start_ms,
                to_ms: seg.start_ms + 1000.0,
                source: s.state.source.clone().unwrap_or_default(),
                phase: crate::beat_detect::DetectPhase::Checking,
            },
            old_bpm: seg.bpm,
            shared: shared.clone(),
            handle: None,
        });
        shared
    }
}
