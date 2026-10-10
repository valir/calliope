//! The level / audible-time scan of a FLAC stem, measured on the integer samples. The server
//! reports it and the app checks what it downloads with the same code, so the two agree bit for
//! bit.
//!
//! A stem is cut into back-to-back, non-overlapping windows of `sample_rate / 10` frames (each
//! counts as [`WINDOW_MS`] ms). A window is audible when its RMS level, taken on the loudest
//! channel, is above a dBFS level. All of it is integer arithmetic: for each channel the window's
//! sum of squares `S_c` is a `u128`, and the window is audible when `max_c S_c > sum_limit(..)`.
//! A partial window at the end is ignored.
//!
//! The library knows no keep/drop decision (the 15 s minimum lives in the app).

use claxon::FlacReader;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Length one window counts as.
pub const WINDOW_MS: u32 = 100;
/// The level a window must exceed (RMS, loudest channel) to count as audible. Shared by the
/// server's report and the app's own check; the app ignores reports made at another level.
pub const AUDIBLE_LEVEL_DBFS: i32 = -40;

/// Result of [`scan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activity {
    pub bits: u32,
    pub sample_rate: u32,
    pub channels: u32,
    /// Max |sample| over every channel; a lower bound when `!complete`.
    pub peak: u64,
    /// Complete windows scanned.
    pub windows: u64,
    /// Windows above the level.
    pub audible_windows: u64,
    /// False when the scan stopped early at `stop_at_ms`.
    pub complete: bool,
}

impl Activity {
    /// `audible_windows * WINDOW_MS`.
    pub fn audible_ms(&self) -> u64 {
        self.audible_windows * u64::from(WINDOW_MS)
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Frames in one window: `sample_rate / 10` (integer division).
pub fn window_frames(sample_rate: u32) -> u64 {
    u64::from(sample_rate / 10)
}

/// The largest window sum of squares that is NOT audible:
/// `floor(frames * 4^(bits-1) * 10^(level_dbfs/10))`. A window is audible when its sum is greater.
/// Exact (`u128`) when `level_dbfs` is a multiple of 10 and not positive (the shipped -40 is);
/// any other level uses an f64 computation, not guaranteed bit-identical across machines.
pub fn sum_limit(bits: u32, frames: u64, level_dbfs: i32) -> u128 {
    let base = u128::from(frames) * (1u128 << (2 * (bits - 1)));
    if level_dbfs <= 0 && level_dbfs % 10 == 0 {
        base / 10u128.pow((-level_dbfs / 10) as u32)
    } else {
        (base as f64 * 10f64.powf(f64::from(level_dbfs) / 10.0)).floor() as u128
    }
}

/// Scans `path`. With `stop_at_ms = Some(m)` it returns as soon as `audible_ms() >= m`
/// (`complete = false`). Errors name the file: "Cannot open stem <f>: ..",
/// "Cannot decode stem <f>: ..", "Stem <f> has an unsupported format" (bits outside 4..=32,
/// 0 channels, sample rate < 10).
pub fn scan(path: &Path, level_dbfs: i32, stop_at_ms: Option<u64>) -> Result<Activity, String> {
    let label = file_label(path);
    let file = File::open(path).map_err(|e| format!("Cannot open stem {label}: {e}"))?;
    let mut reader = FlacReader::new(BufReader::new(file))
        .map_err(|e| format!("Cannot decode stem {label}: {e}"))?;
    let info = reader.streaminfo();
    let (bits, sample_rate, channels) = (info.bits_per_sample, info.sample_rate, info.channels);
    if !(4..=32).contains(&bits) || channels == 0 || sample_rate < 10 {
        return Err(format!("Stem {label} has an unsupported format"));
    }
    let n = window_frames(sample_rate);
    let limit = sum_limit(bits, n, level_dbfs);
    let mut act = Activity {
        bits,
        sample_rate,
        channels,
        peak: 0,
        windows: 0,
        audible_windows: 0,
        complete: true,
    };
    let mut sums = vec![0u128; channels as usize];
    let mut filled = 0u64; // frames in the current window
    let mut blocks = reader.blocks();
    let mut buf = Vec::new();
    loop {
        let block = blocks
            .read_next_or_eof(std::mem::take(&mut buf))
            .map_err(|e| format!("Cannot decode stem {label}: {e}"))?;
        let Some(block) = block else { break };
        let dur = block.duration() as usize;
        let mut pos = 0usize;
        while pos < dur {
            let take = ((n - filled) as usize).min(dur - pos);
            for (ch, sum) in sums.iter_mut().enumerate() {
                for &v in &block.channel(ch as u32)[pos..pos + take] {
                    let a = i64::from(v).unsigned_abs();
                    act.peak = act.peak.max(a);
                    *sum += u128::from(a * a);
                }
            }
            pos += take;
            filled += take as u64;
            if filled == n {
                act.windows += 1;
                if sums.iter().copied().max().unwrap_or(0) > limit {
                    act.audible_windows += 1;
                }
                sums.iter_mut().for_each(|s| *s = 0);
                filled = 0;
                if stop_at_ms.is_some_and(|m| act.audible_ms() >= m) {
                    act.complete = false;
                    return Ok(act);
                }
            }
        }
        buf = block.into_buffer();
    }
    Ok(act)
}

/// 20*log10(peak / 2^(bits-1)); None for peak 0 (digital silence, -inf). Display only.
pub fn to_dbfs(peak: u64, bits: u32) -> Option<f64> {
    (peak > 0).then(|| 20.0 * (peak as f64 / (1u64 << (bits - 1)) as f64).log10())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    const L: i32 = AUDIBLE_LEVEL_DBFS;

    fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize, rate: usize) {
        let config = flacenc::config::Encoder::default().into_verified().unwrap();
        let src = flacenc::source::MemSource::from_samples(samples, channels, bits, rate);
        let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream.write(&mut sink).unwrap();
        std::fs::write(path, sink.as_slice()).unwrap();
    }

    fn act(samples: &[i32], channels: usize, bits: usize, rate: usize, stop: Option<u64>) -> Activity {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.flac");
        write_flac(&p, samples, channels, bits, rate);
        scan(&p, L, stop).unwrap()
    }

    /// Mono 16-bit 8 kHz; `parts` are (windows, |s|) runs of 800-frame windows.
    fn runs(parts: &[(usize, i32)]) -> Vec<i32> {
        parts.iter().flat_map(|&(w, v)| std::iter::repeat_n(v, w * 800)).collect()
    }

    fn mono(parts: &[(usize, i32)]) -> Activity {
        act(&runs(parts), 1, 16, 8000, None)
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../calliope-gui/tests/fixtures/import")
            .join(name)
    }

    #[test]
    fn window_sizes() {
        assert_eq!(window_frames(44100), 4410);
        assert_eq!(window_frames(48000), 4800);
        assert_eq!(window_frames(8000), 800);
    }

    #[test]
    fn limits() {
        assert_eq!(sum_limit(16, 800, -40), 85_899_345);
        assert_eq!(sum_limit(16, 4410, -40), 473_520_144);
        assert_eq!(sum_limit(24, 4410, -40), 4410 * (1u128 << 46) / 10_000);
        assert_eq!(sum_limit(16, 800, 0), 800 << 30);
        // f64 fallback agrees with the exact path where both apply
        assert!(sum_limit(16, 800, -35) > sum_limit(16, 800, -40));
    }

    #[test]
    fn constant_level_boundary() {
        let a = mono(&[(150, 328)]);
        assert_eq!((a.audible_ms(), a.windows, a.complete), (15_000, 150, true));
        assert_eq!(mono(&[(150, 327)]).audible_ms(), 0);
        assert_eq!(mono(&[(150, -328)]).audible_ms(), 15_000);
        assert_eq!(mono(&[(150, -327)]).audible_ms(), 0);
    }

    #[test]
    fn channels_are_not_summed() {
        let stereo = |l: i32, r: i32| -> Vec<i32> {
            (0..10 * 800).flat_map(|_| [l, r]).collect()
        };
        let a = act(&stereo(327, 328), 2, 16, 8000, None);
        assert_eq!((a.windows, a.audible_windows, a.channels), (10, 10, 2));
        assert_eq!(act(&stereo(328, 327), 2, 16, 8000, None).audible_windows, 10);
        assert_eq!(act(&stereo(327, 327), 2, 16, 8000, None).audible_windows, 0);
    }

    #[test]
    fn boundary_149_vs_150_windows() {
        assert_eq!(mono(&[(149, 1000), (10, 0), (1, 0)]).audible_ms(), 14_900);
        assert_eq!(mono(&[(75, 1000), (20, 0), (75, 1000)]).audible_ms(), 15_000);
        assert_eq!(mono(&[(70, 1000), (20, 0), (79, 1000), (5, 0)]).audible_ms(), 14_900);
    }

    #[test]
    fn trailing_partial_window_is_ignored() {
        let mut s = runs(&[(150, 1000)]);
        s.extend(std::iter::repeat_n(1000, 799));
        let a = act(&s, 1, 16, 8000, None);
        assert_eq!((a.audible_ms(), a.windows), (15_000, 150));
        let a = act(&vec![1000; 799], 1, 16, 8000, None);
        assert_eq!((a.audible_ms(), a.windows), (0, 0));
    }

    #[test]
    fn windows_span_flac_blocks() {
        // block size 4096: the window 4000..4800 straddles the first block boundary
        let mut s = vec![0; 8000];
        s[4000..4800].fill(1000);
        let a = act(&s, 1, 16, 8000, None);
        assert_eq!((a.windows, a.audible_windows), (10, 1));
    }

    #[test]
    fn early_stop() {
        let s = runs(&[(600, 1000)]);
        let a = act(&s, 1, 16, 8000, Some(15_000));
        assert_eq!((a.audible_ms(), a.complete, a.windows), (15_000, false, 150));
        let a = act(&s, 1, 16, 8000, None);
        assert_eq!((a.audible_ms(), a.complete), (60_000, true));
        // never reached: complete
        let a = act(&runs(&[(10, 1000)]), 1, 16, 8000, Some(15_000));
        assert_eq!((a.audible_ms(), a.complete), (1000, true));
    }

    #[test]
    fn peak_is_exact_without_early_stop() {
        let mut s = vec![0; 8000];
        s[5000] = -9000;
        let a = act(&s, 1, 16, 8000, None);
        assert_eq!((a.peak, a.audible_ms()), (9000, 0));
    }

    #[test]
    fn stereo_24_bit_44k() {
        let stereo = |v: i32| -> Vec<i32> { (0..4410 * 4).flat_map(|_| [0, v]).collect() };
        // v^2 > 2^46 / 10^4  <=>  v > 2^23 / 100 = 83886.08
        let a = act(&stereo(83_887), 2, 24, 44_100, None);
        assert_eq!((a.bits, a.sample_rate, a.windows, a.audible_windows), (24, 44_100, 4, 4));
        assert_eq!(act(&stereo(83_886), 2, 24, 44_100, None).audible_windows, 0);
        assert_eq!(act(&stereo(-83_887), 2, 24, 44_100, None).audible_windows, 4);
    }

    #[test]
    fn errors_name_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.flac");
        std::fs::write(&bad, b"this is not a flac file at all").unwrap();
        let e = scan(&bad, L, None).unwrap_err();
        assert!(e.contains("bad.flac") && e.contains("Cannot decode stem"), "{e}");
        let e = scan(&dir.path().join("gone.flac"), L, None).unwrap_err();
        assert!(e.contains("gone.flac") && e.contains("Cannot open stem"), "{e}");
    }

    #[test]
    fn unsupported_sample_rate() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("slow.flac");
        write_flac(&p, &[0; 40], 1, 16, 8);
        let e = scan(&p, L, None).unwrap_err();
        assert!(e.contains("slow.flac") && e.contains("unsupported format"), "{e}");
    }

    #[test]
    fn committed_fixtures() {
        let ms = |f: &str| scan(&fixture(f), L, None).unwrap();
        let b = ms("stems-activity/bursts.flac");
        assert_eq!(b.audible_ms(), 9_500);
        assert!(b.peak >= 13_000, "{}", b.peak);
        assert_eq!(ms("stems-activity/phrases.flac").audible_ms(), 16_000);
        assert_eq!(ms("stems-activity/audible-14900ms.flac").audible_ms(), 14_900);
        assert_eq!(ms("stems-activity/audible-15000ms.flac").audible_ms(), 15_000);
        let mut n = 0;
        for e in std::fs::read_dir(fixture("stems")).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "flac") {
                assert_eq!(scan(&p, L, None).unwrap().audible_ms(), 16_000, "{p:?}");
                n += 1;
            }
        }
        assert_eq!(n, 6);
        for f in ["minus45", "minus60", "silent"] {
            assert_eq!(ms(&format!("stems-quiet/{f}.flac")).audible_ms(), 0, "{f}");
        }
    }

    #[test]
    fn to_dbfs_values() {
        assert_eq!(to_dbfs(0, 16), None);
        assert!((to_dbfs(1 << 15, 16).unwrap()).abs() < 1e-9);
        assert_eq!(to_dbfs(1 << 31, 32), Some(0.0));
    }
}
