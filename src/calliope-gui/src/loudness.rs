//! Loudness (ITU-R BS.1770-4 integrated loudness and true peak), the gain rule of "normalize to
//! -14 LUFS with a -1 dBTP ceiling" and the values the UI and `track.json` record. Pure, no IO.
//! This is the only module that names the `ebur128` crate.

use ebur128::{EbuR128, Mode};
use serde::Serialize;

use crate::track_meta::{self, LoudnessRecord};

pub const TARGET_LUFS: f64 = -14.0;
pub const CEILING_DBTP: f64 = -1.0;
/// "Already normalized" tolerance, in LU (and dB for the ceiling).
pub const TOLERANCE: f64 = 0.1;
/// Below this the gated loudness is undefined (BS.1770 absolute gate); shown in the "too quiet" text.
pub const GATE_LUFS: f64 = -70.0;

/// BS.1770-4 meter for interleaved stereo f32 (values beyond +-1 allowed: the unclipped mix).
pub struct Meter {
    inner: EbuR128,
}

impl Meter {
    pub fn new(sample_rate: u32) -> Result<Meter, String> {
        EbuR128::new(2, sample_rate, Mode::I | Mode::TRUE_PEAK)
            .map(|inner| Meter { inner })
            .map_err(|e| format!("Cannot measure the loudness at {sample_rate} Hz: {e}"))
    }

    pub fn add(&mut self, interleaved: &[f32]) -> Result<(), String> {
        self.inner
            .add_frames_f32(interleaved)
            .map_err(|e| format!("Cannot measure the loudness: {e}"))
    }

    pub fn finish(&self) -> Measure {
        let lufs = self
            .inner
            .loudness_global()
            .ok()
            .filter(|l| l.is_finite() && *l >= GATE_LUFS);
        let peak = (0..2)
            .filter_map(|c| self.inner.true_peak(c).ok())
            .fold(0.0f64, f64::max);
        let true_peak = (peak > 0.0 && peak.is_finite()).then(|| 20.0 * peak.log10());
        Measure { lufs, true_peak }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    /// Integrated loudness, LUFS; None when no block passes the gates (silence, below -70 LUFS,
    /// or under 400 ms).
    pub lufs: Option<f64>,
    /// Max true peak over both channels, dBTP; None for digital silence.
    pub true_peak: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Normalized,
    Limited,
    AtTarget,
    AtCeiling,
    TooQuiet,
}

impl Outcome {
    /// The kebab-case name (log lines, the same as the serialised form).
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Normalized => "normalized",
            Outcome::Limited => "limited",
            Outcome::AtTarget => "at-target",
            Outcome::AtCeiling => "at-ceiling",
            Outcome::TooQuiet => "too-quiet",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub outcome: Outcome,
    pub gain_db: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Save,
    Normalize,
}

/// The gain rule (plan 2.3): `g = min(target - L, ceiling - TP)`.
pub fn decide(m: Measure, purpose: Purpose) -> Decision {
    let (Some(l), Some(tp)) = (m.lufs, m.true_peak) else {
        return Decision {
            outcome: Outcome::TooQuiet,
            gain_db: 0.0,
        };
    };
    let g_t = TARGET_LUFS - l;
    let g_c = CEILING_DBTP - tp;
    let g = g_t.min(g_c);
    if purpose == Purpose::Normalize && g.abs() <= TOLERANCE + 1e-9 {
        let outcome = if g_t <= g_c {
            Outcome::AtTarget
        } else {
            Outcome::AtCeiling
        };
        return Decision {
            outcome,
            gain_db: 0.0,
        };
    }
    let outcome = if g_c < g_t {
        Outcome::Limited
    } else {
        Outcome::Normalized
    };
    Decision {
        outcome,
        gain_db: g,
    }
}

/// The one place a dB gain becomes the linear factor (f64 maths, then `as f32`).
pub fn db_to_gain(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

/// What the UI shows (IPC payload): the file's values after the operation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoudnessResult {
    pub outcome: Outcome,
    pub lufs: Option<f64>,
    pub true_peak: Option<f64>,
    pub gain_db: f64,
    pub target: f64,
    pub ceiling: f64,
}

pub fn result_of(m: Measure, d: Decision) -> LoudnessResult {
    LoudnessResult {
        outcome: d.outcome,
        lufs: m.lufs.map(|v| v + d.gain_db),
        true_peak: m.true_peak.map(|v| v + d.gain_db),
        gain_db: d.gain_db,
        target: TARGET_LUFS,
        ceiling: CEILING_DBTP,
    }
}

/// The `track.json` entry for a file measured as `m` and then scaled by `d`.
pub fn record_of(m: Measure, d: Decision, md5: [u8; 16]) -> LoudnessRecord {
    LoudnessRecord {
        lufs: m.lufs.map(|v| v + d.gain_db),
        true_peak: m.true_peak.map(|v| v + d.gain_db),
        gain_db: d.gain_db,
        target: TARGET_LUFS,
        ceiling: CEILING_DBTP,
        md5: track_meta::md5_hex(md5),
    }
}

#[cfg(test)]
#[path = "loudness_tests.rs"]
mod tests;
