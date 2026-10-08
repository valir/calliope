//! Render and save-job tests: generated FLAC stems, claxon to decode the result, no audio device.

use super::*;
use crate::mixer;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use std::sync::atomic::AtomicBool;

const RATE: usize = 22050;

fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize) {
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, channels, bits, RATE);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

/// Deterministic pseudo-noise in the range of `bits`, scaled by `amp` (0..1).
fn noise(n: usize, bits: u32, amp: f64, seed: u64) -> Vec<i32> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let max = (1i64 << (bits - 1)) as f64 * amp;
    (0..n)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (((x >> 33) as f64 / (1u64 << 31) as f64 * 2.0 - 1.0) * max) as i32
        })
        .collect()
}

struct Stem {
    path: PathBuf,
    channels: usize,
    bits: u32,
    samples: Vec<i32>,
}

impl Stem {
    fn new(
        dir: &Path,
        name: &str,
        channels: usize,
        bits: u32,
        frames: usize,
        amp: f64,
        seed: u64,
    ) -> Stem {
        let samples = noise(frames * channels, bits, amp, seed);
        let path = dir.join(name);
        write_flac(&path, &samples, channels, bits as usize);
        Stem {
            path,
            channels,
            bits,
            samples,
        }
    }

    fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    /// Stereo f32 of frame `i`, zeros past the end.
    fn at(&self, i: usize) -> [f32; 2] {
        if i >= self.frames() {
            return [0.0; 2];
        }
        let scale = (1u64 << (self.bits - 1)) as f32;
        let l = self.samples[i * self.channels] as f32 / scale;
        let r = if self.channels == 2 {
            self.samples[i * 2 + 1] as f32 / scale
        } else {
            l
        };
        [l, r]
    }
}

/// Independent model of the render: sum in stem order, clamp, quantise.
fn expected(stems: &[(&Stem, f32)], frames: usize, bits: u32) -> (Vec<i32>, u64) {
    let mut out = Vec::new();
    let mut clipped = 0;
    for i in 0..frames {
        for c in 0..2 {
            let mut sum = 0.0f32;
            for (s, g) in stems {
                sum += s.at(i)[c] * g;
            }
            if !(-1.0..=1.0).contains(&sum) {
                clipped += 1;
            }
            out.push(mixer::quantise(sum.clamp(-1.0, 1.0), bits));
        }
    }
    (out, clipped)
}

fn decode(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    let samples = r.samples().map(|s| s.unwrap()).collect();
    (info, samples)
}

fn input(stems: &[(&Stem, f32)], frames: usize, bits: u32) -> RenderInput {
    RenderInput {
        stems: stems
            .iter()
            .map(|(s, g)| RenderStem {
                path: s.path.clone(),
                gain: *g,
            })
            .collect(),
        sample_rate: RATE as u32,
        bits,
        frames: frames as u64,
    }
}

fn render(input: &RenderInput, part: &Path) -> Result<RenderStats, String> {
    render_flac(input, part, &AtomicBool::new(false), &mut |_| {})
}

#[test]
fn exact_samples_gains_mono_stereo_and_padding() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 16, 10_000, 0.5, 1);
    let b = Stem::new(tmp.path(), "b.flac", 1, 16, 6_000, 0.7, 2); // mono, shorter
    let c = Stem::new(tmp.path(), "c.flac", 2, 16, 8_000, 0.4, 3);
    let unchecked = Stem::new(tmp.path(), "d.flac", 2, 16, 10_000, 0.9, 4);
    let _ = unchecked;
    let stems = [
        (&a, mixer::gain_factor(Some(0.0), true)),
        (&b, mixer::gain_factor(Some(-6.0), true)),
        (&c, mixer::gain_factor(Some(6.0), true)),
    ];
    let part = tmp.path().join("out.part");
    let stats = render(&input(&stems, 10_000, 16), &part).unwrap();
    let (info, got) = decode(&part);
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (22050, 2, 16)
    );
    assert_eq!(info.samples, Some(10_000));
    let (want, clipped) = expected(&stems, 10_000, 16);
    assert_eq!(got.len(), want.len());
    assert!(got == want, "samples differ");
    assert_eq!(stats.clipped_samples, clipped);
    // the padded tail is exactly stem a + c up to 8000, then a alone
    assert_eq!(got[2 * 9_999], mixer::quantise(a.at(9_999)[0], 16));
}

#[test]
fn twenty_four_bit_output() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 24, 5_000, 0.5, 7);
    let b = Stem::new(tmp.path(), "b.flac", 2, 16, 5_000, 0.5, 8);
    let stems = [(&a, 1.0f32), (&b, 0.5)];
    let part = tmp.path().join("out.part");
    render(&input(&stems, 5_000, 24), &part).unwrap();
    let (info, got) = decode(&part);
    assert_eq!(info.bits_per_sample, 24);
    let (want, _) = expected(&stems, 5_000, 24);
    assert!(got == want, "samples differ");
}

#[test]
fn clipping_matches_mix_and_is_full_scale() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 16, 9_000, 0.9, 11);
    let b = Stem::new(tmp.path(), "b.flac", 2, 16, 9_000, 0.9, 12);
    let g = mixer::gain_factor(Some(12.0), true);
    let stems = [(&a, g), (&b, g)];
    let part = tmp.path().join("out.part");
    let stats = render(&input(&stems, 9_000, 16), &part).unwrap();

    // the same blocks through mixer::mix
    let mut total = 0u64;
    for start in (0..9_000).step_by(4096) {
        let n = 4096.min(9_000 - start);
        let blocks: Vec<Vec<f32>> = [&a, &b]
            .iter()
            .map(|s| (start..start + n).flat_map(|i| s.at(i)).collect())
            .collect();
        let inputs: Vec<&[f32]> = blocks.iter().map(|b| b.as_slice()).collect();
        let mut out = vec![0.0; n * 2];
        total += u64::from(mixer::mix(&inputs, &[g, g], &mut out));
    }
    assert!(total > 0);
    assert_eq!(stats.clipped_samples, total);
    let (_, got) = decode(&part);
    let full = got.iter().filter(|&&x| x == 32767 || x == -32768).count() as u64;
    assert!(
        full >= total,
        "{full} full-scale samples for {total} clipped"
    );
}

#[test]
fn failures_remove_the_part() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 16, 3_000, 0.5, 1);
    let part = tmp.path().join("x.part");
    let mut bad = input(&[(&a, 1.0)], 3_000, 16);
    bad.stems.push(RenderStem {
        path: tmp.path().join("missing.flac"),
        gain: 1.0,
    });
    assert!(render(&bad, &part).is_err());
    assert!(!part.exists());
    let ok = input(&[(&a, 1.0)], 3_000, 16);
    render(&ok, &part).unwrap();
    assert!(part.exists());
}

#[test]
fn cancel_mid_render_leaves_no_part() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 16, 40_000, 0.5, 1);
    let part = tmp.path().join("x.part");
    let cancel = AtomicBool::new(false);
    let mut calls = 0;
    let r = render_flac(
        &input(&[(&a, 1.0)], 40_000, 16),
        &part,
        &cancel,
        &mut |_| {
            calls += 1;
            cancel.store(true, Ordering::Relaxed);
        },
    );
    assert_eq!(r, Err(CANCELLED.to_string()));
    assert_eq!(calls, 1);
    assert!(!part.exists());
}

#[test]
fn progress_is_monotonic_and_ends_at_one() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Stem::new(tmp.path(), "a.flac", 2, 16, 20_000, 0.5, 1);
    let mut seen = Vec::new();
    render_flac(
        &input(&[(&a, 1.0)], 20_000, 16),
        &tmp.path().join("p"),
        &AtomicBool::new(false),
        &mut |p| seen.push(p),
    )
    .unwrap();
    assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(seen.last(), Some(&1.0));
    assert!(seen[..seen.len() - 1].iter().all(|&p| p < 1.0));
}

#[test]
fn throttle_allows_ten_per_second() {
    let t0 = Instant::now();
    let mut t = Throttle::new(PROGRESS_INTERVAL);
    let allowed: Vec<u64> = (0..=1000u64)
        .filter(|ms| t.allow(t0 + Duration::from_millis(*ms)))
        .collect();
    assert_eq!(allowed.len(), 11); // 0, 100, ..., 1000
    assert!(allowed.windows(2).all(|w| w[1] - w[0] >= 100));
}
