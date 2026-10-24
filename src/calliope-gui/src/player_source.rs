//! Sources of a track for the Player, decoding to 16-bit PCM in memory, and waveform peaks.
//! No audio device here. FLAC goes through `stem_audio`; anything else through an ffmpeg pipe.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use calliope_lib::process;

use crate::repository::TrackRecord;
use crate::stem_audio;

/// Rate and channels every non-FLAC source is converted to.
pub const FFMPEG_RATE: u32 = 44_100;
pub const FFMPEG_CHANNELS: u32 = 2;
pub const PEAK_BUCKETS: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub key: String,
    pub label: String,
    /// Path relative to the track folder.
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u32,
    pub frames: u64,
    /// Interleaved, `frames * channels` samples.
    pub samples: Vec<i16>,
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The dropdown entries of a track, in order: backings, plain audio, original, stems. Files
/// listed as missing are left out.
pub fn sources(track: &TrackRecord) -> Vec<Source> {
    let present = |f: &str| !track.missing.iter().any(|m| m == f);
    let mut out = Vec::new();
    for b in track.backings.iter().filter(|b| present(&b.file)) {
        out.push(Source {
            key: format!("backing:{}", b.id),
            label: b.name.clone(),
            file: b.file.clone(),
        });
    }
    if let Some(a) = track.audio.as_ref().filter(|a| present(a)) {
        out.push(Source {
            key: "audio".into(),
            label: "Backing track".into(),
            file: a.clone(),
        });
    }
    if let Some(o) = track.original.as_ref().filter(|o| present(o)) {
        out.push(Source {
            key: "original".into(),
            label: "Original mix".into(),
            file: o.clone(),
        });
    }
    for s in track.stems.iter().filter(|s| present(&s.file)) {
        out.push(Source {
            key: format!("stem:{}", s.name),
            label: capitalized(&s.name),
            file: s.file.clone(),
        });
    }
    out
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn is_flac(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("flac"))
}

/// Decodes `file` (absolute) to 16-bit PCM. `progress(done, total)`; `total` is 0 when unknown.
/// Setting `cancel` stops the decode promptly with an error.
pub fn decode(
    file: &Path,
    ffmpeg: Option<&Path>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Pcm, String> {
    if is_flac(file) {
        let info = stem_audio::probe(file)?;
        let bytes = info.frames * u64::from(info.channels) * 2;
        if bytes > stem_audio::MAX_DECODED_BYTES {
            return Err(format!("{} is too long to play", file_label(file)));
        }
        let pcm = stem_audio::decode_i16(file, cancel, progress)?;
        if pcm.frames == 0 || pcm.samples.is_empty() {
            return Err(format!(
                "Cannot decode {}: no audio found",
                file_label(file)
            ));
        }
        return Ok(Pcm {
            rate: info.sample_rate,
            channels: pcm.channels,
            frames: pcm.frames,
            samples: pcm.samples,
        });
    }
    let label = file_label(file);
    let Some(ffmpeg) = ffmpeg else {
        return Err(format!("Install ffmpeg to play {label}."));
    };
    decode_ffmpeg(file, ffmpeg, cancel, progress, &label)
}

fn decode_ffmpeg(
    file: &Path,
    ffmpeg: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
    label: &str,
) -> Result<Pcm, String> {
    let abs = std::path::absolute(file).map_err(|e| format!("Cannot decode {label}: {e}"))?;
    let args: Vec<std::ffi::OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-i",
    ]
    .iter()
    .map(Into::into)
    .chain([
        {
            let mut s = std::ffi::OsString::from("file:");
            s.push(abs.as_os_str());
            s
        },
        "-map".into(),
        "0:a:0".into(),
        "-f".into(),
        "s16le".into(),
        "-acodec".into(),
        "pcm_s16le".into(),
        "-ac".into(),
        FFMPEG_CHANNELS.to_string().into(),
        "-ar".into(),
        FFMPEG_RATE.to_string().into(),
        "pipe:1".into(),
    ])
    .collect();

    let bytes: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let too_big = Arc::new(AtomicBool::new(false));
    let err_last = Arc::new(Mutex::new(String::new()));
    let (b, tb, el) = (bytes.clone(), too_big.clone(), err_last.clone());
    let running = process::spawn_raw_stdout(
        ffmpeg,
        &args,
        None,
        move |chunk| {
            if let Ok(mut v) = b.lock() {
                if (v.len() + chunk.len()) as u64 > stem_audio::MAX_DECODED_BYTES {
                    tb.store(true, Ordering::SeqCst);
                } else {
                    v.extend_from_slice(chunk);
                }
            }
        },
        move |l| {
            if !l.trim().is_empty() {
                if let Ok(mut s) = el.lock() {
                    *s = l;
                }
            }
        },
    )
    .map_err(|e| format!("Cannot decode {label}: {e}"))?;

    let handle = running.cancel_handle();
    let done = Arc::new(AtomicBool::new(false));
    // Watcher: polls the caller's flag, since `cancel` is borrowed and cannot move to a thread.
    let cancelled = std::thread::scope(|scope| {
        let (h, d, tb2) = (handle.clone(), done.clone(), too_big.clone());
        scope.spawn(move || {
            let mut sent = false;
            while !d.load(Ordering::SeqCst) {
                if !sent && (cancel.load(Ordering::SeqCst) || tb2.load(Ordering::SeqCst)) {
                    h.cancel();
                    sent = true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let status = running.wait();
        done.store(true, Ordering::SeqCst);
        status
    });
    let status = cancelled.map_err(|e| format!("Cannot decode {label}: {e}"))?;
    if cancel.load(Ordering::SeqCst) {
        return Err(format!("Loading {label} was cancelled"));
    }
    if too_big.load(Ordering::SeqCst) {
        return Err(format!("{label} is too long to play"));
    }
    let last = err_last.lock().map(|s| s.clone()).unwrap_or_default();
    if !status.success() {
        let why = if last.is_empty() {
            String::new()
        } else {
            format!(": {last}")
        };
        return Err(format!("Cannot decode {label}{why}"));
    }
    let raw = std::mem::take(
        &mut *bytes
            .lock()
            .map_err(|_| "decode buffer poisoned".to_string())?,
    );
    let frame_bytes = 2 * FFMPEG_CHANNELS as usize;
    if raw.len() < frame_bytes {
        return Err(format!("Cannot decode {label}: no audio found"));
    }
    let samples: Vec<i16> = raw
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let frames = (samples.len() / FFMPEG_CHANNELS as usize) as u64;
    progress(frames, frames);
    Ok(Pcm {
        rate: FFMPEG_RATE,
        channels: FFMPEG_CHANNELS,
        frames,
        samples,
    })
}

/// Maximum `|sample|/32768` over all channels in each of `buckets` equal slices of the track
/// (the last slice takes the remainder).
pub fn peaks(pcm: &Pcm, buckets: usize) -> Vec<f32> {
    let ch = pcm.channels.max(1) as usize;
    let frames = pcm.samples.len() / ch;
    let mut out = vec![0f32; buckets];
    if buckets == 0 || frames == 0 {
        return out;
    }
    let per = (frames / buckets).max(1);
    for f in 0..frames {
        let b = (f / per).min(buckets - 1);
        for &s in &pcm.samples[f * ch..(f + 1) * ch] {
            let v = (i32::from(s)).unsigned_abs() as f32 / 32768.0;
            if v > out[b] {
                out[b] = v;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;
    use std::path::PathBuf;
    use std::time::Instant;

    fn fixture_copy() -> (tempfile::TempDir, PathBuf) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-player");
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("lib");
        copy_dir(&from, &to);
        (dir, to)
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dest = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &dest);
            } else {
                std::fs::copy(e.path(), dest).unwrap();
            }
        }
    }

    fn track(root: &Path, title: &str) -> (TrackRecord, PathBuf) {
        let lib = Repository::new(root).scan();
        let t = lib.tracks.into_iter().find(|t| t.title == title).unwrap();
        let dir = root.join("tracks").join(&t.id);
        (t, dir)
    }

    fn keys(t: &TrackRecord) -> Vec<(String, String)> {
        sources(t).into_iter().map(|s| (s.key, s.label)).collect()
    }

    fn pair(k: &str, l: &str) -> (String, String) {
        (k.to_string(), l.to_string())
    }

    #[test]
    fn source_lists() {
        let (_d, root) = fixture_copy();
        let (t, _) = track(&root, "Practice Song");
        assert_eq!(
            keys(&t),
            [
                pair("backing:backing", "Backing"),
                pair("original", "Original mix"),
                pair("stem:vocals", "Vocals"),
                pair("stem:guitar", "Guitar")
            ]
        );
        let (t, _) = track(&root, "Missing Backing");
        assert_eq!(keys(&t), [pair("stem:vocals", "Vocals")]);
        let (t, _) = track(&root, "Missing Audio");
        assert!(sources(&t).is_empty());
    }

    #[test]
    fn plain_audio_source() {
        let (_d, root) = fixture_copy();
        let (t, _) = track(&root, "MP3 Backing");
        assert_eq!(keys(&t), [pair("audio", "Backing track")]);
    }

    fn no_progress() -> impl FnMut(u64, u64) {
        |_, _| {}
    }

    #[test]
    fn mp3_decodes_through_ffmpeg() {
        let (_d, root) = fixture_copy();
        let (t, dir) = track(&root, "MP3 Backing");
        let file = dir.join(&sources(&t)[0].file);
        let pcm = decode(
            &file,
            Some(Path::new("ffmpeg")),
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap();
        assert_eq!((pcm.rate, pcm.channels), (44100, 2));
        let secs = pcm.frames as f64 / 44100.0;
        assert!((secs - 20.0).abs() <= 0.1, "{secs}");
        assert_eq!(pcm.samples.len() as u64, pcm.frames * 2);
    }

    #[test]
    fn missing_ffmpeg_and_bad_files_name_the_file() {
        let (_d, root) = fixture_copy();
        let (t, dir) = track(&root, "MP3 Backing");
        let file = dir.join(&sources(&t)[0].file);
        let e = decode(&file, None, &AtomicBool::new(false), &mut no_progress()).unwrap_err();
        assert_eq!(e, "Install ffmpeg to play backing.mp3.");
        let bad = dir.join("broken.mp3");
        std::fs::write(&bad, b"this is not audio").unwrap();
        let e = decode(
            &bad,
            Some(Path::new("ffmpeg")),
            &AtomicBool::new(false),
            &mut no_progress(),
        )
        .unwrap_err();
        assert!(e.contains("broken.mp3"), "{e}");
    }

    #[test]
    fn flac_equals_stem_audio() {
        let (_d, root) = fixture_copy();
        let (t, dir) = track(&root, "Practice Song");
        let file = dir.join(&sources(&t)[0].file);
        let pcm = decode(&file, None, &AtomicBool::new(false), &mut no_progress()).unwrap();
        let want = stem_audio::decode_i16(&file, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(pcm.samples, want.samples);
        assert_eq!((pcm.channels, pcm.frames), (want.channels, want.frames));
        assert_eq!(pcm.rate, 22050);
    }

    #[test]
    fn backing_peaks() {
        let (_d, root) = fixture_copy();
        let (t, dir) = track(&root, "Practice Song");
        let file = dir.join(&sources(&t)[0].file);
        let pcm = decode(&file, None, &AtomicBool::new(false), &mut no_progress()).unwrap();
        let p = peaks(&pcm, PEAK_BUCKETS);
        assert_eq!(p.len(), 2048);
        let q1 = p[..512].iter().cloned().fold(0.0, f32::max);
        let q2 = p[512..1024].iter().cloned().fold(0.0, f32::max);
        assert!((q1 - 0.2).abs() <= 0.02, "{q1}");
        assert!((q2 - 0.9).abs() <= 0.02, "{q2}");
    }

    #[test]
    fn peaks_remainder_and_channels() {
        let pcm = Pcm {
            rate: 1,
            channels: 2,
            frames: 5,
            samples: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, -16384],
        };
        assert_eq!(peaks(&pcm, 2), [0.0, 0.5]);
    }

    fn alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
            && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map_or(true, |s| s.contains(") Z"))
    }

    #[test]
    fn cancelled_ffmpeg_returns_promptly_and_leaves_no_child() {
        // A fake "ffmpeg" that never ends and records its pid.
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = dir.path().join("ffmpeg");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > {}\nexec sleep 60\n",
                pidfile.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            c2.store(true, Ordering::SeqCst);
        });
        let t0 = Instant::now();
        let e = decode(
            Path::new("/x/song.mp3"),
            Some(&script),
            &cancel,
            &mut no_progress(),
        )
        .unwrap_err();
        canceller.join().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
        assert!(e.contains("cancelled"), "{e}");
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!alive(pid));
    }
}
