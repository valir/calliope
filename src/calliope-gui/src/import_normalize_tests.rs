//! The normalizing step on synthetic 44.1 kHz stereo FLACs in a temporary folder; claxon decodes
//! the results. No audio device, no server.

use super::*;
use crate::loudness::Meter;
use crate::mixer;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use serde_json::Value;
use std::cell::Cell;
use std::os::unix::fs::MetadataExt;

const RATE: usize = 44100;

fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize) {
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, channels, bits, RATE);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

fn decode(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

fn measure_samples(samples: &[i32], bits: u32) -> Measure {
    let scale = (1u64 << (bits - 1)) as f32;
    let mut meter = Meter::new(RATE as u32).unwrap();
    let floats: Vec<f32> = samples.iter().map(|&x| x as f32 / scale).collect();
    meter.add(&floats).unwrap();
    meter.finish()
}

/// A stereo 1 kHz sine of `peak_db` dBFS.
fn tone(peak_db: f64, secs: f64, bits: u32) -> Vec<i32> {
    let amp = 10f64.powf(peak_db / 20.0) * (1u64 << (bits - 1)) as f64;
    let max = (1i64 << (bits - 1)) as f64;
    (0..(secs * RATE as f64) as usize)
        .flat_map(|i| {
            let v = (amp * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / RATE as f64).sin())
                .round()
                .clamp(-max, max - 1.0) as i32;
            [v, v]
        })
        .collect()
}

/// A tone scaled (with the meter) to `lufs` integrated.
fn at_lufs(lufs: f64, secs: f64, bits: u32) -> Vec<i32> {
    let m = measure_samples(&tone(-24.0, secs, bits), bits);
    tone(-24.0 + lufs - m.lufs.unwrap(), secs, bits)
}

/// Six 25 ms 1 kHz bursts at -3 dBFS with 10 ms raised-cosine fades, one per second.
fn bursts() -> Vec<i32> {
    let mut out = vec![0i32; 6 * RATE * 2];
    let (len, fade) = (RATE / 40, RATE / 100);
    let amp = 10f64.powf(-3.0 / 20.0) * 32768.0;
    for b in 0..6 {
        for i in 0..len {
            let edge = i.min(len - 1 - i);
            let w = if edge < fade {
                0.5 - 0.5 * (std::f64::consts::PI * edge as f64 / fade as f64).cos()
            } else {
                1.0
            };
            let v = (amp * w * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / RATE as f64).sin())
                .round() as i32;
            let at = (b * RATE + RATE / 2 + i) * 2;
            out[at] = v;
            out[at + 1] = v;
        }
    }
    out
}

struct Rig {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    audio: PathBuf,
}

impl Rig {
    fn new(samples: &[i32], channels: usize, bits: usize) -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("file-x");
        fs::create_dir(&dir).unwrap();
        let audio = dir.join("audio.flac");
        write_flac(&audio, samples, channels, bits);
        Rig {
            _tmp: tmp,
            dir,
            audio,
        }
    }

    fn run(&self) -> Result<Normalized, String> {
        normalize(&self.dir, &self.audio, &AtomicBool::new(false), &mut |_| {})
    }

    fn marker(&self) -> Value {
        serde_json::from_slice(&fs::read(self.dir.join(MARKER_FILE)).unwrap()).unwrap()
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn assert_untouched_leftovers_free(&self) {
        assert!(!self.dir.join(PART_FILE).exists());
        assert!(!self.dir.join(NORMALIZED_FLAC).exists());
        assert!(!self.dir.join(MARKER_FILE).exists());
    }
}

#[test]
fn a_quiet_tone_is_scaled_to_the_target() {
    let samples = at_lufs(-24.0, 6.0, 16);
    let rig = Rig::new(&samples, 2, 16);
    let before = fs::read(&rig.audio).unwrap();
    let (_, src) = decode(&rig.audio);
    let calls = Cell::new(0);
    let last = Cell::new(0.0f32);
    let n = normalize(&rig.dir, &rig.audio, &AtomicBool::new(false), &mut |f| {
        calls.set(calls.get() + 1);
        assert!(f >= last.get() - 1e-6 && (0.0..=1.0).contains(&f));
        last.set(f);
    })
    .unwrap();
    assert!(calls.get() > 0 && last.get() == 1.0);
    assert_eq!(n.decision.outcome, Outcome::Normalized);
    assert!((n.decision.gain_db - 10.0).abs() <= 0.01, "{n:?}");
    assert!(n.changed && !n.resumed);
    assert_eq!(n.audio, rig.dir.join(NORMALIZED_FLAC));
    assert!((n.measured.lufs.unwrap() + 24.0).abs() < 0.01);
    assert!((n.result.lufs.unwrap() + 14.0).abs() < 0.02);

    let (info, got) = decode(&n.audio);
    assert_eq!((info.bits_per_sample, info.channels), (24, 2));
    let g = loudness::db_to_gain(n.decision.gain_db);
    let want: Vec<i32> = src
        .iter()
        .map(|&x| mixer::quantise(x as f32 / 32768.0 * g, 24))
        .collect();
    assert!(got == want, "samples differ from quantise(x * G, 24)");
    let m = measure_samples(&got, 24);
    assert!((m.lufs.unwrap() + 14.0).abs() <= 0.05, "{m:?}");

    let v = rig.marker();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["file"], NORMALIZED_FLAC);
    assert_eq!(v["outcome"], "normalized");
    assert_eq!(v["target"], -14.0);
    assert_eq!(v["ceiling"], -1.0);
    assert_eq!(v["md5"], md5_hex(info.md5sum));
    assert_eq!(
        v["source_md5"],
        md5_hex(StemReader::open(&rig.audio).unwrap().md5())
    );
    assert!((v["gain_db"].as_f64().unwrap() - 10.0).abs() <= 0.01);
    assert!((v["measured_lufs"].as_f64().unwrap() + 24.0).abs() < 0.01);
    assert!(v["measured_tp"].as_f64().is_some());
    assert_eq!(
        rig.names(),
        ["audio.flac", "normalize.json", "normalized.flac"]
    );
    assert_eq!(fs::read(&rig.audio).unwrap(), before);
}

#[test]
fn bursts_are_limited_by_the_peak_ceiling() {
    let rig = Rig::new(&bursts(), 2, 16);
    let n = rig.run().unwrap();
    assert_eq!(n.decision.outcome, Outcome::Limited, "{n:?}");
    assert!(n.decision.gain_db > 0.0);
    let (_, got) = decode(&n.audio);
    let m = measure_samples(&got, 24);
    assert!(m.true_peak.unwrap() <= -1.0 + 0.01, "{m:?}");
    assert!(m.lufs.unwrap() < -14.0, "{m:?}");
    assert_eq!(rig.marker()["outcome"], "limited");
}

#[test]
fn a_loud_signal_gets_a_negative_gain_and_never_reaches_full_scale() {
    let mut samples = at_lufs(-8.0, 6.0, 16);
    for i in [1000usize, 90_000, 200_000] {
        samples[i * 2] = 32767;
        samples[i * 2 + 1] = -32768;
    }
    let rig = Rig::new(&samples, 2, 16);
    let n = rig.run().unwrap();
    assert!(n.changed && n.decision.gain_db < 0.0, "{n:?}");
    let (_, got) = decode(&n.audio);
    assert!(got.iter().all(|&x| x.abs() < (1 << 23) - 1));
    let m = measure_samples(&got, 24);
    assert!(m.true_peak.unwrap() <= -1.0 + 0.01, "{m:?}");
}

#[test]
fn a_file_at_the_target_is_left_alone() {
    let rig = Rig::new(&at_lufs(-14.0, 6.0, 16), 2, 16);
    let n = rig.run().unwrap();
    assert_eq!(n.decision.outcome, Outcome::AtTarget);
    assert_eq!(n.decision.gain_db, 0.0);
    assert!(!n.changed && !n.resumed);
    assert_eq!(n.audio, rig.audio);
    assert!(!rig.dir.join(NORMALIZED_FLAC).exists());
    let v = rig.marker();
    assert_eq!(v["file"], "audio.flac");
    assert_eq!(v["md5"], v["source_md5"]);
    assert_eq!(v["outcome"], "at-target");
}

#[test]
fn silence_and_a_short_tone_are_too_quiet() {
    for samples in [vec![0i32; 10 * RATE * 2], tone(-24.0, 0.3, 16)] {
        let rig = Rig::new(&samples, 2, 16);
        let before = fs::read(&rig.audio).unwrap();
        let n = rig.run().unwrap();
        assert_eq!(n.decision.outcome, Outcome::TooQuiet, "{n:?}");
        assert_eq!(n.decision.gain_db, 0.0);
        assert!(!n.changed);
        assert_eq!(n.audio, rig.audio);
        assert_eq!(fs::read(&rig.audio).unwrap(), before);
        assert_eq!(rig.names(), ["audio.flac", "normalize.json"]);
    }
}

#[test]
fn a_second_run_resumes_without_touching_anything() {
    let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
    let first = rig.run().unwrap();
    let out = rig.dir.join(NORMALIZED_FLAC);
    let (bytes, meta) = (fs::read(&out).unwrap(), fs::metadata(&out).unwrap());
    let marker_meta = fs::metadata(rig.dir.join(MARKER_FILE)).unwrap();
    let again = normalize(&rig.dir, &rig.audio, &AtomicBool::new(false), &mut |_| {
        panic!("progress called on resume")
    })
    .unwrap();
    assert!(again.resumed);
    assert_eq!(
        (&again.audio, again.measured, again.decision, &again.result),
        (&first.audio, first.measured, first.decision, &first.result)
    );
    let meta2 = fs::metadata(&out).unwrap();
    assert_eq!(fs::read(&out).unwrap(), bytes);
    assert_eq!(
        (meta.ino(), meta.modified().unwrap()),
        (meta2.ino(), meta2.modified().unwrap())
    );
    assert_eq!(
        marker_meta.modified().unwrap(),
        fs::metadata(rig.dir.join(MARKER_FILE))
            .unwrap()
            .modified()
            .unwrap()
    );
    // an unchanged outcome resumes too
    let quiet = Rig::new(&[0i32; 2 * RATE * 2], 2, 16);
    quiet.run().unwrap();
    assert!(resume(&quiet.dir, &quiet.audio).unwrap().resumed);
}

#[test]
fn invalid_markers_are_not_resumed_and_the_rewrite_is_identical() {
    type Break = Box<dyn Fn(&Path)>;
    let edit = |f: fn(&mut Value)| -> Break {
        Box::new(move |dir: &Path| {
            let p = dir.join(MARKER_FILE);
            let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
            f(&mut v);
            fs::write(p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        })
    };
    let breaks: Vec<(&str, Break)> = vec![
        (
            "garbage",
            Box::new(|d| fs::write(d.join(MARKER_FILE), b"not json").unwrap()),
        ),
        ("schema 2", edit(|v| v["schema"] = 2.into())),
        ("target", edit(|v| v["target"] = (-16.0).into())),
        ("ceiling", edit(|v| v["ceiling"] = (-2.0).into())),
        (
            "source md5",
            edit(|v| v["source_md5"] = "0123456789abcdef0123456789abcdef".into()),
        ),
        (
            "file md5",
            edit(|v| v["md5"] = "0123456789abcdef0123456789abcdef".into()),
        ),
        (
            "mismatch",
            edit(|v| {
                v["outcome"] = "at-target".into();
            }),
        ),
        ("unknown outcome", edit(|v| v["outcome"] = "fine".into())),
        (
            "deleted",
            Box::new(|d| fs::remove_file(d.join(NORMALIZED_FLAC)).unwrap()),
        ),
        (
            "no marker",
            Box::new(|d| fs::remove_file(d.join(MARKER_FILE)).unwrap()),
        ),
    ];
    for (name, brk) in breaks {
        let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
        rig.run().unwrap();
        let bytes = fs::read(rig.dir.join(NORMALIZED_FLAC)).unwrap();
        let marker = fs::read(rig.dir.join(MARKER_FILE)).unwrap();
        brk(&rig.dir);
        assert!(resume(&rig.dir, &rig.audio).is_none(), "{name}");
        let n = rig.run().unwrap();
        assert!(!n.resumed, "{name}");
        assert_eq!(
            fs::read(rig.dir.join(NORMALIZED_FLAC)).unwrap(),
            bytes,
            "{name}"
        );
        assert_eq!(
            fs::read(rig.dir.join(MARKER_FILE)).unwrap(),
            marker,
            "{name}"
        );
    }
    // a normalized.flac without a marker (crash before the marker) is replaced by the same bytes
    let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
    rig.run().unwrap();
    let bytes = fs::read(rig.dir.join(NORMALIZED_FLAC)).unwrap();
    fs::remove_file(rig.dir.join(MARKER_FILE)).unwrap();
    fs::write(rig.dir.join(NORMALIZED_FLAC), b"junk").unwrap();
    assert!(!rig.run().unwrap().resumed);
    assert_eq!(fs::read(rig.dir.join(NORMALIZED_FLAC)).unwrap(), bytes);
}

#[test]
fn a_marker_for_another_audio_file_is_not_resumed() {
    let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
    rig.run().unwrap();
    write_flac(&rig.audio, &at_lufs(-30.0, 4.0, 16), 2, 16);
    assert!(resume(&rig.dir, &rig.audio).is_none());
}

#[test]
fn cancel_leaves_nothing_behind() {
    let rig = Rig::new(&at_lufs(-24.0, 6.0, 16), 2, 16);
    let before = fs::read(&rig.audio).unwrap();
    // before the call
    let cancel = AtomicBool::new(true);
    assert_eq!(
        normalize(&rig.dir, &rig.audio, &cancel, &mut |_| {}).unwrap_err(),
        CANCELLED
    );
    rig.assert_untouched_leftovers_free();
    // while measuring (progress below 0.4) and while writing (above 0.4)
    for at in [0.1f32, 0.7] {
        let cancel = AtomicBool::new(false);
        let err = normalize(&rig.dir, &rig.audio, &cancel, &mut |f| {
            if f >= at {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert_eq!(err, CANCELLED, "at {at}");
        rig.assert_untouched_leftovers_free();
        assert_eq!(rig.names(), ["audio.flac"], "at {at}");
        assert_eq!(fs::read(&rig.audio).unwrap(), before);
    }
}

#[test]
fn leftover_part_files_and_unsupported_inputs() {
    let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
    fs::write(rig.dir.join(PART_FILE), b"half written").unwrap();
    let n = rig.run().unwrap();
    assert!(n.changed);
    assert!(!rig.dir.join(PART_FILE).exists());

    let rig = Rig::new(&at_lufs(-24.0, 4.0, 16), 2, 16);
    let before = fs::read(&rig.audio).unwrap();
    fs::create_dir(rig.dir.join(PART_FILE)).unwrap();
    let err = rig.run().unwrap_err();
    assert!(err.contains("cannot clear .normalized.flac.part"), "{err}");
    assert_eq!(fs::read(&rig.audio).unwrap(), before);
    assert!(!rig.dir.join(NORMALIZED_FLAC).exists());

    let mono = Rig::new(&vec![1000i32; RATE], 1, 16);
    assert!(mono.run().unwrap_err().contains("unsupported format"));
    let eight = Rig::new(&vec![10i32; RATE * 2], 2, 8);
    assert!(eight.run().unwrap_err().contains("unsupported format"));
}
