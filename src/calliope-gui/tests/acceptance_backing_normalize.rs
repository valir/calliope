//! QA acceptance tests for gui-backing-normalize (Rust side), one test per criterion.
//!
//! Audio only ever goes to `ManualBackend` (never a real device). Every test works on a temp
//! copy of `tests/fixtures/library-editor`; stems are synthesised at test time with flacenc.
//! The texts of the result line are frontend code (`loudness-format.test.ts`).
#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    clippy::type_complexity,
    clippy::all
)]

use calliope_gui::audio_out::ManualBackend;
use calliope_gui::backing_render::{self, PlayerGate, RenderInput, RenderStem};
use calliope_gui::editor::{EditorEvent, EditorManager, EventSink, JobKind, OpenSpec};
use calliope_gui::import_job::RepoLock;
use calliope_gui::loudness::{self, Meter, Outcome};
use calliope_gui::mixer;
use calliope_gui::player::{self, PlayerEvent, PlayerManager};
use calliope_gui::player_source;
use calliope_gui::repository::Repository;
use serde_json::Value;
use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
const MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
const RATE: u32 = 22050;

type Events = Arc<Mutex<Vec<EditorEvent>>>;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn fixture_repo() -> (tempfile::TempDir, Repository) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor");
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    copy_tree(&src, &root);
    let repo = Repository::new(&root);
    repo.ensure_layout().unwrap();
    (tmp, repo)
}

fn dir_of(repo: &Repository, id: &str) -> PathBuf {
    repo.tracks_dir().join(id)
}

fn spec(repo: &Repository, id: &str) -> OpenSpec {
    let rec = repo.load_record(id).unwrap();
    OpenSpec {
        dir: dir_of(repo, id),
        id: rec.id,
        title: rec.title,
        stems: rec.stems,
        missing: rec.missing,
        variant: rec.backings.into_iter().next(),
        tempo: Default::default(),
    }
}

fn manager() -> (EditorManager, ManualBackend, Events) {
    let backend = ManualBackend::new();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
    (
        EditorManager::new(Arc::new(backend.clone()), sink),
        backend,
        events,
    )
}

fn no_lock() -> Arc<dyn RepoLock> {
    Arc::new(|f: &mut dyn FnMut()| f())
}

fn is_final(e: &EditorEvent) -> bool {
    matches!(
        e,
        EditorEvent::Saved { .. }
            | EditorEvent::Normalized { .. }
            | EditorEvent::SaveFailed { .. }
            | EditorEvent::SaveCancelled { .. }
    )
}

/// The one final event of the running job.
fn final_event(events: &Events) -> EditorEvent {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(e) = events.lock().unwrap().iter().find(|e| is_final(e)).cloned() {
            return e;
        }
        assert!(Instant::now() < end, "the job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn clear(events: &Events) {
    events.lock().unwrap().clear();
}

fn json_of(dir: &Path) -> Value {
    serde_json::from_slice(&fs::read(dir.join("track.json")).unwrap()).unwrap()
}

fn stray_parts(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".calliope-backing-") || n.ends_with(".part"))
        .collect()
}

fn trash_files(repo: &Repository) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in fs::read_dir(repo.root.join("trash"))
        .into_iter()
        .flatten()
        .flatten()
    {
        out.extend(fs::read_dir(d.path()).unwrap().flatten().map(|e| e.path()));
    }
    out
}

fn read_flac(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

fn write_flac(path: &Path, samples: &[i32], bits: usize, rate: u32) {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, 2, bits, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

/// The loudness and true peak of a FLAC file, measured on its decoded samples.
fn measure_file(path: &Path) -> loudness::Measure {
    let (info, got) = read_flac(path);
    let scale = (1u64 << (info.bits_per_sample - 1)) as f32;
    let mut meter = Meter::new(info.sample_rate).unwrap();
    let floats: Vec<f32> = got.iter().map(|&x| x as f32 / scale).collect();
    meter.add(&floats).unwrap();
    meter.finish()
}

/// Stereo 1 kHz sine of `peak_db` dBFS, `secs` long, with an optional `(start s, ms, peak dBFS)` burst.
fn tone(rate: u32, secs: f64, peak_db: f64, burst: Option<(f64, f64, f64)>) -> Vec<f32> {
    let frames = (secs * rate as f64) as usize;
    (0..frames)
        .flat_map(|n| {
            let t = n as f64 / rate as f64;
            let db = match burst {
                Some((at, ms, bdb)) if t >= at && t < at + ms / 1000.0 => bdb,
                _ => peak_db,
            };
            let v = (10f64.powf(db / 20.0) * (2.0 * PI * 1000.0 * t).sin()) as f32;
            [v, v]
        })
        .collect()
}

fn quantise(pcm: &[f32], bits: u32) -> Vec<i32> {
    let scale = (1u64 << (bits - 1)) as f32;
    let max = scale as i32 - 1;
    pcm.iter()
        .map(|&v| ((v * scale).round() as i32).clamp(-max - 1, max))
        .collect()
}

fn measure_pcm(rate: u32, pcm: &[f32]) -> loudness::Measure {
    let mut m = Meter::new(rate).unwrap();
    m.add(pcm).unwrap();
    m.finish()
}

/// `pcm` scaled so that it measures exactly `lufs`.
fn at_lufs(rate: u32, pcm: Vec<f32>, lufs: f64) -> Vec<f32> {
    let g = 10f64.powf((lufs - measure_pcm(rate, &pcm).lufs.unwrap()) / 20.0) as f32;
    pcm.into_iter().map(|v| v * g).collect()
}

/// FOUR with its `vocals` stem replaced by `pcm` (16 bit) and the vocals lane checked at `gain_db`.
fn open_four_with(
    pcm: &[f32],
    gain_db: f32,
) -> (
    tempfile::TempDir,
    Repository,
    EditorManager,
    ManualBackend,
    Events,
) {
    let (tmp, repo) = fixture_repo();
    write_flac(
        &dir_of(&repo, FOUR).join("stems/vocals.flac"),
        &quantise(pcm, 16),
        16,
        RATE,
    );
    let (mut m, b, ev) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(gain_db), true, Instant::now())
        .unwrap();
    (tmp, repo, m, b, ev)
}

/// Saves FOUR and returns the file, the `saved` event's parts and the repository.
fn save(
    m: &mut EditorManager,
    repo: &Repository,
    ev: &Events,
) -> (PathBuf, loudness::LoudnessResult, u64, Vec<String>) {
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    match final_event(ev) {
        EditorEvent::Saved {
            file,
            loudness,
            clipped_samples,
            warnings,
            ..
        } => (
            dir_of(repo, FOUR).join(file),
            loudness,
            clipped_samples,
            warnings,
        ),
        e => panic!("{e:?}"),
    }
}

fn normalize(
    m: &mut EditorManager,
    repo: &Repository,
    ev: &Events,
    id: &str,
) -> (loudness::LoudnessResult, bool, Option<Value>) {
    clear(ev);
    m.start_normalize(id, repo.clone(), no_lock()).unwrap();
    match final_event(ev) {
        EditorEvent::Normalized {
            loudness,
            changed,
            track,
            ..
        } => (
            loudness,
            changed,
            track.map(|t| serde_json::to_value(t).unwrap()),
        ),
        e => panic!("{e:?}"),
    }
}

fn near(got: f64, want: f64, tol: f64, what: &str) {
    assert!((got - want).abs() <= tol, "{what}: {got} vs {want}");
}

// ====================================================================== criterion 1

#[test]
fn ac1_ebu_reference_signals() {
    let sines = |segs: &[(f64, f64)]| -> Vec<f32> {
        let mut out = Vec::new();
        let mut n = 0usize;
        for &(db, secs) in segs {
            for _ in 0..(secs * RATE as f64) as usize {
                let v = (10f64.powf(db / 20.0) * (2.0 * PI * 1000.0 * n as f64 / RATE as f64).sin())
                    as f32;
                out.extend([v, v]);
                n += 1;
            }
        }
        out
    };
    // faded tone at fs/4 (cases 16 and 19)
    let quarter = |peak: f64, phase: f64| -> Vec<f32> {
        let frames = RATE as usize * 3;
        let fade = (0.02 * RATE as f64) as usize;
        (0..frames)
            .flat_map(|n| {
                let edge = n.min(frames - 1 - n);
                let w = if edge >= fade {
                    1.0
                } else {
                    0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos()
                };
                let v = (w
                    * peak
                    * (2.0 * PI * (RATE as f64 / 4.0) * n as f64 / RATE as f64
                        + phase.to_radians())
                    .sin()) as f32;
                [v, v]
            })
            .collect()
    };
    // (name, pcm, wanted LUFS, wanted dBTP)
    let cases: Vec<(&str, Vec<f32>, Option<f64>, Option<f64>)> = vec![
        ("1", sines(&[(-23.0, 20.0)]), Some(-23.0), None),
        (
            "3",
            sines(&[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)]),
            Some(-23.0),
            None,
        ),
        (
            "4",
            sines(&[
                (-72.0, 10.0),
                (-36.0, 10.0),
                (-23.0, 60.0),
                (-36.0, 10.0),
                (-72.0, 10.0),
            ]),
            Some(-23.0),
            None,
        ),
        ("16", quarter(0.5, 45.0), None, Some(-6.0)),
        ("19", quarter(1.41, 45.0), None, Some(3.0)),
    ];
    let tmp = tempfile::tempdir().unwrap();
    for bits in [16u32, 24] {
        for (name, pcm, lufs, tp) in &cases {
            let path = tmp.path().join(format!("case{name}-{bits}.flac"));
            write_flac(&path, &quantise(pcm, bits), bits as usize, RATE);
            let input = RenderInput {
                stems: vec![RenderStem {
                    path: path.clone(),
                    gain: 1.0,
                }],
                sample_rate: RATE,
                bits,
                frames: (pcm.len() / 2) as u64,
            };
            let m =
                backing_render::measure_mix(&input, &AtomicBool::new(false), &mut |_| {}).unwrap();
            let what = format!("case {name} at {bits} bits: {m:?}");
            if let Some(want) = lufs {
                near(m.lufs.expect(&what), *want, 0.1, &what);
            }
            if let Some(want) = tp {
                near(m.true_peak.expect(&what), *want, 0.1, &what);
            }
        }
    }
}

// ====================================================================== criterion 2

/// A 6 s stem at -20 LUFS with a short loud burst (peak about -10 dBFS).
fn quiet_stem_with_burst() -> Vec<f32> {
    let base = at_lufs(
        RATE,
        tone(RATE, 12.0, -19.0, Some((3.0, 10.0, -10.0))),
        -20.0,
    );
    let m = measure_pcm(RATE, &base);
    near(m.lufs.unwrap(), -20.0, 0.01, "stem loudness");
    assert!(
        m.true_peak.unwrap() < -8.0 && m.true_peak.unwrap() > -12.0,
        "{m:?}"
    );
    base
}

#[test]
fn ac2_a_minus_20_lufs_mix_is_saved_at_minus_14() {
    let (_t, repo, mut m, _b, ev) = open_four_with(&quiet_stem_with_burst(), 0.0);
    let (file, result, clipped, warnings) = save(&mut m, &repo, &ev);
    assert_eq!(result.outcome, Outcome::Normalized);
    near(result.gain_db, 6.0, 0.1, "gain in the result");
    let got = measure_file(&file);
    near(got.lufs.unwrap(), -14.0, 0.1, "decoded loudness");
    assert_eq!(clipped, 0);
    assert!(warnings.is_empty(), "{warnings:?}");
    let rec = &json_of(&dir_of(&repo, FOUR))["backings"][0]["loudness"];
    near(rec["gain_db"].as_f64().unwrap(), 6.0, 0.1, "recorded gain");
    near(
        rec["lufs"].as_f64().unwrap(),
        -14.0,
        0.1,
        "recorded loudness",
    );
    assert_eq!(rec["target"], -14.0);
    assert_eq!(rec["ceiling"], -1.0);
}

// ====================================================================== criterion 3

#[test]
fn ac3_a_mix_whose_gain_would_pass_the_ceiling_is_limited_by_it() {
    // -20 LUFS with a -3 dBFS burst: +6 dB would put the peak at about +3 dBTP
    let pcm = at_lufs(
        RATE,
        tone(RATE, 12.0, -19.0, Some((3.0, 10.0, -3.0))),
        -20.0,
    );
    let (_t, repo, mut m, _b, ev) = open_four_with(&pcm, 0.0);
    let (file, result, clipped, _w) = save(&mut m, &repo, &ev);
    assert_eq!(result.outcome, Outcome::Limited);
    let got = measure_file(&file);
    near(got.true_peak.unwrap(), -1.0, 0.1, "decoded true peak");
    assert!(got.lufs.unwrap() < -14.0, "{got:?}");
    assert!(got.lufs.unwrap() > -20.0, "{got:?}");
    near(result.true_peak.unwrap(), -1.0, 0.1, "reported true peak");
    assert_eq!(clipped, 0);
}

// ====================================================================== criterion 4

#[test]
fn ac4_a_mix_that_clipped_before_is_saved_without_a_clipped_sample() {
    // a peaky stem (-30 dBFS tone, one -9 dBFS burst) with the lane at +12 dB: the burst
    // reaches +3 dBFS, which clips in playback
    let pcm = tone(RATE, 12.0, -30.0, Some((3.0, 10.0, -9.0)));
    let gain = mixer::gain_factor(Some(12.0), true);
    let mut out = vec![0.0f32; pcm.len()];
    let clipped_in_playback = mixer::mix(&[&pcm[..]], &[gain], &mut out);
    assert!(clipped_in_playback > 0, "the mix must clip in playback");

    let (_t, repo, mut m, _b, ev) = open_four_with(&pcm, 12.0);
    let (file, result, clipped, warnings) = save(&mut m, &repo, &ev);
    assert_eq!(clipped, 0);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(result.outcome, Outcome::Limited);
    let (info, samples) = read_flac(&file);
    let full_scale = 1i32 << (info.bits_per_sample - 1);
    assert!(
        samples.iter().all(|s| s.abs() < full_scale - 1),
        "a sample reaches full scale"
    );
    near(
        measure_file(&file).true_peak.unwrap(),
        -1.0,
        0.1,
        "true peak",
    );
}

// ====================================================================== criteria 5, 6

/// MIXED with a `player.json` next to it (tempo and loops), as the Player leaves them.
fn mixed_with_player_json() -> (tempfile::TempDir, Repository, Vec<u8>) {
    let (t, repo) = fixture_repo();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/library-player/tracks/0199c0a0-0000-7000-8000-000000000304");
    let bytes = fs::read(from.join("player.json")).unwrap();
    fs::write(dir_of(&repo, MIXED).join("player.json"), &bytes).unwrap();
    (t, repo, bytes)
}

fn stem_bytes(dir: &Path) -> Vec<Vec<u8>> {
    ["vocals", "guitar"]
        .iter()
        .map(|s| fs::read(dir.join(format!("stems/{s}.flac"))).unwrap())
        .collect()
}

#[test]
fn ac5_normalize_brings_an_existing_backing_to_minus_14_and_touches_nothing_else() {
    let (_t, repo, player_json) = mixed_with_player_json();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let (old_info, old_samples) = read_flac(&file);
    let stems_before = stem_bytes(&dir);
    let before = json_of(&dir);
    let old_measure = measure_file(&file);
    assert!(old_measure.lufs.unwrap() < -15.0, "{old_measure:?}");

    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (result, changed, track) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    assert_eq!(result.outcome, Outcome::Normalized);

    let (info, samples) = read_flac(&file);
    assert_eq!(info.samples, old_info.samples, "same length");
    assert_eq!(samples.len(), old_samples.len());
    assert_eq!(info.sample_rate, old_info.sample_rate);
    assert_eq!(info.bits_per_sample, old_info.bits_per_sample);
    near(measure_file(&file).lufs.unwrap(), -14.0, 0.1, "loudness");
    assert_eq!(stem_bytes(&dir), stems_before, "stems unchanged");
    assert_eq!(fs::read(dir.join("player.json")).unwrap(), player_json);

    let after = json_of(&dir);
    let rec = &after["backings"][0]["loudness"];
    near(rec["lufs"].as_f64().unwrap(), -14.0, 0.1, "recorded lufs");
    assert!(rec["true_peak"].is_number());
    near(
        rec["gain_db"].as_f64().unwrap(),
        result.gain_db,
        1e-9,
        "recorded gain",
    );
    assert_eq!(after["backings"][0]["mix"], before["backings"][0]["mix"]);
    assert_eq!(after["stems"], before["stems"]);
    assert_eq!(after["backings"][0]["file"], before["backings"][0]["file"]);
    assert!(track.is_some(), "the event carries the new record");
    assert!(stray_parts(&dir).is_empty());
}

#[test]
fn ac6_an_already_normalized_backing_is_not_rewritten() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (first, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    let bytes = fs::read(&file).unwrap();
    let mtime = fs::metadata(&file).unwrap().modified().unwrap();
    let json = fs::read(dir.join("track.json")).unwrap();
    let trash = trash_files(&repo).len();

    let (second, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(!changed);
    assert_eq!(second.outcome, Outcome::AtTarget);
    assert_eq!(second.gain_db, 0.0);
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), mtime);
    assert_eq!(fs::read(dir.join("track.json")).unwrap(), json);
    assert_eq!(trash_files(&repo).len(), trash, "nothing new in the trash");
}

// ====================================================================== criterion 7

#[test]
fn ac7_cancel_during_normalize_leaves_the_file_byte_identical() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let bytes = fs::read(&file).unwrap();
    let json = fs::read(dir.join("track.json")).unwrap();
    let player_json_existed = dir.join("player.json").exists();
    let (mut m, _b, ev) = manager();
    m.set_job_pace(Some(Duration::from_millis(20)));
    m.open(&spec(&repo, MIXED)).unwrap();
    m.start_normalize(MIXED, repo.clone(), no_lock()).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let half = ev.lock().unwrap().iter().any(
            |e| matches!(e, EditorEvent::Saving { progress, job, .. } if *progress >= 0.5 && *job == JobKind::Normalize),
        );
        if half {
            break;
        }
        assert!(Instant::now() < end, "no progress");
        std::thread::sleep(Duration::from_millis(5));
    }
    m.cancel_save(MIXED).unwrap();
    match final_event(&ev) {
        EditorEvent::SaveCancelled { job, .. } => assert_eq!(job, JobKind::Normalize),
        e => panic!("{e:?}"),
    }
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert_eq!(fs::read(dir.join("track.json")).unwrap(), json);
    assert!(stray_parts(&dir).is_empty());
    assert!(trash_files(&repo).is_empty());
    assert_eq!(dir.join("player.json").exists(), player_json_existed);
}

// ====================================================================== criterion 8

#[test]
fn ac8_normalize_is_refused_without_a_backing_file() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, FOUR);
    let json = fs::read(dir.join("track.json")).unwrap();
    let (mut m, _b, ev) = manager();
    let snap = m.open(&spec(&repo, FOUR)).unwrap();
    assert!(!snap.variant.exists);
    let err = m
        .start_normalize(FOUR, repo.clone(), no_lock())
        .unwrap_err();
    assert!(err.contains("Save the backing first"), "{err}");
    assert!(m.snapshot(FOUR).unwrap().job.is_none());
    assert!(ev.lock().unwrap().iter().all(|e| !is_final(e)));
    assert_eq!(fs::read(dir.join("track.json")).unwrap(), json);
    assert!(
        !dir.join("backings").exists() || fs::read_dir(dir.join("backings")).unwrap().count() == 0
    );
}

// ====================================================================== criterion 9

/// Runs the swap under the lock of a `PlayerManager`, as the app's gate does under the host.
struct TestGate(Arc<Mutex<PlayerManager>>);

impl PlayerGate for TestGate {
    fn without_player(
        &self,
        id: &str,
        f: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut m = self.0.lock().unwrap();
        m.release_for_audio(id)?;
        f()
    }
}

fn player_spec(repo: &Repository, id: &str) -> player::OpenSpec {
    let rec = repo.load_record(id).unwrap();
    player::OpenSpec {
        id: rec.id.clone(),
        title: rec.title.clone(),
        dir: dir_of(repo, id),
        sources: player_source::sources(&rec),
        ffmpeg: None,
    }
}

fn closed_count(events: &Arc<Mutex<Vec<PlayerEvent>>>, id: &str) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, PlayerEvent::Closed { id: i } if i == id))
        .count()
}

#[test]
fn ac9_the_player_plays_the_normalized_file_after_normalize() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let pbackend = ManualBackend::new();
    let pevents: Arc<Mutex<Vec<PlayerEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let pe = pevents.clone();
    let psink: player::EventSink = Arc::new(move |e| pe.lock().unwrap().push(e));
    let pm = Arc::new(Mutex::new(PlayerManager::new(
        Arc::new(pbackend.clone()),
        psink,
    )));
    let pspec = player_spec(&repo, MIXED);
    assert_eq!(pspec.sources[0].key, "backing:backing");
    {
        let mut p = pm.lock().unwrap();
        p.open(&pspec).unwrap();
        p.play(MIXED, Instant::now()).unwrap();
    }
    let old: Vec<f32> = pbackend.pull(4000);
    assert!(
        old.iter().any(|v| v.abs() > 0.01),
        "the old file is audible"
    );

    let (mut m, _b, ev) = manager();
    m.set_player_gate(Arc::new(TestGate(pm.clone())));
    m.open(&spec(&repo, MIXED)).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    assert_eq!(closed_count(&pevents, MIXED), 1, "the Player was closed");
    assert!(!pm.lock().unwrap().is_open(MIXED));

    // the Player view reopens the track: it plays the new file
    let mut p = pm.lock().unwrap();
    p.open(&player_spec(&repo, MIXED)).unwrap();
    let t0 = Instant::now();
    p.seek(MIXED, 1000, t0).unwrap();
    p.play(MIXED, t0).unwrap();
    drop(p);
    let (info, decoded) = read_flac(&file);
    let scale = (1u64 << (info.bits_per_sample - 1)) as f32;
    let start = RATE as usize; // 1000 ms
    let got = pbackend.pull(8000);
    // the start of the sound is ramped in; after that the samples are the file's
    for i in 4000 * 2..8000 * 2 {
        let want = decoded[start * 2 + i] as f32 / scale;
        assert!(
            (got[i] - want).abs() < 1e-6,
            "sample {i}: {} vs {want}",
            got[i]
        );
    }
    assert!(got.iter().any(|v| v.abs() > 0.01));
}

// ====================================================================== QA extensions

fn write_flac_ch(path: &Path, samples: &[i32], channels: usize, bits: usize, rate: u32) {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, channels, bits, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

/// FOUR with all four stems rewritten at `rate`/`bits`: `vocals` (stereo) carries `pcm`, `bass`
/// is a mono silence, `drums` and `guitar` stereo silence. Vocals checked at 0 dB.
fn open_four_custom(
    pcm: &[f32],
    rate: u32,
    bits: u32,
) -> (
    tempfile::TempDir,
    Repository,
    EditorManager,
    ManualBackend,
    Events,
) {
    let (tmp, repo) = fixture_repo();
    let frames = pcm.len() / 2;
    let stems = dir_of(&repo, FOUR).join("stems");
    write_flac_ch(
        &stems.join("vocals.flac"),
        &quantise(pcm, bits),
        2,
        bits as usize,
        rate,
    );
    write_flac_ch(
        &stems.join("bass.flac"),
        &vec![0; frames],
        1,
        bits as usize,
        rate,
    );
    for n in ["drums", "guitar"] {
        write_flac_ch(
            &stems.join(format!("{n}.flac")),
            &vec![0; frames * 2],
            2,
            bits as usize,
            rate,
        );
    }
    let (mut m, b, ev) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true, Instant::now())
        .unwrap();
    (tmp, repo, m, b, ev)
}

#[test]
fn qa_save_normalizes_at_other_sample_rates_with_a_mono_stem_present() {
    for rate in [8000u32, 11025, 44100, 48000, 96000] {
        let pcm = at_lufs(
            rate,
            tone(rate, 8.0, -19.0, Some((2.0, 10.0, -12.0))),
            -22.0,
        );
        let (_t, repo, mut m, _b, ev) = open_four_custom(&pcm, rate, 16);
        let (file, result, clipped, _w) = save(&mut m, &repo, &ev);
        assert_eq!(result.outcome, Outcome::Normalized, "rate {rate}");
        let (info, _) = read_flac(&file);
        assert_eq!(info.sample_rate, rate);
        assert_eq!(info.channels, 2);
        near(
            measure_file(&file).lufs.unwrap(),
            -14.0,
            0.1,
            &format!("loudness at {rate} Hz"),
        );
        assert_eq!(clipped, 0);
    }
}

#[test]
fn qa_save_keeps_24_bits_and_hits_the_target_precisely() {
    let pcm = at_lufs(RATE, tone(RATE, 8.0, -19.0, None), -25.0);
    let (_t, repo, mut m, _b, ev) = open_four_custom(&pcm, RATE, 24);
    let (file, result, _c, _w) = save(&mut m, &repo, &ev);
    let (info, _) = read_flac(&file);
    assert_eq!(info.bits_per_sample, 24);
    near(
        measure_file(&file).lufs.unwrap(),
        -14.0,
        0.02,
        "24 bit loudness",
    );
    near(result.gain_db, 11.0, 0.1, "gain");
}

#[test]
fn qa_save_of_a_track_shorter_than_400_ms_is_written_untouched_as_too_quiet() {
    let pcm = tone(RATE, 0.3, -20.0, None);
    let (_t, repo, mut m, _b, ev) = open_four_custom(&pcm, RATE, 16);
    let (file, result, clipped, _w) = save(&mut m, &repo, &ev);
    assert_eq!(result.outcome, Outcome::TooQuiet);
    assert_eq!(result.gain_db, 0.0);
    let (info, samples) = read_flac(&file);
    assert_eq!(info.samples.unwrap(), (0.3 * RATE as f64) as u64);
    // unchanged level: the file holds the mix as it is
    let want = quantise(&pcm, 16);
    assert_eq!(samples, want);
    assert_eq!(clipped, 0);
    // and Normalize on it leaves it untouched, too
    let bytes = fs::read(&file).unwrap();
    let (r, changed, _) = normalize(&mut m, &repo, &ev, FOUR);
    assert!(!changed);
    assert_eq!(r.outcome, Outcome::TooQuiet);
    assert_eq!(fs::read(&file).unwrap(), bytes);
}

#[test]
fn qa_save_of_digital_silence_is_too_quiet_and_writes_silence() {
    let pcm = vec![0.0f32; RATE as usize * 2 * 2];
    let (_t, repo, mut m, _b, ev) = open_four_custom(&pcm, RATE, 16);
    let (file, result, _c, _w) = save(&mut m, &repo, &ev);
    assert_eq!(result.outcome, Outcome::TooQuiet);
    assert!(read_flac(&file).1.iter().all(|&s| s == 0));
}

#[test]
fn qa_a_record_whose_md5_no_longer_matches_is_measured_again() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    // the file is replaced by something quieter behind the app's back; the record still says -14
    let (info, samples) = read_flac(&file);
    let quiet: Vec<i32> = samples.iter().map(|s| s / 4).collect();
    write_flac(
        &file,
        &quiet,
        info.bits_per_sample as usize,
        info.sample_rate,
    );
    let before = measure_file(&file).lufs.unwrap();
    assert!(before < -16.0, "{before}");
    // a fresh session (the file changed on disk)
    let (mut m2, _b2, ev2) = manager();
    m2.open(&spec(&repo, MIXED)).unwrap();
    let (r, changed, _) = normalize(&mut m2, &repo, &ev2, MIXED);
    assert!(changed, "the stale record must not be trusted: {r:?}");
    near(measure_file(&file).lufs.unwrap(), -14.0, 0.1, "loudness");
}

#[test]
fn qa_a_hand_edited_record_that_claims_the_target_is_checked_against_the_md5() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let bytes = fs::read(&file).unwrap();
    let mut j = json_of(&dir);
    j["backings"][0]["loudness"] = serde_json::json!({
        "lufs": -14.0, "true_peak": -3.0, "gain_db": 0.0, "target": -14.0,
        "ceiling": -1.0, "md5": "00000000000000000000000000000000"
    });
    fs::write(
        dir.join("track.json"),
        serde_json::to_vec_pretty(&j).unwrap(),
    )
    .unwrap();
    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed, "a lying record must not skip the measurement");
    assert_ne!(fs::read(&file).unwrap(), bytes);
    near(measure_file(&file).lufs.unwrap(), -14.0, 0.1, "loudness");
}

#[test]
fn qa_unknown_keys_in_the_loudness_record_are_ignored_and_other_variants_stay_untouched() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    // a second variant with its own file and a loudness record with a future key
    let first = dir.join("backings/backing.flac");
    let second = dir.join("backings/other.flac");
    fs::copy(&first, &second).unwrap();
    let mut j = json_of(&dir);
    let mut v2 = j["backings"][0].clone();
    v2["id"] = "other".into();
    v2["name"] = "Other".into();
    v2["file"] = "backings/other.flac".into();
    v2["loudness"] = serde_json::json!({
        "lufs": -30.0, "true_peak": -20.0, "gain_db": 0.0, "target": -14.0,
        "ceiling": -1.0, "md5": "0123456789abcdef0123456789abcdef", "future_key": {"x": 1}
    });
    j["backings"].as_array_mut().unwrap().push(v2.clone());
    fs::write(
        dir.join("track.json"),
        serde_json::to_vec_pretty(&j).unwrap(),
    )
    .unwrap();
    let second_bytes = fs::read(&second).unwrap();

    let rec = repo.load_record(MIXED).expect("loads with a future key");
    assert_eq!(rec.backings.len(), 2);

    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    assert_eq!(
        fs::read(&second).unwrap(),
        second_bytes,
        "other variant file"
    );
    let after = json_of(&dir);
    assert_eq!(after["backings"][1]["file"], v2["file"]);
    assert_eq!(after["backings"][1]["mix"], v2["mix"]);
    assert_eq!(
        after["backings"][1]["loudness"]["lufs"], -30.0,
        "other variant's record"
    );
    assert!(after["backings"][0]["loudness"].is_object());
}

#[test]
fn qa_normalize_puts_the_old_file_in_the_trash_byte_for_byte() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let old = fs::read(dir.join("backings/backing.flac")).unwrap();
    let (mut m, _b, ev) = manager();
    m.open(&spec(&repo, MIXED)).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
    let trash = trash_files(&repo);
    assert!(
        trash
            .iter()
            .any(|p| fs::read(p).map(|b| b == old).unwrap_or(false)),
        "old backing not found in the trash: {trash:?}"
    );
}

fn wait_for_progress(ev: &Events, max: f32) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let seen = ev.lock().unwrap().iter().any(
            |e| matches!(e, EditorEvent::Saving { progress, job, .. } if *progress > 0.0 && *progress <= max && *job == JobKind::Normalize),
        );
        if seen {
            return;
        }
        assert!(Instant::now() < end, "no progress");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn qa_cancel_in_the_measuring_phase_leaves_everything_as_it_was() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let bytes = fs::read(&file).unwrap();
    let json = fs::read(dir.join("track.json")).unwrap();
    let (mut m, _b, ev) = manager();
    m.set_job_pace(Some(Duration::from_millis(30)));
    m.open(&spec(&repo, MIXED)).unwrap();
    m.start_normalize(MIXED, repo.clone(), no_lock()).unwrap();
    wait_for_progress(&ev, 0.3);
    m.cancel_save(MIXED).unwrap();
    assert!(matches!(
        final_event(&ev),
        EditorEvent::SaveCancelled {
            job: JobKind::Normalize,
            ..
        }
    ));
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert_eq!(fs::read(dir.join("track.json")).unwrap(), json);
    assert!(stray_parts(&dir).is_empty());
    assert!(trash_files(&repo).is_empty());
    // and a new Normalize works afterwards
    m.set_job_pace(None);
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(changed);
}

#[test]
fn qa_a_file_changed_between_measure_and_swap_is_a_conflict_and_is_kept() {
    let (_t, repo) = fixture_repo();
    let dir = dir_of(&repo, MIXED);
    let file = dir.join("backings/backing.flac");
    let (mut m, _b, ev) = manager();
    m.set_job_pace(Some(Duration::from_millis(20)));
    m.open(&spec(&repo, MIXED)).unwrap();
    m.start_normalize(MIXED, repo.clone(), no_lock()).unwrap();
    wait_for_progress(&ev, 0.3);
    // somebody else rewrites the file during the job
    let (info, samples) = read_flac(&file);
    let other: Vec<i32> = samples.iter().map(|s| s / 2).collect();
    let tmp = dir.join("backings/elsewhere.flac");
    write_flac(
        &tmp,
        &other,
        info.bits_per_sample as usize,
        info.sample_rate,
    );
    fs::rename(&tmp, &file).unwrap();
    let theirs = fs::read(&file).unwrap();
    match final_event(&ev) {
        EditorEvent::SaveFailed { message, job, .. } => {
            assert_eq!(job, JobKind::Normalize);
            assert!(
                message.contains("conflict") || message.contains("changed"),
                "{message}"
            );
        }
        e => panic!("expected a conflict, got {e:?}"),
    }
    assert_eq!(fs::read(&file).unwrap(), theirs, "their file is kept");
    assert!(stray_parts(&dir).is_empty());
}

#[test]
fn qa_normalize_with_the_player_playing_closes_it_once_and_the_audio_stays_valid() {
    // ac9 covers the reopened file; here the Player is mid-playback with the gate and a second
    // Normalize (no change) must not close it again.
    let (_t, repo) = fixture_repo();
    let pbackend = ManualBackend::new();
    let pevents: Arc<Mutex<Vec<PlayerEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let pe = pevents.clone();
    let psink: player::EventSink = Arc::new(move |e| pe.lock().unwrap().push(e));
    let pm = Arc::new(Mutex::new(PlayerManager::new(
        Arc::new(pbackend.clone()),
        psink,
    )));
    {
        let mut p = pm.lock().unwrap();
        p.open(&player_spec(&repo, MIXED)).unwrap();
        p.play(MIXED, Instant::now()).unwrap();
    }
    let (mut m, _b, ev) = manager();
    m.set_player_gate(Arc::new(TestGate(pm.clone())));
    m.open(&spec(&repo, MIXED)).unwrap();
    normalize(&mut m, &repo, &ev, MIXED);
    assert_eq!(closed_count(&pevents, MIXED), 1);
    // the Player reopens and plays; the second Normalize changes nothing and leaves it open
    pm.lock().unwrap().open(&player_spec(&repo, MIXED)).unwrap();
    pm.lock().unwrap().play(MIXED, Instant::now()).unwrap();
    let (_r, changed, _) = normalize(&mut m, &repo, &ev, MIXED);
    assert!(!changed);
    assert_eq!(closed_count(&pevents, MIXED), 1, "no second close");
    assert!(pm.lock().unwrap().is_open(MIXED));
}
