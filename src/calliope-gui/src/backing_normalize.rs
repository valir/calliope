//! The Normalize job (plan 2.6): measures a saved backing file and, unless it is already at
//! the target, rewrites it with the gain that reaches -14 LUFS without passing the -1 dBTP
//! ceiling. The old file is kept in the trash; the stems and everything else stay as they are.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::backing_render::{
    fmt_opt, measure_mix, render_flac, reporter, PlayerGate, RenderInput, RenderStem, SaveShared,
    CANCELLED,
};
use crate::editor::{EditorEvent, EventSink, JobKind};
use crate::fsutil::FileStamp;
use crate::import_job::{locked, RepoLock};
use crate::loudness::{self, Decision, Measure, Outcome, Purpose};
use crate::repository::{BackingReplace, Repository, TrackRecord};
use crate::stem_audio::StemReader;
use crate::track_meta::{self, LoudnessRecord, TrackMeta};

const MISSING: &str = "The backing file is missing; save the backing first";
const CHANGED: &str = "conflict: the backing variant changed on disk; select the track again";

pub struct NormalizeParams {
    pub id: String,
    pub dir: PathBuf,
    /// The session's `VariantTarget.id`.
    pub variant: String,
    /// Its `backings/<x>.flac`.
    pub file: String,
    pub repo: Repository,
    pub lock: Arc<dyn RepoLock>,
    pub gate: Arc<dyn PlayerGate>,
    /// Test knob: slept per progress call.
    pub pace: Option<Duration>,
}

/// Starts the job on its own thread. Exactly one final event is sent (`normalized`,
/// `save-failed` or `save-cancelled`, `job: normalize`), after `shared` shows it finished.
pub fn spawn(p: NormalizeParams, shared: Arc<SaveShared>, sink: EventSink) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let id = p.id.clone();
        let part = p
            .dir
            .join(format!(".calliope-backing-{}.part", uuid::Uuid::now_v7()));
        // A panic must not leave the job "running" forever or the part file behind.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run(&p, &shared, &sink, &part)
        }))
        .unwrap_or_else(|_| Err("The normalize stopped unexpectedly".to_string()));
        let event = match result {
            Ok(ev) => ev,
            Err(msg) => {
                // The part may be missing: never created, or already renamed into place.
                let _ = fs::remove_file(&part);
                if msg == CANCELLED {
                    eprintln!("calliope: editor normalize cancelled id={id}");
                    EditorEvent::SaveCancelled {
                        id,
                        job: JobKind::Normalize,
                    }
                } else {
                    eprintln!("calliope: editor normalize failed id={id} message={msg}");
                    EditorEvent::SaveFailed {
                        id,
                        message: msg,
                        job: JobKind::Normalize,
                    }
                }
            }
        };
        shared.finished.store(true, Ordering::Release);
        sink(event);
    })
}

/// Whether the stored record may stand in for a measurement of the file with this MD5.
fn record_measure(rec: &LoudnessRecord, md5: [u8; 16]) -> Option<Measure> {
    let valid = rec.target == loudness::TARGET_LUFS
        && rec.ceiling == loudness::CEILING_DBTP
        && md5 != [0; 16]
        && rec.md5 == track_meta::md5_hex(md5);
    let m = Measure {
        lufs: Some(rec.lufs?),
        true_peak: Some(rec.true_peak?),
    };
    let at = matches!(
        loudness::decide(m, Purpose::Normalize).outcome,
        Outcome::AtTarget | Outcome::AtCeiling
    );
    (valid && at).then_some(m)
}

fn log_done(id: &str, file: &str, m: Measure, d: Decision, changed: bool, via: &str) {
    let r = loudness::result_of(m, d);
    eprintln!(
        "calliope: editor normalized id={id} file={file} outcome={} lufs={} tp={} gain={:.2} changed={changed} via={via}",
        d.outcome.as_str(),
        fmt_opt(r.lufs),
        fmt_opt(r.true_peak),
        r.gain_db
    );
}

fn run(
    p: &NormalizeParams,
    shared: &SaveShared,
    sink: &EventSink,
    part: &Path,
) -> Result<EditorEvent, String> {
    let cancelled = || shared.cancel.load(Ordering::Relaxed);
    let path = p.dir.join(&p.file);
    // 1. the file and its variant
    let before = FileStamp::of(&path).map_err(|_| MISSING.to_string())?;
    let meta: TrackMeta = fs::read(p.dir.join("track.json"))
        .map_err(|e| format!("Cannot read track.json: {e}"))
        .and_then(|b| {
            serde_json::from_slice(&b).map_err(|e| format!("Cannot read track.json: {e}"))
        })?;
    let variant = meta
        .backings
        .iter()
        .find(|b| b.id == p.variant && b.file == p.file)
        .ok_or_else(|| CHANGED.to_string())?;
    // 2. open it
    let (rate, bits, frames, md5) = {
        let reader = StemReader::open(&path)?;
        if reader.channels() != 2 {
            return Err("Only stereo backings can be normalized".into());
        }
        if !matches!(reader.bits(), 16 | 24) {
            return Err("Only 16 and 24 bit backings can be normalized".into());
        }
        let frames = reader
            .frames()
            .filter(|f| *f > 0)
            .ok_or("The backing file has no length in its header; save it again")?;
        (reader.sample_rate(), reader.bits(), frames, reader.md5())
    };
    let id = p.id.as_str();
    // 3. the record may stand in for the measurement
    if let Some(m) = variant
        .loudness
        .as_ref()
        .and_then(|rec| record_measure(rec, md5))
    {
        let d = loudness::decide(m, Purpose::Normalize);
        log_done(id, &p.file, m, d, false, "record");
        return Ok(EditorEvent::Normalized {
            id: p.id.clone(),
            file: p.file.clone(),
            loudness: loudness::result_of(m, d),
            changed: false,
            track: None,
            warnings: Vec::new(),
        });
    }
    // 4. measure
    let mut report = reporter(id, JobKind::Normalize, shared, sink, p.pace);
    let mut input = RenderInput {
        stems: vec![RenderStem {
            path: path.clone(),
            gain: 1.0,
        }],
        sample_rate: rate,
        bits,
        frames,
    };
    let m = measure_mix(&input, &shared.cancel, &mut |f| report(f * 0.4))?;
    let d = loudness::decide(m, Purpose::Normalize);
    if cancelled() {
        return Err(CANCELLED.into());
    }
    // 5. untouched: at most the record is written
    if matches!(
        d.outcome,
        Outcome::AtTarget | Outcome::AtCeiling | Outcome::TooQuiet
    ) {
        let rec = loudness::record_of(m, d, md5);
        let mut track: Option<TrackRecord> = None;
        if variant.loudness.as_ref() != Some(&rec) {
            let written = locked(&*p.lock, || {
                p.repo
                    .record_backing_loudness(id, &p.variant, &p.file, before, rec)
            })?;
            track = Some(written);
        }
        log_done(id, &p.file, m, d, false, "measure");
        return Ok(EditorEvent::Normalized {
            id: p.id.clone(),
            file: p.file.clone(),
            loudness: loudness::result_of(m, d),
            changed: false,
            track,
            warnings: Vec::new(),
        });
    }
    // 6. write the part with the gain
    input.stems[0].gain = loudness::db_to_gain(d.gain_db);
    let stats = render_flac(&input, part, &shared.cancel, &mut |f| report(0.4 + f * 0.6))?;
    if cancelled() {
        return Err(CANCELLED.into());
    }
    // 7. commit, with the Player released
    let rec = loudness::record_of(m, d, stats.md5);
    let mut replaced = None;
    p.gate.without_player(id, &mut || {
        let r = locked(&*p.lock, || {
            p.repo.replace_backing_audio(BackingReplace {
                id,
                variant: &p.variant,
                file: &p.file,
                part,
                before,
                loudness: rec.clone(),
            })
        })?;
        replaced = Some(r);
        Ok(())
    })?;
    let result = replaced.expect("the gate ran the commit");
    log_done(id, &p.file, m, d, true, "measure");
    Ok(EditorEvent::Normalized {
        id: p.id.clone(),
        file: p.file.clone(),
        loudness: loudness::result_of(m, d),
        changed: true,
        track: Some(result.track),
        warnings: result.warnings,
    })
}

#[cfg(test)]
#[path = "backing_normalize_tests.rs"]
mod tests;
