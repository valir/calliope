use super::*;
use crate::player_source::{peaks, Pcm};

fn stem(channels: u32, samples: Vec<i16>) -> StemPcm {
    StemPcm {
        channels,
        frames: samples.len() as u64 / u64::from(channels),
        samples,
    }
}

fn pcm_of(s: &StemPcm) -> Pcm {
    Pcm {
        rate: 44100,
        channels: s.channels,
        frames: s.frames,
        samples: s.samples.clone(),
    }
}

fn wave(n: usize, ch: usize, amp: f32) -> Vec<i16> {
    (0..n * ch)
        .map(|i| (((i as f32) * 0.37).sin() * amp * 32767.0) as i16)
        .collect()
}

fn run(stems: &[StemPcm], gains: &[f32], buckets: usize) -> Vec<f32> {
    let total = stems.iter().map(|s| s.frames).max().unwrap_or(0);
    mix_peaks(stems, gains, total, buckets, &AtomicBool::new(false)).unwrap()
}

#[test]
fn one_stem_at_unity_equals_player_peaks() {
    for (n, buckets) in [(10_000, 64), (9_999, 100), (50, 64)] {
        let s = stem(2, wave(n, 2, 0.9));
        assert_eq!(
            run(std::slice::from_ref(&s), &[1.0], buckets),
            peaks(&pcm_of(&s), buckets)
        );
    }
}

#[test]
fn two_stems_sum_with_gain_mono_widened_and_clipped() {
    let a = stem(2, vec![8192, 0, 8192, 0]); // 0.25
    let b = stem(1, vec![16384, -16384]); // 0.5 both channels, then short: zeros after
    let g = mixer::gain_factor(Some(-6.0), true);
    let p = run(&[a.clone(), b.clone()], &[1.0, g], 1);
    assert!((p[0] - (0.25 + 0.5 * g)).abs() < 1e-6, "{p:?}");
    // clipping
    let loud = stem(2, vec![30000, 30000]);
    assert_eq!(run(&[loud.clone(), loud], &[1.0, 1.0], 1), vec![1.0]);
}

#[test]
fn zero_gains_give_zeros_without_iterating() {
    let s = stem(2, wave(10_000, 2, 0.5));
    // already cancelled: only the early exit can return Some
    let p = mix_peaks(&[s], &[0.0], 10_000, 8, &AtomicBool::new(true));
    assert_eq!(p, Some(vec![0.0; 8]));
}

#[test]
fn cancel_gives_none() {
    let s = stem(2, wave(10_000, 2, 0.5));
    assert_eq!(
        mix_peaks(&[s], &[1.0], 10_000, 8, &AtomicBool::new(true)),
        None
    );
}

#[test]
fn debounce_fires_once_after_300ms_and_restarts_on_touch() {
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let mut d = Debounce::new();
    assert!(!d.take_due(t0 + ms(1000)));
    d.touch(t0);
    assert!(!d.take_due(t0 + ms(299)));
    assert!(d.take_due(t0 + ms(300)));
    assert!(!d.take_due(t0 + ms(301)));
    d.touch(t0);
    d.touch(t0 + ms(200));
    assert!(!d.take_due(t0 + ms(499)));
    assert!(d.take_due(t0 + ms(500)));
    assert!(!d.is_pending());
}
