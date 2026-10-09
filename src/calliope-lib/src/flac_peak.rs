//! The sample peak of a FLAC stem, measured on the integer samples. The server reports it and the
//! app checks what it downloads with the same code, so the two cannot drift apart.
//!
//! The library knows no threshold: callers pass the dBFS level they decide with.

use claxon::FlacReader;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Result of [`scan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    /// Bits per sample of the stem.
    pub bits: u32,
    /// Largest |sample| seen over every channel. With an early stop it is only a lower bound.
    pub peak: u64,
    /// False when the scan stopped early at a sample at or above the limit.
    pub complete: bool,
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Max |sample| over every channel (any channel count, 4..=32 bits; |i32::MIN| = 2^31 fits).
/// With `stop_at_dbfs = Some(t)` it returns at the first |s| >= `limit(bits, t)` (complete = false).
/// Errors name the file: "Cannot decode stem <file>: ...", "Stem <file> has an unsupported format".
pub fn scan(path: &Path, stop_at_dbfs: Option<f64>) -> Result<Scan, String> {
    let label = file_label(path);
    let file = File::open(path).map_err(|e| format!("Cannot open stem {label}: {e}"))?;
    let mut reader = FlacReader::new(BufReader::new(file))
        .map_err(|e| format!("Cannot decode stem {label}: {e}"))?;
    let bits = reader.streaminfo().bits_per_sample;
    let channels = reader.streaminfo().channels;
    if !(4..=32).contains(&bits) || channels == 0 {
        return Err(format!("Stem {label} has an unsupported format"));
    }
    let stop = stop_at_dbfs.map(|t| limit(bits, t));
    let mut peak = 0u64;
    let mut blocks = reader.blocks();
    let mut buf = Vec::new();
    loop {
        let block = blocks
            .read_next_or_eof(std::mem::take(&mut buf))
            .map_err(|e| format!("Cannot decode stem {label}: {e}"))?;
        let Some(block) = block else { break };
        for ch in 0..channels {
            for &v in block.channel(ch) {
                let a = i64::from(v).unsigned_abs();
                peak = peak.max(a);
                if stop.is_some_and(|l| a >= l) {
                    return Ok(Scan { bits, peak, complete: false });
                }
            }
        }
        buf = block.into_buffer();
    }
    Ok(Scan { bits, peak, complete: true })
}

/// Smallest |s| that is NOT below `dbfs`: ceil(2^(bits-1) * 10^(dbfs/20)) (16-bit, -50 -> 104).
pub fn limit(bits: u32, dbfs: f64) -> u64 {
    let full_scale = (1u64 << (bits - 1)) as f64;
    (full_scale * 10f64.powf(dbfs / 20.0)).ceil() as u64
}

/// peak < limit(bits, dbfs): the one comparison every keep/drop decision uses.
pub fn is_below(peak: u64, bits: u32, dbfs: f64) -> bool {
    peak < limit(bits, dbfs)
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

    fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize) {
        let config = flacenc::config::Encoder::default().into_verified().unwrap();
        let src = flacenc::source::MemSource::from_samples(samples, channels, bits, 8000);
        let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream.write(&mut sink).unwrap();
        std::fs::write(path, sink.as_slice()).unwrap();
    }

    fn scan_of(samples: &[i32], channels: usize, bits: usize, stop: Option<f64>) -> Scan {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.flac");
        write_flac(&p, samples, channels, bits);
        scan(&p, stop).unwrap()
    }

    fn spike(v: i32, at: usize) -> Vec<i32> {
        let mut s = vec![0; 40];
        s[at] = v;
        s
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../calliope-gui/tests/fixtures/import")
            .join(name)
    }

    #[test]
    fn all_zero_stereo_is_peak_zero_and_complete() {
        let s = scan_of(&vec![0; 80], 2, 16, None);
        assert_eq!(s, Scan { bits: 16, peak: 0, complete: true });
        assert_eq!(scan_of(&vec![0; 80], 2, 16, Some(-50.0)), s);
    }

    #[test]
    fn limits() {
        assert_eq!(limit(16, -50.0), 104);
        assert_eq!(limit(24, -50.0), 26528);
    }

    #[test]
    fn boundary_16_bit() {
        for (v, below) in [(103, true), (-103, true), (104, false), (-104, false)] {
            let s = scan_of(&spike(v, 5), 1, 16, None);
            assert_eq!(s.peak, v.unsigned_abs() as u64);
            assert_eq!(is_below(s.peak, s.bits, -50.0), below, "{v}");
        }
        assert!(is_below(26527, 24, -50.0));
        assert!(!is_below(26528, 24, -50.0));
    }

    #[test]
    fn early_stop_and_exact_max() {
        let mut s = vec![0; 20_000];
        s[100] = 500;
        s[15_000] = 9000;
        let stopped = scan_of(&s, 1, 16, Some(-50.0));
        assert!(!stopped.complete);
        assert_eq!(stopped.peak, 500);
        let full = scan_of(&s, 1, 16, None);
        assert_eq!(full, Scan { bits: 16, peak: 9000, complete: true });
        let quiet = scan_of(&spike(103, 3), 1, 16, Some(-50.0));
        assert_eq!(quiet, Scan { bits: 16, peak: 103, complete: true });
    }

    #[test]
    fn extreme_widths() {
        // claxon cannot decode 32-bit frames (no frame-header code for them), so the 32-bit
        // arithmetic is checked on the numbers; real files cover 8 and 24 bits.
        assert_eq!(scan_of(&spike(-(1 << 23), 7), 1, 24, None).peak, 1 << 23);
        assert_eq!(limit(32, -50.0), 6_790_940);
        assert_eq!(to_dbfs(1 << 31, 32), Some(0.0));
        assert!(!is_below(1 << 31, 32, -50.0));
        assert!(is_below(0, 32, -50.0));
        let s = scan_of(&spike(-100, 7), 1, 8, None);
        assert_eq!((s.bits, s.peak), (8, 100));
    }

    #[test]
    fn loud_sample_only_in_right_channel_of_last_block() {
        let mut s = vec![0; 2 * 10_000];
        *s.last_mut().unwrap() = 5000;
        assert_eq!(scan_of(&s, 2, 16, None).peak, 5000);
        assert!(!scan_of(&s, 2, 16, Some(-50.0)).complete);
    }

    #[test]
    fn non_flac_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.flac");
        std::fs::write(&bad, b"this is not a flac file at all").unwrap();
        let e = scan(&bad, None).unwrap_err();
        assert!(e.contains("bad.flac") && e.contains("Cannot decode stem"), "{e}");
        assert!(scan(&dir.path().join("gone.flac"), None).unwrap_err().contains("gone.flac"));
    }

    #[test]
    fn committed_fixtures() {
        assert_eq!(scan(&fixture("stems-quiet/silent.flac"), None).unwrap().peak, 0);
        let m60 = scan(&fixture("stems-quiet/minus60.flac"), None).unwrap();
        assert!((30..=34).contains(&m60.peak), "{}", m60.peak);
        let m45 = scan(&fixture("stems-quiet/minus45.flac"), None).unwrap();
        assert!((175..=185).contains(&m45.peak), "{}", m45.peak);
    }

    #[test]
    fn to_dbfs_values() {
        assert_eq!(to_dbfs(0, 16), None);
        let db = to_dbfs(103, 16).unwrap();
        assert!((db + 50.05).abs() < 0.01, "{db}");
        assert!((to_dbfs(1 << 15, 16).unwrap()).abs() < 1e-9);
    }
}
