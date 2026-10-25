//! Where the music starts: a level-onset detector on the track's backing audio. Pure logic over
//! 16-bit PCM, no IO.
//!
//! The start is the first 10 ms window whose power rises above a threshold 30 dB under the
//! track's loudest 50 ms and stays there (at least 3 of the next 5 windows), then refined to
//! 1 ms. It is a level test relative to the loudest part, not beat tracking: leading silence or
//! steady noise stays below it, an isolated click does not count as music.

use crate::player_source::{Pcm, Source};

/// Threshold below the loudest 50 ms (power ratio 10^(-3)).
pub const LEVEL_DB: f64 = -30.0;
/// Envelope window.
pub const WINDOW_MS: f64 = 10.0;
/// The "loudest part" is the loudest 50 ms (5 windows).
pub const LOUD_WINDOWS: usize = 5;
/// At least this many of the 5 windows from the onset are above the threshold.
pub const SUSTAIN: usize = 3;
/// Onset refinement window.
pub const REFINE_MS: f64 = 1.0;
/// Refinement level below the threshold.
pub const REFINE_DB: f64 = -6.0;

fn db_ratio(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

/// Mean power (full scale = 1) of frames `a..b`, averaged over the channels.
fn power(pcm: &Pcm, a: usize, b: usize) -> f64 {
    let ch = pcm.channels.max(1) as usize;
    let b = b.min(pcm.frames as usize);
    if a >= b {
        return 0.0;
    }
    let sum: f64 = pcm.samples[a * ch..b * ch]
        .iter()
        .map(|&s| {
            let v = f64::from(s) / 32768.0;
            v * v
        })
        .sum();
    sum / ((b - a) * ch) as f64
}

/// Where the music starts, in ms from the file start; 0.0 when nothing rises (silence, music from
/// the first window, or steady noise only).
pub fn find_start(pcm: &Pcm) -> f64 {
    let rate = f64::from(pcm.rate);
    let frames = (pcm.frames as usize).min(pcm.samples.len() / pcm.channels.max(1) as usize);
    let w = ((rate * WINDOW_MS / 1000.0).round() as usize).max(1);
    if frames == 0 || pcm.rate == 0 {
        return 0.0;
    }
    let p: Vec<f64> = (0..frames.div_ceil(w))
        .map(|k| power(pcm, k * w, (k + 1) * w))
        .collect();
    let n = p.len();
    let mean5 = |k: usize| {
        let s = &p[k..(k + LOUD_WINDOWS).min(n)];
        s.iter().sum::<f64>() / s.len() as f64
    };
    let loud = (0..n).map(mean5).fold(0.0, f64::max);
    if loud <= 0.0 {
        return 0.0;
    }
    let thr = loud * db_ratio(LEVEL_DB);
    let onset = (0..n).find(|&k| {
        p[k] >= thr && {
            let s = &p[k..(k + LOUD_WINDOWS).min(n)];
            s.iter().filter(|&&v| v >= thr).count() >= SUSTAIN.min(n - k)
        }
    });
    let Some(k) = onset else {
        return 0.0;
    };
    let sub = ((rate * REFINE_MS / 1000.0).round() as usize).max(1);
    let fine_thr = thr * db_ratio(REFINE_DB);
    let from = k.saturating_sub(1) * w;
    let to = ((k + 1) * w).min(frames);
    let f = (from..to)
        .step_by(sub)
        .find(|&a| power(pcm, a, a + sub) >= fine_thr)
        .unwrap_or(k * w);
    f as f64 * 1000.0 / rate
}

/// The source to analyse: the track's backing track (the first `backing:*`, else `audio`, else
/// `original`; all sources of a track are sample-aligned), else the source that is loaded.
pub fn analysis_source<'a>(sources: &'a [Source], loaded: &str) -> Option<&'a Source> {
    sources
        .iter()
        .find(|s| s.key.starts_with("backing:"))
        .or_else(|| sources.iter().find(|s| s.key == "audio"))
        .or_else(|| sources.iter().find(|s| s.key == "original"))
        .or_else(|| sources.iter().find(|s| s.key == loaded))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        /// Uniform in [-1, 1).
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as f64 / (1u64 << 31) as f64) * 2.0 - 1.0
        }
    }

    /// Stereo or mono PCM from a per-frame level function `f(t_seconds) -> (noise_amp, sine_amp)`.
    fn make(rate: u32, channels: u32, secs: f64, f: impl Fn(f64) -> (f64, f64)) -> Pcm {
        let frames = (secs * f64::from(rate)).round() as u64;
        let mut rng = Lcg(7);
        let mut samples = Vec::with_capacity((frames * u64::from(channels)) as usize);
        for i in 0..frames {
            let t = i as f64 / f64::from(rate);
            let (noise, sine) = f(t);
            for _ in 0..channels {
                let v = noise * rng.next() + sine * (2.0 * std::f64::consts::PI * 440.0 * t).sin();
                samples.push((v * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
            }
        }
        Pcm {
            rate,
            channels,
            frames,
            samples,
        }
    }

    /// Uniform noise amplitude for -50 dBFS RMS.
    const NOISE: f64 = 0.00316 * 1.732;

    fn noisy_intro(rate: u32, channels: u32) -> Pcm {
        make(rate, channels, 4.2, |t| {
            if t < 1.2 {
                (NOISE, 0.0)
            } else {
                (0.0, 0.5)
            }
        })
    }

    #[test]
    fn music_after_noise_is_found_within_3_ms() {
        for (rate, ch) in [(44100, 2), (48000, 1)] {
            let ms = find_start(&noisy_intro(rate, ch));
            assert!((ms - 1200.0).abs() < 3.0, "{rate} Hz x{ch}: {ms}");
        }
    }

    #[test]
    fn nothing_rises_gives_zero() {
        let sine = make(44100, 2, 2.0, |_| (0.0, 0.5));
        assert_eq!(find_start(&sine), 0.0);
        let silence = make(44100, 2, 1.0, |_| (0.0, 0.0));
        assert_eq!(find_start(&silence), 0.0);
        let noise = make(44100, 2, 2.0, |_| (NOISE, 0.0));
        assert_eq!(find_start(&noise), 0.0);
        let empty = Pcm {
            rate: 44100,
            channels: 2,
            frames: 0,
            samples: vec![],
        };
        assert_eq!(find_start(&empty), 0.0);
    }

    #[test]
    fn an_isolated_click_in_the_lead_in_is_ignored() {
        let mut pcm = noisy_intro(44100, 2);
        let a = (0.5 * 44100.0) as usize * 2;
        for s in &mut pcm.samples[a..a + 88 * 2] {
            *s = 32767;
        }
        let ms = find_start(&pcm);
        assert!((ms - 1200.0).abs() < 3.0, "{ms}");
    }

    #[test]
    fn a_quiet_intro_counts_when_within_30_db_of_the_loudest_part() {
        let pcm = make(44100, 2, 4.0, |t| {
            if t < 0.8 {
                (NOISE / 10.0, 0.0)
            } else if t < 2.0 {
                (0.0, 0.05)
            } else {
                (0.0, 0.5)
            }
        });
        let ms = find_start(&pcm);
        assert!((ms - 800.0).abs() < 3.0, "{ms}");
    }

    #[test]
    fn a_fade_in_is_found_where_it_crosses_30_db() {
        let pcm = make(44100, 2, 3.5, |t| {
            if t < 1.0 {
                (0.0, 0.0)
            } else {
                (0.0, ((t - 1.0).min(1.0)) * 0.99)
            }
        });
        let ms = find_start(&pcm);
        assert!((ms - 1031.6).abs() < 15.0, "{ms}");
    }

    #[test]
    fn analysis_source_prefers_the_backing_track() {
        let src = |key: &str| Source {
            key: key.into(),
            label: key.into(),
            file: format!("{key}.flac"),
        };
        let key = |l: &[Source], loaded: &str| analysis_source(l, loaded).map(|s| s.key.clone());
        let all = [
            src("stem:vocals"),
            src("original"),
            src("audio"),
            src("backing:b1"),
        ];
        assert_eq!(key(&all, "stem:vocals").as_deref(), Some("backing:b1"));
        assert_eq!(key(&all[..3], "original").as_deref(), Some("audio"));
        assert_eq!(key(&all[..2], "audio").as_deref(), Some("original"));
        assert_eq!(
            key(&all[..1], "stem:vocals").as_deref(),
            Some("stem:vocals")
        );
        assert_eq!(key(&[], "audio"), None);
    }
}
