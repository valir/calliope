//! The normalizing step of every import (gui-import-normalize, plan 2.3-2.5): measures the
//! converted `audio.flac` and, unless it is already at the target or too quiet or short, writes
//! a scaled 24-bit copy `normalized.flac` next to it. `audio.flac` is never modified, so a mix
//! can never be normalized twice; a small `normalize.json` marker lets a resume reuse the result.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::backing_render::{measure_mix, render_flac, RenderInput, RenderStem, CANCELLED};
use crate::fsutil;
use crate::loudness::{self, Decision, LoudnessResult, Measure, Outcome, Purpose};
use crate::stem_audio::StemReader;
use crate::track_meta::md5_hex;

pub const NORMALIZED_FLAC: &str = "normalized.flac";
pub const MARKER_FILE: &str = "normalize.json";
pub const PART_FILE: &str = ".normalized.flac.part";
/// The normalized copy is always written as 24-bit FLAC.
pub const OUTPUT_BITS: u32 = 24;

const CONVERTED_FILE: &str = "audio.flac";

#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    /// The mix the import goes on with: `<dir>/normalized.flac`, or `converted` itself when unchanged.
    pub audio: PathBuf,
    /// What the meter said about the converted file.
    pub measured: Measure,
    pub decision: Decision,
    /// `loudness::result_of(measured, decision)`: the values of `audio`.
    pub result: LoudnessResult,
    /// A new file was written (outcome normalized or limited).
    pub changed: bool,
    /// Taken from a valid `normalize.json`: nothing was measured or written.
    pub resumed: bool,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    schema: u32,
    source_md5: String,
    file: String,
    md5: String,
    outcome: String,
    measured_lufs: Option<f64>,
    measured_tp: Option<f64>,
    gain_db: f64,
    target: f64,
    ceiling: f64,
}

const OUTCOMES: [Outcome; 5] = [
    Outcome::Normalized,
    Outcome::Limited,
    Outcome::AtTarget,
    Outcome::AtCeiling,
    Outcome::TooQuiet,
];

fn is_changed(o: Outcome) -> bool {
    matches!(o, Outcome::Normalized | Outcome::Limited)
}

fn is_regular(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

/// The STREAMINFO MD5 of a FLAC file (only the header is read).
fn header_md5(path: &Path) -> Option<[u8; 16]> {
    StemReader::open(path).ok().map(|r| r.md5())
}

/// The result recorded by an earlier run in `dir`, if `normalize.json` is valid for `converted`.
pub fn resume(dir: &Path, converted: &Path) -> Option<Normalized> {
    let marker_path = dir.join(MARKER_FILE);
    if !is_regular(&marker_path) {
        return None;
    }
    let m: Marker = serde_json::from_slice(&fs::read(&marker_path).ok()?).ok()?;
    if m.schema != 1 || m.target != loudness::TARGET_LUFS || m.ceiling != loudness::CEILING_DBTP {
        return None;
    }
    let outcome = OUTCOMES.into_iter().find(|o| o.as_str() == m.outcome)?;
    let changed = is_changed(outcome);
    let want_file = if changed {
        NORMALIZED_FLAC
    } else {
        CONVERTED_FILE
    };
    if m.file != want_file {
        return None;
    }
    let source_md5 = header_md5(converted)?;
    if source_md5 == [0; 16] || md5_hex(source_md5) != m.source_md5 {
        return None;
    }
    let audio = dir.join(want_file);
    if !is_regular(&audio) || md5_hex(header_md5(&audio)?) != m.md5 {
        return None;
    }
    if !changed && m.md5 != m.source_md5 {
        return None;
    }
    let measured = Measure {
        lufs: m.measured_lufs,
        true_peak: m.measured_tp,
    };
    let decision = Decision {
        outcome,
        gain_db: if changed { m.gain_db } else { 0.0 },
    };
    Some(Normalized {
        audio,
        measured,
        decision,
        result: loudness::result_of(measured, decision),
        changed,
        resumed: true,
    })
}

fn remove_stale(path: &Path) -> Result<(), String> {
    if is_regular(path) {
        fs::remove_file(path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.to_string())
    } else {
        Ok(())
    }
}

/// `converted` is `<dir>/audio.flac`. `progress` gets 0.0..0.4 while measuring and 0.4..1.0 while
/// writing. A set `cancel` returns `Err(backing_render::CANCELLED)`.
pub fn normalize(
    dir: &Path,
    converted: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Normalized, String> {
    check_cancel(cancel)?;
    if let Some(n) = resume(dir, converted) {
        return Ok(n);
    }
    let marker_path = dir.join(MARKER_FILE);
    let out = dir.join(NORMALIZED_FLAC);
    remove_stale(&marker_path)?;
    remove_stale(&out)?;
    let (rate, frames, source_md5) = {
        let reader = StemReader::open(converted)
            .map_err(|_| "the converted audio has an unsupported format".to_string())?;
        if reader.channels() != 2 || !matches!(reader.bits(), 16 | 24) {
            return Err("the converted audio has an unsupported format".into());
        }
        let frames = reader
            .frames()
            .filter(|f| *f > 0)
            .ok_or("the converted audio has no length in its header")?;
        (reader.sample_rate(), frames, reader.md5())
    };
    let mut input = RenderInput {
        stems: vec![RenderStem {
            path: converted.to_path_buf(),
            gain: 1.0,
        }],
        sample_rate: rate,
        bits: OUTPUT_BITS,
        frames,
    };
    let m = measure_mix(&input, cancel, &mut |f| progress(f * 0.4))?;
    check_cancel(cancel)?;
    let d = loudness::decide(m, Purpose::Normalize);
    let changed = is_changed(d.outcome);
    let (audio, md5) = if changed {
        let part = dir.join(PART_FILE);
        match fs::symlink_metadata(&part) {
            Ok(meta) if meta.is_file() => {
                fs::remove_file(&part).map_err(|e| format!("cannot clear {PART_FILE}: {e}"))?
            }
            Ok(_) => return Err(format!("cannot clear {PART_FILE}")),
            Err(_) => {}
        }
        input.stems[0].gain = loudness::db_to_gain(d.gain_db);
        let stats = render_flac(&input, &part, cancel, &mut |f| progress(0.4 + f * 0.6))?;
        if let Err(e) = check_cancel(cancel) {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        if stats.clipped_samples > 0 {
            eprintln!(
                "calliope: import normalize clipped={} samples",
                stats.clipped_samples
            );
        }
        if let Err(e) = fs::rename(&part, &out) {
            let _ = fs::remove_file(&part);
            return Err(format!("cannot move {PART_FILE} into place: {e}"));
        }
        (out, stats.md5)
    } else {
        (converted.to_path_buf(), source_md5)
    };
    let marker = Marker {
        schema: 1,
        source_md5: md5_hex(source_md5),
        file: if changed {
            NORMALIZED_FLAC
        } else {
            CONVERTED_FILE
        }
        .to_string(),
        md5: md5_hex(md5),
        outcome: d.outcome.as_str().to_string(),
        measured_lufs: m.lufs,
        measured_tp: m.true_peak,
        gain_db: d.gain_db,
        target: loudness::TARGET_LUFS,
        ceiling: loudness::CEILING_DBTP,
    };
    let bytes = serde_json::to_vec_pretty(&marker).map_err(|e| e.to_string())?;
    fsutil::write_atomic(&marker_path, &bytes)
        .map_err(|e| format!("cannot write {MARKER_FILE}: {e}"))?;
    Ok(Normalized {
        audio,
        measured: m,
        decision: d,
        result: loudness::result_of(m, d),
        changed,
        resumed: false,
    })
}

#[cfg(test)]
#[path = "import_normalize_tests.rs"]
mod tests;
