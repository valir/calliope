//! Gain, mixing and quantising rules shared by playback and Save (plan 2.3).

#![allow(dead_code)] // used by the editor session and the render job (tasks 7 and 9) and the tests

use crate::stem_audio::StemPcm;

pub const MIN_DB: f32 = -60.0;
pub const MAX_DB: f32 = 12.0;
pub const STEP_DB: f32 = 0.5;

/// Linear gain for a stem: 0 when Off (None or at/below `MIN_DB`) or not unmuted.
pub fn gain_factor(gain_db: Option<f32>, unmuted: bool) -> f32 {
    match gain_db {
        Some(db) if unmuted && db > MIN_DB => 10f32.powf(db / 20.0),
        _ => 0.0,
    }
}

/// Snaps to the 0.5 dB grid (half away from zero) and the range. `None` means Off.
pub fn clamp_db(db: f32) -> Option<f32> {
    if db.is_nan() {
        return None;
    }
    let snapped = ((db / STEP_DB).round() * STEP_DB).min(MAX_DB);
    if snapped <= MIN_DB {
        None
    } else {
        Some(snapped)
    }
}

/// Fills `scratch[..frames * 2]` with interleaved stereo f32 from `stem` starting at
/// `start_frame`: i16 / 32768, mono copied to both channels, zeros past the end.
pub fn to_stereo_f32(stem: &StemPcm, start_frame: u64, frames: usize, scratch: &mut [f32]) {
    let ch = stem.channels as usize;
    let available = stem.frames.saturating_sub(start_frame).min(frames as u64) as usize;
    let start = start_frame.min(stem.frames) as usize;
    for i in 0..available {
        let base = (start + i) * ch;
        let l = f32::from(stem.samples[base]) / 32768.0;
        let r = if ch >= 2 {
            f32::from(stem.samples[base + 1]) / 32768.0
        } else {
            l
        };
        scratch[2 * i] = l;
        scratch[2 * i + 1] = r;
    }
    scratch[2 * available..2 * frames].fill(0.0);
}

/// Sums interleaved-stereo blocks with their gains into `out`, hard-clips to [-1, 1] and
/// returns the number of clipped samples. Each input must be at least `out.len()` long.
pub fn mix(inputs: &[&[f32]], gains: &[f32], out: &mut [f32]) -> u32 {
    out.fill(0.0);
    for (input, &gain) in inputs.iter().zip(gains) {
        if gain == 0.0 {
            continue;
        }
        for (o, &s) in out.iter_mut().zip(input.iter()) {
            *o += s * gain;
        }
    }
    let mut clipped = 0;
    for o in out.iter_mut() {
        if *o > 1.0 {
            *o = 1.0;
            clipped += 1;
        } else if *o < -1.0 {
            *o = -1.0;
            clipped += 1;
        }
    }
    clipped
}

/// Rounds a [-1, 1] sample to a signed integer of `bits` bits, clamping to its range.
pub fn quantise(x: f32, bits: u32) -> i32 {
    let scale = (1i64 << (bits - 1)) as f64;
    let v = (f64::from(x) * scale).round();
    v.clamp(-scale, scale - 1.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(channels: u32, samples: Vec<i16>) -> StemPcm {
        StemPcm {
            channels,
            frames: samples.len() as u64 / u64::from(channels),
            samples,
        }
    }

    #[test]
    fn gain_factor_values() {
        assert_eq!(gain_factor(Some(0.0), true), 1.0);
        assert!((gain_factor(Some(-6.0), true) - 0.501187).abs() < 1e-6);
        assert!((gain_factor(Some(12.0), true) - 3.981072).abs() < 1e-5);
        assert_eq!(gain_factor(None, true), 0.0);
        assert_eq!(gain_factor(Some(-60.0), true), 0.0);
        assert_eq!(gain_factor(Some(0.0), false), 0.0);
    }

    #[test]
    fn clamp_db_snaps_and_limits() {
        assert_eq!(clamp_db(13.0), Some(12.0));
        assert_eq!(clamp_db(-0.3), Some(-0.5));
        assert_eq!(clamp_db(0.25), Some(0.5));
        assert_eq!(clamp_db(-0.25), Some(-0.5));
        assert_eq!(clamp_db(-80.0), None);
        assert_eq!(clamp_db(-60.0), None);
        assert_eq!(clamp_db(-59.5), Some(-59.5));
    }

    #[test]
    fn mono_is_copied_to_both_channels() {
        let stem = pcm(1, vec![16384, -16384]);
        let mut s = [9.0; 4];
        to_stereo_f32(&stem, 0, 2, &mut s);
        assert_eq!(s, [0.5, 0.5, -0.5, -0.5]);
    }

    #[test]
    fn stereo_and_offset() {
        let stem = pcm(2, vec![0, 0, 16384, -32768]);
        let mut s = [9.0; 2];
        to_stereo_f32(&stem, 1, 1, &mut s);
        assert_eq!(s, [0.5, -1.0]);
    }

    #[test]
    fn frames_past_the_end_are_zero() {
        let stem = pcm(2, vec![16384, 16384]);
        let mut s = [9.0; 6];
        to_stereo_f32(&stem, 0, 3, &mut s);
        assert_eq!(s, [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        to_stereo_f32(&stem, 10, 2, &mut s);
        assert_eq!(&s[..4], &[0.0; 4]);
    }

    #[test]
    fn two_stems_sum() {
        let a = [0.25f32, -0.25, 0.1, 0.0];
        let b = [0.25f32, 0.5, 0.1, 0.0];
        let mut out = [0.0; 4];
        let clipped = mix(&[&a, &b], &[1.0, 1.0], &mut out);
        assert_eq!(clipped, 0);
        assert_eq!(out, [0.5, 0.25, 0.2, 0.0]);
    }

    #[test]
    fn mix_clips_and_counts() {
        let a = [0.5f32, -0.5, 0.1, 0.5];
        let g = gain_factor(Some(12.0), true);
        let mut out = [0.0; 4];
        let clipped = mix(&[&a], &[g], &mut out);
        assert_eq!(clipped, 3);
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], -1.0);
        assert!((out[2] - 0.398107).abs() < 1e-5);
        assert_eq!(out[3], 1.0);
    }

    #[test]
    fn quantise_16_and_24() {
        assert_eq!(quantise(1.0, 16), 32767);
        assert_eq!(quantise(-1.0, 16), -32768);
        assert_eq!(quantise(0.5, 16), 16384);
        assert_eq!(quantise(0.0, 16), 0);
        assert_eq!(quantise(1.0, 24), 8_388_607);
        assert_eq!(quantise(-1.0, 24), -8_388_608);
        assert_eq!(quantise(0.5, 24), 4_194_304);
    }
}
