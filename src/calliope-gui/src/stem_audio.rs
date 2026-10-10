//! FLAC stem decoding for the backing track editor: probing, compatibility rules, 16-bit
//! in-memory decoding for playback, and a full-precision streaming reader for the render.

#![allow(dead_code)] // used by the editor session and the render job (tasks 7 and 9) and the tests

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use claxon::FlacReader;
use calliope_lib::flac_level;
use calliope_lib::stems_api::StemLevel;

/// Cap on decoded audio per track: sum(frames x channels x 2 bytes).
pub const MAX_DECODED_BYTES: u64 = 3 * 512 * 1024 * 1024; // 1.5 GiB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StemInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub bits: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Common {
    pub sample_rate: u32,
    pub bits_out: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StemPcm {
    pub channels: u32,
    pub frames: u64,
    /// Interleaved, `frames * channels` samples.
    pub samples: Vec<i16>,
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn open(path: &Path) -> Result<FlacReader<BufReader<File>>, String> {
    let label = file_label(path);
    let file = File::open(path).map_err(|e| format!("Cannot open stem {label}: {e}"))?;
    FlacReader::new(BufReader::new(file))
        .map_err(|e| format!("Stem {label} is not a readable FLAC file: {e}"))
}

/// Reads the STREAMINFO block only.
pub fn probe(path: &Path) -> Result<StemInfo, String> {
    let reader = open(path)?;
    let si = reader.streaminfo();
    let frames = si
        .samples
        .ok_or_else(|| format!("Stem {} does not record its length", file_label(path)))?;
    Ok(StemInfo {
        sample_rate: si.sample_rate,
        channels: si.channels,
        bits: si.bits_per_sample,
        frames,
    })
}

/// Applies the stem rules (2.2) and the memory cap. Every error names the stem.
pub fn check_compatible(stems: &[(String, StemInfo)]) -> Result<Common, String> {
    let mut rate: Option<u32> = None;
    let mut bits_out = 16;
    let mut frames = 0u64;
    let mut bytes = 0u64;
    for (name, info) in stems {
        if !(1..=2).contains(&info.channels) {
            return Err(format!(
                "Stem {name} has {} channels; only mono and stereo are supported",
                info.channels
            ));
        }
        if !(8..=24).contains(&info.bits) {
            return Err(format!(
                "Stem {name} has {} bits per sample; only 8 to 24 are supported",
                info.bits
            ));
        }
        match rate {
            None => rate = Some(info.sample_rate),
            Some(r) if r != info.sample_rate => {
                return Err(format!(
                    "The stems have different sample rates (stem {name} has {} Hz, expected {r} Hz)",
                    info.sample_rate
                ));
            }
            _ => {}
        }
        if info.bits > 16 {
            bits_out = 24;
        }
        frames = frames.max(info.frames);
        bytes = bytes.saturating_add(info.frames.saturating_mul(u64::from(info.channels) * 2));
        if bytes > MAX_DECODED_BYTES {
            return Err(format!(
                "The stems are too long to load (stem {name} exceeds the 1.5 GiB limit of decoded audio)"
            ));
        }
    }
    let sample_rate = rate.ok_or_else(|| "The track has no stems".to_string())?;
    Ok(Common { sample_rate, bits_out, frames })
}

/// Converts a sample of `bits` bits to 16 bits, rounding half up when reducing.
fn to_i16(v: i32, bits: u32) -> i16 {
    let out = if bits > 16 {
        let shift = bits - 16;
        (v + (1 << (shift - 1))) >> shift
    } else {
        v << (16 - bits)
    };
    out.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Decodes a whole stem to interleaved i16. `progress(decoded_frames, total_frames)` is
/// called after each block; `cancel` is checked before each block.
pub fn decode_i16(
    path: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<StemPcm, String> {
    let label = file_label(path);
    let mut reader = open(path)?;
    let si = reader.streaminfo();
    let total = si
        .samples
        .ok_or_else(|| format!("Stem {label} does not record its length"))?;
    let channels = si.channels;
    let bits = si.bits_per_sample;
    if !(1..=2).contains(&channels) || !(8..=24).contains(&bits) {
        return Err(format!("Stem {label} has an unsupported format"));
    }
    let mut samples: Vec<i16> = Vec::with_capacity((total * u64::from(channels)) as usize);
    let mut blocks = reader.blocks();
    let mut buf = Vec::new();
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(format!("Loading stem {label} was cancelled"));
        }
        let block = blocks
            .read_next_or_eof(std::mem::take(&mut buf))
            .map_err(|e| format!("Cannot decode stem {label}: {e}"))?;
        let Some(block) = block else { break };
        let n = block.duration() as usize;
        for i in 0..n {
            for ch in 0..channels {
                samples.push(to_i16(block.sample(ch, i as u32), bits));
            }
        }
        done += n as u64;
        buf = block.into_buffer();
        progress(done, total);
    }
    Ok(StemPcm { channels, frames: done, samples })
}

/// A stem audible (100 ms windows above `flac_level::AUDIBLE_LEVEL_DBFS`) for less than this is
/// empty and is dropped at import. The only place the number appears.
pub const EMPTY_STEM_MIN_AUDIBLE_MS: u64 = 15_000;

/// The verdict on one stem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StemCheck {
    /// audible_ms >= the minimum; `audible_ms` is a lower bound when the scan stopped early.
    Audible { audible_ms: u64 },
    /// audible_ms < the minimum (exact). `peak_dbfs` is for logs only (None = digital silence;
    /// always None for a server report).
    Empty { audible_ms: u64, peak_dbfs: Option<f64> },
}

/// True when `audible_ms` is below the minimum.
pub fn is_empty(audible_ms: u64) -> bool {
    audible_ms < EMPTY_STEM_MIN_AUDIBLE_MS
}

/// Measures a stem with the scan shared with the server, stopping as soon as it has proven the
/// stem audible for the minimum.
pub fn check_stem(path: &Path) -> Result<StemCheck, String> {
    let a = flac_level::scan(path, flac_level::AUDIBLE_LEVEL_DBFS, Some(EMPTY_STEM_MIN_AUDIBLE_MS))?;
    let audible_ms = a.audible_ms();
    if is_empty(audible_ms) {
        Ok(StemCheck::Empty { audible_ms, peak_dbfs: flac_level::to_dbfs(a.peak, a.bits) })
    } else {
        Ok(StemCheck::Audible { audible_ms })
    }
}

/// The decision for a server report; `None` when it was measured at another level or window
/// (unusable).
pub fn check_reported(level: &StemLevel) -> Option<StemCheck> {
    if level.level_dbfs != flac_level::AUDIBLE_LEVEL_DBFS || level.window_ms != flac_level::WINDOW_MS {
        return None;
    }
    Some(if is_empty(level.audible_ms) {
        StemCheck::Empty { audible_ms: level.audible_ms, peak_dbfs: None }
    } else {
        StemCheck::Audible { audible_ms: level.audible_ms }
    })
}

/// Streaming full-precision reader for the render: f32 stereo, mono duplicated, silence
/// after the end.
pub struct StemReader {
    label: String,
    reader: FlacReader<BufReader<File>>,
    channels: u32,
    sample_rate: u32,
    bits: u32,
    scale: f32,
    buf: Vec<i32>,
    /// Current block as interleaved source samples, and the read position in frames.
    block: Vec<i32>,
    pos: usize,
    finished: bool,
}

impl StemReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let reader = open(path)?;
        let si = reader.streaminfo();
        if !(1..=2).contains(&si.channels) || !(8..=24).contains(&si.bits_per_sample) {
            return Err(format!("Stem {} has an unsupported format", file_label(path)));
        }
        Ok(Self {
            label: file_label(path),
            channels: si.channels,
            sample_rate: si.sample_rate,
            bits: si.bits_per_sample,
            scale: 1.0 / (1u64 << (si.bits_per_sample - 1)) as f32,
            reader,
            buf: Vec::new(),
            block: Vec::new(),
            pos: 0,
            finished: false,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn bits(&self) -> u32 {
        self.bits
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Fills `out` (interleaved stereo, `frames` frames) and returns the number of frames
    /// that came from the file; the rest is zero.
    pub fn read(&mut self, out: &mut [f32], frames: usize) -> Result<usize, String> {
        let frames = frames.min(out.len() / 2);
        let ch = self.channels as usize;
        let mut written = 0;
        while written < frames {
            let avail = self.block.len() / ch - self.pos;
            if avail == 0 {
                if self.finished || !self.next_block()? {
                    self.finished = true;
                    break;
                }
                continue;
            }
            let n = avail.min(frames - written);
            for i in 0..n {
                let base = (self.pos + i) * ch;
                let l = self.block[base] as f32 * self.scale;
                let r = if ch == 2 { self.block[base + 1] as f32 * self.scale } else { l };
                out[(written + i) * 2] = l;
                out[(written + i) * 2 + 1] = r;
            }
            self.pos += n;
            written += n;
        }
        out[written * 2..frames * 2].fill(0.0);
        Ok(written)
    }

    fn next_block(&mut self) -> Result<bool, String> {
        let mut blocks = self.reader.blocks();
        let block = blocks
            .read_next_or_eof(std::mem::take(&mut self.buf))
            .map_err(|e| format!("Cannot decode stem {}: {e}", self.label))?;
        let Some(block) = block else { return Ok(false) };
        let n = block.duration() as usize;
        self.block.clear();
        for i in 0..n {
            for c in 0..self.channels {
                self.block.push(block.sample(c, i as u32));
            }
        }
        self.pos = 0;
        self.buf = block.into_buffer();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    /// Writes a FLAC file from interleaved samples.
    fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize, rate: usize) {
        let config = flacenc::config::Encoder::default().into_verified().unwrap();
        let src = flacenc::source::MemSource::from_samples(samples, channels, bits, rate);
        let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream.write(&mut sink).unwrap();
        std::fs::write(path, sink.as_slice()).unwrap();
    }

    fn no_progress() -> impl FnMut(u64, u64) {
        |_, _| {}
    }

    fn info(rate: u32, channels: u32, bits: u32, frames: u64) -> StemInfo {
        StemInfo { sample_rate: rate, channels, bits, frames }
    }

    fn named(v: Vec<(&str, StemInfo)>) -> Vec<(String, StemInfo)> {
        v.into_iter().map(|(n, i)| (n.to_string(), i)).collect()
    }

    /// 10000 frames, longer than one block, so multi-block paths run.
    fn ramp(n: usize, scale: i32) -> Vec<i32> {
        (0..n as i32).map(|i| ((i * 7) % 2000 - 1000) * scale).collect()
    }

    #[test]
    fn probe_and_decode_16_bit_mono_exact() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.flac");
        let s = ramp(10000, 20);
        write_flac(&p, &s, 1, 16, 22050);
        assert_eq!(probe(&p).unwrap(), info(22050, 1, 16, 10000));
        let mut calls = 0;
        let pcm = decode_i16(&p, &AtomicBool::new(false), &mut |d, t| {
            calls += 1;
            assert!(d <= t);
        })
        .unwrap();
        assert!(calls >= 2);
        assert_eq!((pcm.channels, pcm.frames), (1, 10000));
        let want: Vec<i16> = s.iter().map(|&v| v as i16).collect();
        assert_eq!(pcm.samples, want);
    }

    #[test]
    fn decode_16_bit_stereo_exact() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.flac");
        let s = ramp(2 * 6000, 30);
        write_flac(&p, &s, 2, 16, 44100);
        let pcm = decode_i16(&p, &AtomicBool::new(false), &mut no_progress()).unwrap();
        assert_eq!((pcm.channels, pcm.frames), (2, 6000));
        let want: Vec<i16> = s.iter().map(|&v| v as i16).collect();
        assert_eq!(pcm.samples, want);
    }

    #[test]
    fn decode_24_bit_rounds_to_16() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.flac");
        // 24 -> 16 drops 8 bits: 127 -> 0, 128 -> 1 (half up), -128 -> 0, -129 -> -1.
        let s = vec![0, 127, 128, 255, 256, -127, -128, -129, 0x7FFFFF, -0x800000];
        let mut padded = s.clone();
        padded.resize(32, 0); // FLAC blocks need at least 16 frames
        write_flac(&p, &padded, 1, 24, 44100);
        let pcm = decode_i16(&p, &AtomicBool::new(false), &mut no_progress()).unwrap();
        assert_eq!(pcm.samples[..10], [0, 0, 1, 1, 1, 0, 0, -1, 32767, -32768]);
    }

    #[test]
    fn decode_8_bit_scales_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.flac");
        let mut s = vec![1, -1, 127, -128];
        s.resize(32, 0);
        write_flac(&p, &s, 1, 8, 8000);
        let pcm = decode_i16(&p, &AtomicBool::new(false), &mut no_progress()).unwrap();
        assert_eq!(pcm.samples[..4], [256, -256, 32512, -32768]);
    }

    #[test]
    fn check_compatible_accepts_and_summarises() {
        let c = check_compatible(&named(vec![
            ("a", info(22050, 1, 16, 100)),
            ("b", info(22050, 2, 16, 250)),
        ]))
        .unwrap();
        assert_eq!(c, Common { sample_rate: 22050, bits_out: 16, frames: 250 });
        let c = check_compatible(&named(vec![
            ("a", info(48000, 1, 16, 100)),
            ("b", info(48000, 2, 24, 50)),
        ]))
        .unwrap();
        assert_eq!(c.bits_out, 24);
        assert_eq!(c.frames, 100);
    }

    #[test]
    fn check_compatible_rejects_with_stem_named() {
        let e = check_compatible(&named(vec![
            ("vocals", info(22050, 1, 16, 10)),
            ("drums", info(44100, 1, 16, 10)),
        ]))
        .unwrap_err();
        assert!(e.contains("different sample rates") && e.contains("drums"), "{e}");
        let e = check_compatible(&named(vec![("bass", info(22050, 3, 16, 10))])).unwrap_err();
        assert!(e.contains("bass") && e.contains("3 channels"), "{e}");
        let e = check_compatible(&named(vec![("piano", info(22050, 1, 4, 10))])).unwrap_err();
        assert!(e.contains("piano"), "{e}");
        // 1.5 GiB cap: exactly at the cap passes, one more byte-pair fails.
        let big = MAX_DECODED_BYTES / 4; // frames of a stereo stem that fill the cap exactly
        assert!(check_compatible(&named(vec![("a", info(22050, 2, 16, big))])).is_ok());
        let e = check_compatible(&named(vec![
            ("a", info(22050, 2, 16, big)),
            ("other", info(22050, 1, 16, 1)),
        ]))
        .unwrap_err();
        assert!(e.contains("other") && e.contains("1.5 GiB"), "{e}");
    }

    #[test]
    fn stem_reader_exact_then_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("r.flac");
        let s = ramp(9000, 3);
        write_flac(&p, &s, 1, 16, 22050);
        let mut r = StemReader::open(&p).unwrap();
        let mut out = vec![9.0f32; 2 * 9500];
        // Read in odd-sized chunks across block boundaries.
        let mut got = 0;
        let mut total = 0;
        while total < 9500 {
            let n = 1234.min(9500 - total);
            got += r.read(&mut out[total * 2..(total + n) * 2], n).unwrap();
            total += n;
        }
        assert_eq!(got, 9000);
        for (i, &v) in s.iter().enumerate() {
            let want = v as f32 / 32768.0;
            assert_eq!(out[i * 2], want, "frame {i}");
            assert_eq!(out[i * 2 + 1], want);
        }
        assert!(out[9000 * 2..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn stem_reader_stereo_24_bit_full_precision() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("r24.flac");
        let mut s = vec![1, -1, 0x7FFFFF, -0x800000, 12345, 54321];
        s.resize(2 * 20, 0); // 20 frames
        write_flac(&p, &s, 2, 24, 44100);
        let mut r = StemReader::open(&p).unwrap();
        let mut out = vec![9.0f32; 2 * 22];
        assert_eq!(r.read(&mut out, 22).unwrap(), 20);
        let sc = 1.0 / 8388608.0f32;
        let want: Vec<f32> = s.iter().map(|&v| v as f32 * sc).chain([0.0; 4]).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn cancel_flag_stops_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.flac");
        write_flac(&p, &ramp(10000, 1), 1, 16, 22050);
        let e = decode_i16(&p, &AtomicBool::new(true), &mut no_progress()).unwrap_err();
        assert!(e.contains("c.flac") && e.contains("cancelled"), "{e}");
    }

    #[test]
    fn missing_and_non_flac_files_name_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.flac");
        assert!(probe(&missing).unwrap_err().contains("gone.flac"));
        assert!(decode_i16(&missing, &AtomicBool::new(false), &mut no_progress())
            .unwrap_err()
            .contains("gone.flac"));
        let bad = dir.path().join("bad.flac");
        std::fs::write(&bad, b"this is not a flac file at all").unwrap();
        assert!(probe(&bad).unwrap_err().contains("bad.flac"));
        assert!(StemReader::open(&bad).err().unwrap().contains("bad.flac"));
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import").join(name)
    }

    fn lvl(audible_ms: u64, level_dbfs: i32, window_ms: u32) -> StemLevel {
        StemLevel { name: "x".into(), audible_ms, level_dbfs, window_ms, peak_dbfs: None }
    }

    #[test]
    fn check_stem_activity_fixtures() {
        match check_stem(&fixture("stems-activity/bursts.flac")).unwrap() {
            StemCheck::Empty { audible_ms: 9500, peak_dbfs: Some(p) } => assert!(p > -9.0, "{p}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            check_stem(&fixture("stems-activity/phrases.flac")).unwrap(),
            StemCheck::Audible { .. }
        ));
        assert!(matches!(
            check_stem(&fixture("stems-activity/audible-14900ms.flac")).unwrap(),
            StemCheck::Empty { audible_ms: 14_900, .. }
        ));
        assert!(matches!(
            check_stem(&fixture("stems-activity/audible-15000ms.flac")).unwrap(),
            StemCheck::Audible { audible_ms: 15_000 }
        ));
        assert_eq!(
            check_stem(&fixture("stems-quiet/silent.flac")).unwrap(),
            StemCheck::Empty { audible_ms: 0, peak_dbfs: None }
        );
    }

    #[test]
    fn check_stem_committed_stems_are_kept_and_quiet_ones_empty() {
        for name in ["bass", "drums", "guitar", "other", "piano", "vocals"] {
            let c = check_stem(&fixture(&format!("stems/{name}.flac"))).unwrap();
            assert!(matches!(c, StemCheck::Audible { .. }), "{name}: {c:?}");
        }
        for name in ["silent", "minus60", "minus45"] {
            let c = check_stem(&fixture(&format!("stems-quiet/{name}.flac"))).unwrap();
            assert!(matches!(c, StemCheck::Empty { audible_ms: 0, .. }), "{name}: {c:?}");
        }
    }

    #[test]
    fn check_reported_decides_and_ignores_other_measures() {
        assert_eq!(
            check_reported(&lvl(14_900, -40, 100)),
            Some(StemCheck::Empty { audible_ms: 14_900, peak_dbfs: None })
        );
        assert_eq!(check_reported(&lvl(15_000, -40, 100)), Some(StemCheck::Audible { audible_ms: 15_000 }));
        assert_eq!(check_reported(&lvl(0, -50, 100)), None);
        assert_eq!(check_reported(&lvl(0, -40, 50)), None);
    }

    #[test]
    fn check_stem_rejects_non_flac() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.flac");
        std::fs::write(&bad, b"this is not a flac file at all").unwrap();
        assert!(check_stem(&bad).unwrap_err().contains("bad.flac"));
        assert!(check_stem(&dir.path().join("gone.flac")).is_err());
    }
}
