//! Metronome click sounds, count-in plan and beat placement. Pure logic, no IO.

use std::f32::consts::TAU;

pub(crate) const CLICK_PEAK: f32 = 0.5;
pub(crate) const ACCENT_HZ: f32 = 2000.0;
pub(crate) const NORMAL_HZ: f32 = 1500.0;
const CLICK_SECONDS: f32 = 0.030;
/// Decay time constant: `exp(-30 ms / tau)` is about 0.0067, so the click is below 0.01 at 30 ms.
const DECAY_TAU: f32 = 0.006;

/// The two pre-rendered click sounds at the output rate.
#[derive(Debug, Clone)]
pub(crate) struct Click {
    accent: Vec<f32>,
    normal: Vec<f32>,
}

fn render(hz: f32, rate: u32) -> Vec<f32> {
    let n = ((CLICK_SECONDS * rate as f32).round() as usize).max(1);
    let mut v: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            (TAU * hz * t).sin() * (-t / DECAY_TAU).exp()
        })
        .collect();
    let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        for s in &mut v {
            *s *= CLICK_PEAK / peak;
        }
    }
    v
}

impl Click {
    pub(crate) fn new(rate: u32) -> Self {
        Click {
            accent: render(ACCENT_HZ, rate),
            normal: render(NORMAL_HZ, rate),
        }
    }

    pub(crate) fn samples(&self, accent: bool) -> &[f32] {
        if accent {
            &self.accent
        } else {
            &self.normal
        }
    }
}

/// Output frames of the three audible count-in clicks and of the audio start (the silent 4th beat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountIn {
    pub clicks: [u64; 3],
    pub start: u64,
}

pub(crate) fn count_in_plan(bpm: f64, rate: u32) -> CountIn {
    let b = (rate as f64 * 60.0 / bpm).round() as u64;
    CountIn {
        clicks: [0, b, 2 * b],
        start: 3 * b,
    }
}

/// Beats (source frame, accent) that fall inside the output block starting at source frame
/// `block_start` and `block_len` output frames long, as (output offset, accent). At `speed`
/// the block covers `block_len * speed` source frames. `beats` must be sorted.
pub(crate) fn place(
    beats: &[(u64, bool)],
    block_start: u64,
    block_len: usize,
    speed: f64,
) -> impl Iterator<Item = (usize, bool)> + '_ {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    let first = beats.partition_point(|b| b.0 < block_start);
    beats[first..]
        .iter()
        .map(move |&(f, a)| ((((f - block_start) as f64) / speed).ceil() as usize, a))
        .take_while(move |&(off, _)| off < block_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goertzel(x: &[f32], hz: f32, rate: u32) -> f32 {
        let w = TAU * hz / rate as f32;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &v in x {
            let s = v + c * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        s1 * s1 + s2 * s2 - c * s1 * s2
    }

    #[test]
    fn count_in_at_120_bpm() {
        let p = count_in_plan(120.0, 22050);
        assert_eq!(p.clicks, [0, 11025, 22050]);
        assert_eq!(p.start, 33075);
    }

    #[test]
    fn click_peak_and_decay() {
        let rate = 22050;
        let c = Click::new(rate);
        for accent in [true, false] {
            let s = c.samples(accent);
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((peak - 0.5).abs() < 1e-4);
            let n30 = (0.030 * rate as f32).round() as usize;
            assert!(s.len() <= n30 + 1);
            assert!(s.iter().skip(n30.saturating_sub(1)).all(|v| v.abs() < 0.01));
        }
    }

    #[test]
    fn accent_and_normal_differ_in_frequency() {
        let rate = 22050;
        let c = Click::new(rate);
        let (a, n) = (c.samples(true), c.samples(false));
        assert!(goertzel(a, 2000.0, rate) > 3.0 * goertzel(a, 1500.0, rate));
        assert!(goertzel(n, 1500.0, rate) > 3.0 * goertzel(n, 2000.0, rate));
    }

    #[test]
    fn place_only_beats_in_block() {
        let beats = [(0, true), (100, false), (200, false), (300, true)];
        let got: Vec<_> = place(&beats, 100, 100, 1.0).collect();
        assert_eq!(got, vec![(0, false)]);
        let got: Vec<_> = place(&beats, 50, 200, 1.0).collect();
        assert_eq!(got, vec![(50, false), (150, false)]);
        assert_eq!(place(&beats, 301, 100, 1.0).count(), 0);
    }

    #[test]
    fn place_scales_with_speed() {
        let beats = [(100, true), (300, false)];
        // speed 2: 100 output frames cover 200 source frames
        let got: Vec<_> = place(&beats, 0, 100, 2.0).collect();
        assert_eq!(got, vec![(50, true)]);
    }
}
