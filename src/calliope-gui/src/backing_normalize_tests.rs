//! Normalize job tests: temporary repositories with a generated backing FLAC, claxon to decode
//! the result, a counting gate instead of a Player, no audio device.

use super::*;
use crate::backing_render::NoPlayer;
use crate::editor::tests::{copy_tree, spec_of};
use crate::loudness::Meter;
use crate::mixer;
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

const ID: &str = "0199c0a0-0000-7000-8000-000000000202";
const FILE: &str = "backings/backing.flac";
const RATE: usize = 22050;

struct Rig {
    _tmp: tempfile::TempDir,
    repo: Repository,
    root: PathBuf,
    dir: PathBuf,
}

impl Rig {
    /// The fixture track with a variant `backing`, whose file is replaced by `samples`.
    fn new(samples: &[i32], channels: usize, bits: usize) -> Rig {
        let (_t, spec) = spec_of(ID);
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let repo = Repository::new(root.clone());
        repo.ensure_layout().unwrap();
        let dir = root.join("tracks").join(ID);
        copy_tree(&spec.dir, &dir);
        let rig = Rig {
            _tmp: tmp,
            repo,
            root,
            dir,
        };
        rig.write_backing(FILE, samples, channels, bits);
        rig
    }

    fn write_backing(&self, name: &str, samples: &[i32], channels: usize, bits: usize) {
        let config = flacenc::config::Encoder::default().into_verified().unwrap();
        let src = flacenc::source::MemSource::from_samples(samples, channels, bits, RATE);
        let stream =
            flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream.write(&mut sink).unwrap();
        fs::write(self.dir.join(name), sink.as_slice()).unwrap();
    }

    fn file(&self) -> PathBuf {
        self.dir.join(FILE)
    }

    fn track_json(&self) -> Vec<u8> {
        fs::read(self.dir.join("track.json")).unwrap()
    }

    fn edit_track(&self, f: impl FnOnce(&mut Value)) {
        let mut v: Value = serde_json::from_slice(&self.track_json()).unwrap();
        f(&mut v);
        fs::write(
            self.dir.join("track.json"),
            serde_json::to_vec_pretty(&v).unwrap(),
        )
        .unwrap();
    }

    fn parts(&self) -> Vec<String> {
        fs::read_dir(&self.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".calliope-backing-"))
            .collect()
    }

    /// Every file in the trash.
    fn trashed(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for d in fs::read_dir(self.root.join("trash"))
            .into_iter()
            .flatten()
            .flatten()
        {
            out.extend(fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()));
        }
        out
    }
}

struct CountGate(AtomicUsize);

impl PlayerGate for CountGate {
    fn without_player(
        &self,
        _id: &str,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        f()
    }
}

struct Run {
    events: Vec<EditorEvent>,
    gate_calls: usize,
}

impl Run {
    fn last(&self) -> &EditorEvent {
        self.events.last().unwrap()
    }

    fn saw_progress(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, EditorEvent::Saving { .. }))
    }

    /// The `normalized` event: (loudness, changed, track written).
    fn normalized(&self) -> (loudness::LoudnessResult, bool, bool) {
        match self.last() {
            EditorEvent::Normalized {
                loudness,
                changed,
                track,
                ..
            } => (loudness.clone(), *changed, track.is_some()),
            e => panic!("{e:?}"),
        }
    }

    fn failed(&self) -> String {
        match self.last() {
            EditorEvent::SaveFailed { message, job, .. } => {
                assert_eq!(*job, JobKind::Normalize);
                message.clone()
            }
            e => panic!("{e:?}"),
        }
    }
}

fn run_with(rig: &Rig, file: &str, pace: Option<Duration>, cancel_at: Option<f32>) -> Run {
    let events = Arc::new(Mutex::new(Vec::new()));
    let shared = SaveShared::new();
    let (ev, sh) = (events.clone(), shared.clone());
    let sink: EventSink = Arc::new(move |e| {
        if let (EditorEvent::Saving { progress, .. }, Some(at)) = (&e, cancel_at) {
            if *progress >= at {
                sh.cancel.store(true, Ordering::Relaxed);
            }
        }
        ev.lock().unwrap().push(e);
    });
    let gate = Arc::new(CountGate(AtomicUsize::new(0)));
    let params = NormalizeParams {
        id: ID.into(),
        dir: rig.dir.clone(),
        variant: "backing".into(),
        file: file.into(),
        repo: rig.repo.clone(),
        lock: Arc::new(|f: &mut dyn FnMut()| f()),
        gate: gate.clone(),
        pace,
    };
    spawn(params, shared.clone(), sink).join().unwrap();
    assert!(shared.is_finished());
    let events = events.lock().unwrap().clone();
    Run {
        events,
        gate_calls: gate.0.load(Ordering::SeqCst),
    }
}

fn run(rig: &Rig) -> Run {
    run_with(rig, FILE, None, None)
}

fn decode(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

fn measure(path: &Path) -> loudness::Measure {
    let (info, got) = decode(path);
    let scale = (1u64 << (info.bits_per_sample - 1)) as f32;
    let mut meter = Meter::new(info.sample_rate).unwrap();
    let floats: Vec<f32> = got.iter().map(|&x| x as f32 / scale).collect();
    meter.add(&floats).unwrap();
    meter.finish()
}

/// A stereo 1 kHz sine of `peak_db` dBFS, `secs` long, `bits` deep.
fn tone(peak_db: f64, secs: usize, bits: u32) -> Vec<i32> {
    let amp = 10f64.powf(peak_db / 20.0) * (1u64 << (bits - 1)) as f64;
    (0..secs * RATE)
        .flat_map(|i| {
            let v = (amp * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / RATE as f64).sin())
                .round() as i32;
            [v, v]
        })
        .collect()
}

#[test]
fn a_quiet_tone_is_brought_to_the_target_and_the_old_file_is_kept() {
    let samples = tone(-20.0, 6, 16);
    let rig = Rig::new(&samples, 2, 16);
    let old_bytes = fs::read(rig.file()).unwrap();
    let (old_info, old) = decode(&rig.file());
    let r = run(&rig);
    let (res, changed, track) = r.normalized();
    assert!(changed && track);
    assert_eq!(res.outcome, loudness::Outcome::Normalized);
    assert!((res.gain_db - 6.0).abs() <= 0.1, "{res:?}");
    assert!((res.lufs.unwrap() + 14.0).abs() <= 0.1);
    assert_eq!(r.gate_calls, 1);
    assert!(r.saw_progress());

    let (info, got) = decode(&rig.file());
    assert_eq!(
        (
            info.samples,
            info.sample_rate,
            info.bits_per_sample,
            info.channels
        ),
        (old_info.samples, 22050, 16, 2)
    );
    let m = measure(&rig.file());
    assert!((m.lufs.unwrap() + 14.0).abs() <= 0.1, "{m:?}");
    let g = loudness::db_to_gain(res.gain_db);
    let want: Vec<i32> = old
        .iter()
        .map(|&x| mixer::quantise(x as f32 / 32768.0 * g, 16))
        .collect();
    assert!(got == want, "samples differ from quantise(old * G)");

    // the old bytes are in the trash; the record is in track.json; nothing else is left
    let trashed = r_trash_bytes(&rig);
    assert_eq!(trashed, vec![old_bytes]);
    assert!(rig.parts().is_empty());
    let v: Value = serde_json::from_slice(&rig.track_json()).unwrap();
    let rec = &v["backings"][0]["loudness"];
    assert_eq!(rec["target"], -14.0);
    assert_eq!(rec["ceiling"], -1.0);
    assert_eq!(rec["md5"], track_meta::md5_hex(info.md5sum));
    assert!((rec["lufs"].as_f64().unwrap() - res.lufs.unwrap()).abs() < 1e-9);

    // a second run is at the target, answered from the record: no gate, no progress, no write
    let (file_before, json_before) = (fs::read(rig.file()).unwrap(), rig.track_json());
    let mtimes = |rig: &Rig| {
        (
            fs::metadata(rig.file()).unwrap().modified().unwrap(),
            fs::metadata(rig.dir.join("track.json"))
                .unwrap()
                .modified()
                .unwrap(),
        )
    };
    let t0 = mtimes(&rig);
    let r2 = run(&rig);
    let (res2, changed2, track2) = r2.normalized();
    assert_eq!(res2.outcome, loudness::Outcome::AtTarget);
    assert!(!changed2 && !track2);
    assert_eq!((r2.gate_calls, r2.saw_progress()), (0, false));
    assert_eq!(fs::read(rig.file()).unwrap(), file_before);
    assert_eq!(rig.track_json(), json_before);
    assert_eq!(mtimes(&rig), t0);

    // without the record the same file is measured: at target, only the record is written
    rig.edit_track(|v| {
        v["backings"][0].as_object_mut().unwrap().remove("loudness");
    });
    let r3 = run(&rig);
    let (res3, changed3, track3) = r3.normalized();
    assert_eq!(res3.outcome, loudness::Outcome::AtTarget);
    assert!(!changed3 && track3 && r3.saw_progress());
    assert_eq!(r3.gate_calls, 0);
    assert_eq!(fs::read(rig.file()).unwrap(), file_before);
    let v: Value = serde_json::from_slice(&rig.track_json()).unwrap();
    assert_eq!(v["backings"][0]["loudness"]["gain_db"], 0.0);
    assert_eq!(rig.trashed().len(), 1);

    // a record whose md5 does not match is ignored (measured again, then corrected)
    rig.edit_track(|v| {
        v["backings"][0]["loudness"]["md5"] = Value::from("0".repeat(31) + "1");
    });
    let r4 = run(&rig);
    let (res4, changed4, track4) = r4.normalized();
    assert_eq!(res4.outcome, loudness::Outcome::AtTarget);
    assert!(!changed4 && track4 && r4.saw_progress());
    let v: Value = serde_json::from_slice(&rig.track_json()).unwrap();
    assert_eq!(
        v["backings"][0]["loudness"]["md5"],
        track_meta::md5_hex(info.md5sum)
    );
}

fn r_trash_bytes(rig: &Rig) -> Vec<Vec<u8>> {
    rig.trashed().iter().map(|p| fs::read(p).unwrap()).collect()
}

#[test]
fn a_24_bit_backing_stays_24_bit() {
    let rig = Rig::new(&tone(-20.0, 3, 24), 2, 24);
    let (_, old) = decode(&rig.file());
    let (res, changed, _) = run(&rig).normalized();
    assert!(changed);
    let (info, got) = decode(&rig.file());
    assert_eq!((info.bits_per_sample, info.channels), (24, 2));
    let g = loudness::db_to_gain(res.gain_db);
    let want: Vec<i32> = old
        .iter()
        .map(|&x| mixer::quantise(x as f32 / 8_388_608.0 * g, 24))
        .collect();
    assert!(got == want);
}

#[test]
fn a_high_crest_signal_is_limited_by_the_peak_ceiling() {
    let mut samples = tone(-20.0, 6, 16);
    let burst = (0.944 * 32768.0) as i32;
    for f in 3 * RATE..3 * RATE + 44 {
        samples[2 * f] = burst;
        samples[2 * f + 1] = burst;
    }
    let rig = Rig::new(&samples, 2, 16);
    let (res, changed, _) = run(&rig).normalized();
    assert!(changed);
    assert_eq!(res.outcome, loudness::Outcome::Limited);
    let m = measure(&rig.file());
    assert!((m.true_peak.unwrap() + 1.0).abs() <= 0.1, "{m:?}");
    assert!(m.lufs.unwrap() < -14.0, "{m:?}");
}

#[test]
fn silence_is_too_quiet_and_the_file_is_left_alone() {
    let rig = Rig::new(&vec![0; 2 * 3 * RATE], 2, 16);
    let before = fs::read(rig.file()).unwrap();
    let r = run(&rig);
    let (res, changed, _) = r.normalized();
    assert_eq!(res.outcome, loudness::Outcome::TooQuiet);
    assert!(!changed);
    assert_eq!(r.gate_calls, 0);
    assert_eq!(fs::read(rig.file()).unwrap(), before);
    assert!(rig.trashed().is_empty() && rig.parts().is_empty());
}

#[test]
fn cancel_in_the_write_pass_changes_nothing() {
    let rig = Rig::new(&tone(-20.0, 6, 16), 2, 16);
    let (file, json) = (fs::read(rig.file()).unwrap(), rig.track_json());
    let r = run_with(&rig, FILE, Some(Duration::from_millis(5)), Some(0.5));
    match r.last() {
        EditorEvent::SaveCancelled { job, .. } => assert_eq!(*job, JobKind::Normalize),
        e => panic!("{e:?}"),
    }
    assert_eq!(r.gate_calls, 0);
    assert_eq!(fs::read(rig.file()).unwrap(), file);
    assert_eq!(rig.track_json(), json);
    assert!(rig.parts().is_empty() && rig.trashed().is_empty());
}

#[test]
fn errors_give_their_message_and_leave_nothing_behind() {
    // a mono file
    let rig = Rig::new(
        &tone(-20.0, 2, 16)
            .iter()
            .step_by(2)
            .copied()
            .collect::<Vec<_>>(),
        1,
        16,
    );
    let r = run(&rig);
    assert_eq!(r.failed(), "Only stereo backings can be normalized");
    let json = rig.track_json();
    assert_eq!(rig.track_json(), json);
    assert!(rig.parts().is_empty() && rig.trashed().is_empty());

    // a missing file, and a symlink
    let rig = Rig::new(&tone(-20.0, 2, 16), 2, 16);
    rig.write_backing("backings/other.flac", &tone(-20.0, 2, 16), 2, 16);
    #[cfg(unix)]
    {
        fs::remove_file(rig.file()).unwrap();
        std::os::unix::fs::symlink(rig.dir.join("backings/other.flac"), rig.file()).unwrap();
        assert_eq!(run(&rig).failed(), MISSING);
    }
    fs::remove_file(rig.file()).unwrap();
    assert_eq!(run(&rig).failed(), MISSING);
    assert_eq!(rig.dir.join("backings").read_dir().unwrap().count(), 1);

    // the variant now names another file
    let r = run_with(&rig, "backings/other.flac", None, None);
    assert_eq!(r.failed(), CHANGED);
    assert!(rig.parts().is_empty() && rig.trashed().is_empty());
}

#[test]
fn a_file_that_changes_during_the_job_is_a_conflict_and_the_part_goes() {
    // the commit sees a different file than the one measured
    let rig = Rig::new(&tone(-20.0, 3, 16), 2, 16);
    let events = Arc::new(Mutex::new(Vec::new()));
    let shared = SaveShared::new();
    let (ev, dir) = (events.clone(), rig.dir.clone());
    let swapped = Arc::new(AtomicBool::new(false));
    let sink: EventSink = Arc::new(move |e| {
        if matches!(e, EditorEvent::Saving { progress, .. } if progress >= 0.5)
            && !swapped.swap(true, Ordering::SeqCst)
        {
            fs::write(dir.join(FILE), b"not a flac").unwrap();
        }
        ev.lock().unwrap().push(e);
    });
    let params = NormalizeParams {
        id: ID.into(),
        dir: rig.dir.clone(),
        variant: "backing".into(),
        file: FILE.into(),
        repo: rig.repo.clone(),
        lock: Arc::new(|f: &mut dyn FnMut()| f()),
        gate: Arc::new(NoPlayer),
        pace: None,
    };
    spawn(params, shared, sink).join().unwrap();
    let last = events.lock().unwrap().last().cloned().unwrap();
    let EditorEvent::SaveFailed { message, .. } = last else {
        panic!("{last:?}")
    };
    assert!(message.starts_with("conflict:"), "{message}");
    assert_eq!(fs::read(rig.file()).unwrap(), b"not a flac");
    assert!(rig.parts().is_empty() && rig.trashed().is_empty());
}
