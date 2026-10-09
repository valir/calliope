//! QA acceptance and data-safety tests for gui-backing-track-editor (Rust side).
//!
//! The crate is a binary, so the modules are compiled into this test crate with `#[path]`.
//! Audio only ever goes to `ManualBackend` / `NullBackend` (never a real device). Every test
//! works on a temp copy of `tests/fixtures/library-editor`.
#![allow(dead_code, unused_imports, unused_variables, clippy::type_complexity, clippy::all)]

#[path = "../src/fsutil.rs"]
mod fsutil;
#[path = "../src/track_meta.rs"]
mod track_meta;
#[path = "../src/repository.rs"]
mod repository;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/picker.rs"]
mod picker;
#[path = "../src/import_tmp.rs"]
mod import_tmp;
#[path = "../src/media.rs"]
mod media;
#[path = "../src/download.rs"]
mod download;
#[path = "../src/tools.rs"]
mod tools;
#[path = "../src/import_job.rs"]
mod import_job;
#[path = "../src/transport.rs"]
mod transport;
#[path = "../src/mixer.rs"]
mod mixer;
#[path = "../src/stem_audio.rs"]
mod stem_audio;
#[path = "../src/audio_out.rs"]
mod audio_out;
#[path = "../src/backing_render.rs"]
mod backing_render;
#[path = "../src/editor.rs"]
mod editor;
#[path = "../src/ipc.rs"]
mod ipc;

pub const VERSION: &str = env!("CALLIOPE_VERSION");

use audio_out::{backend_kind, BackendKind, ManualBackend};
use editor::{EditorEvent, EditorManager, EventSink, OpenSpec};
use import_job::RepoLock;
use repository::Repository;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
const MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
const PLAIN: &str = "0199c0a0-0000-7000-8000-000000000203";
const SIX: &str = "0199c0a0-0000-7000-8000-000000000204";
const MISSING: &str = "0199c0a0-0000-7000-8000-000000000205";
const RATE: usize = 22050;

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

/// A repository (`<tmp>/repo`) holding the five fixture tracks.
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
    }
}

fn manager() -> (EditorManager, ManualBackend, Events) {
    let backend = ManualBackend::new();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
    (EditorManager::new(Box::new(backend.clone()), sink), backend, events)
}

fn no_lock() -> Arc<dyn RepoLock> {
    Arc::new(|f: &mut dyn FnMut()| f())
}

fn final_event(events: &Events) -> EditorEvent {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let found = events
            .lock()
            .unwrap()
            .iter()
            .find(|e| matches!(e, EditorEvent::Saved { .. } | EditorEvent::SaveFailed { .. } | EditorEvent::SaveCancelled { .. }))
            .cloned();
        if let Some(e) = found {
            return e;
        }
        assert!(Instant::now() < end, "the save did not finish");
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

fn read_flac(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

fn trash_backing_files(repo: &Repository) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(repo.root.join("trash")) else { return out };
    for d in rd.flatten() {
        if d.file_name().to_string_lossy().ends_with("-backing") || d.file_name().to_string_lossy().contains("-backing") {
            for f in fs::read_dir(d.path()).unwrap().flatten() {
                out.push(f.path());
            }
        }
    }
    out
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

fn decode(dir: &Path, stem: &str) -> stem_audio::StemPcm {
    stem_audio::decode_i16(&dir.join(format!("stems/{stem}.flac")), &std::sync::atomic::AtomicBool::new(false), &mut |_, _| {}).unwrap()
}

fn stereo(pcm: &stem_audio::StemPcm, start: usize, frames: usize) -> Vec<f32> {
    let mut v = vec![0.0; frames * 2];
    mixer::to_stereo_f32(pcm, start as u64, frames, &mut v);
    v
}

fn assert_close(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!((x - y).abs() < 1e-5, "sample {i}: {x} vs {y}");
    }
}

fn write_flac(path: &Path, samples: &[i32], channels: usize, bits: usize, rate: usize) {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(samples, channels, bits, rate);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

/// Opens FOUR in a fresh repo with vocals checked, ready for `start_save`.
fn ready_to_save() -> (tempfile::TempDir, Repository, OpenSpec, EditorManager, ManualBackend, Events) {
    let (tmp, repo) = fixture_repo();
    let sp = spec(&repo, FOUR);
    let (mut m, b, ev) = manager();
    m.open(&sp).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    (tmp, repo, sp, m, b, ev)
}

// ============================================================ criteria 1, 2 (activation)

#[test]
fn ac1_track_without_stems_cannot_open_and_missing_stem_names_the_file() {
    let (_t, repo) = fixture_repo();
    let (mut m, _b, _e) = manager();
    let err = m.open(&spec(&repo, PLAIN)).unwrap_err();
    assert!(err.contains("has no stems"), "{err}");
    assert!(!m.is_open(PLAIN));
    let err = m.open(&spec(&repo, MISSING)).unwrap_err();
    assert!(err.contains("stems/") && err.contains("missing"), "{err}");
    assert!(m.current().is_none());
}

#[test]
fn ac2_track_with_stems_opens_with_one_lane_per_stem_and_original_is_hidden() {
    let (_t, repo) = fixture_repo();
    // give the four-lane track a kept original mix, as the stem import does
    let dir = dir_of(&repo, FOUR);
    let mut v = json_of(&dir);
    v["original"] = json!("original.flac");
    fs::copy(dir.join("stems/vocals.flac"), dir.join("original.flac")).unwrap();
    fs::write(dir.join("track.json"), serde_json::to_vec_pretty(&v).unwrap()).unwrap();

    let st = ipc::EditorState::new(Box::new(ManualBackend::new()));
    let repo_state = ipc::RepoState::new(repo.root.clone(), Box::new(picker::ScriptedPicker::new(repo.root.join("none"))));
    let snap = ipc::do_open_editor(&st, &repo_state, FOUR, Arc::new(|_| {})).unwrap();
    let names: Vec<_> = snap.stems.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar"], "original must not be a lane");
    assert_eq!(snap.duration_ms, 12000);

    let (mut m, _b, _e) = manager();
    assert_eq!(m.open(&spec(&repo, SIX)).unwrap().stems.len(), 6, "as many lanes as stems");
}

// ========================================== owner: never-saved starts unchecked; Play gating

#[test]
fn ac4_never_saved_track_starts_unchecked_and_play_needs_a_checked_stem() {
    let (_t, repo) = fixture_repo();
    let (mut m, b, _e) = manager();
    let snap = m.open(&spec(&repo, FOUR)).unwrap();
    assert!(snap.stems.iter().all(|l| !l.unmuted && l.gain_db == Some(0.0)));
    assert_eq!(m.play(FOUR).unwrap_err(), "Check a stem first");
    assert!(!b.is_open(), "no output stream may be opened by a refused Play");
    assert_eq!(m.start_save(FOUR, repo.clone(), no_lock()).unwrap_err(), "Check a stem first");
    m.set_stem(FOUR, "drums", Some(0.0), true).unwrap();
    assert!(m.play(FOUR).unwrap().playing);
}

#[test]
fn owner_saved_mix_is_restored_on_open() {
    let (_t, repo) = fixture_repo();
    let (mut m, _b, _e) = manager();
    let snap = m.open(&spec(&repo, MIXED)).unwrap();
    assert_eq!(snap.stems[0].unmuted, true);
    assert_eq!(snap.stems[0].gain_db, Some(-6.0));
    assert_eq!(snap.stems[1].unmuted, false);
    assert!(snap.variant.exists);
}

// ===================================================== criteria 3, 5, 9, 10 (play, solo, pause)

#[test]
fn ac3_lane_play_checks_its_unmute_plays_only_that_stem_then_mix_play_returns_to_mix() {
    let (_t, repo) = fixture_repo();
    let sp = spec(&repo, FOUR);
    let (mut m, b, _e) = manager();
    m.open(&sp).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let st = m.lane_play(FOUR, "guitar").unwrap();
    assert!(st.playing);
    assert_eq!(st.solo.as_deref(), Some("guitar"));
    let un: Vec<bool> = m.snapshot(FOUR).unwrap().stems.iter().map(|l| l.unmuted).collect();
    assert_eq!(un, [true, false, false, true]);
    let gui = decode(&sp.dir, "guitar");
    assert_close(&b.pull(400), &stereo(&gui, 0, 400));
    // Mix Play: solo ends, guitar (now checked) plays together with vocals
    let st = m.play(FOUR).unwrap();
    assert_eq!(st.solo, None);
    let voc = decode(&sp.dir, "vocals");
    let want: Vec<f32> = stereo(&gui, 400, 300).iter().zip(stereo(&voc, 400, 300)).map(|(a, b)| (a + b).clamp(-1.0, 1.0)).collect();
    assert_close(&b.pull(300), &want);
    // solo never touches volumes
    assert!(m.snapshot(FOUR).unwrap().stems.iter().all(|l| l.gain_db == Some(0.0)));
}

#[test]
fn ac5_play_mixes_exactly_the_checked_stems_with_db_gains_and_off() {
    let (_t, repo) = fixture_repo();
    let sp = spec(&repo, FOUR);
    let (mut m, b, _e) = manager();
    m.open(&sp).unwrap();
    m.set_stem(FOUR, "vocals", Some(-6.0), true).unwrap();
    m.set_stem(FOUR, "drums", Some(3.0), true).unwrap();
    m.set_stem(FOUR, "bass", None, true).unwrap(); // Off: silent even if checked
    m.set_stem(FOUR, "guitar", Some(0.0), false).unwrap(); // unchecked: silent
    m.play(FOUR).unwrap();
    let (v, d) = (decode(&sp.dir, "vocals"), decode(&sp.dir, "drums"));
    let (gv, gd) = (10f32.powf(-6.0 / 20.0), 10f32.powf(3.0 / 20.0));
    let want: Vec<f32> = stereo(&v, 0, 2000).iter().zip(stereo(&d, 0, 2000)).map(|(a, b)| (a * gv + b * gd).clamp(-1.0, 1.0)).collect();
    assert_close(&b.pull(2000), &want);
}

#[test]
fn ac9_ac10_pause_holds_position_and_resume_continues() {
    let (_t, repo) = fixture_repo();
    let sp = spec(&repo, FOUR);
    let (mut m, b, _e) = manager();
    m.open(&sp).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.pull(RATE); // 1 s
    let st = m.pause(FOUR).unwrap();
    assert!(!st.playing);
    assert!((st.position_ms as i64 - 1000).abs() <= 25, "{}", st.position_ms);
    assert_eq!(peak(&b.pull(5000)), 0.0);
    assert_eq!(m.snapshot(FOUR).unwrap().transport.position_ms, st.position_ms);
    m.play(FOUR).unwrap();
    let voc = decode(&sp.dir, "vocals");
    assert_close(&b.pull(100), &stereo(&voc, RATE, 100));
}

#[test]
fn ac8_stop_returns_to_zero() {
    let (_t, repo) = fixture_repo();
    let (mut m, b, _e) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.pull(5000);
    let st = m.stop(FOUR).unwrap();
    assert_eq!((st.playing, st.position_ms), (false, 0));
    assert!(!b.is_open());
}

// ============================ criteria 11-15 (seek / wheel / 0.3 s rule) with injected time

#[test]
fn ac11_seek_clamps_and_ac12_ac13_nudge_is_100ms_per_step() {
    let (_t, repo) = fixture_repo();
    let (mut m, _b, _e) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    let t = Instant::now();
    assert_eq!(m.seek(FOUR, 5000, t).unwrap().position_ms, 5000);
    assert_eq!(m.nudge(FOUR, -100, t).unwrap().position_ms, 4900); // wheel up -> earlier
    assert_eq!(m.nudge(FOUR, 100, t).unwrap().position_ms, 5000); // wheel down -> later
    assert_eq!(m.seek(FOUR, 999_999, t).unwrap().position_ms, 12000);
    assert_eq!(m.nudge(FOUR, 100, t).unwrap().position_ms, 12000);
    m.seek(FOUR, 0, t).unwrap();
    assert_eq!(m.nudge(FOUR, -100, t).unwrap().position_ms, 0);
}

#[test]
fn ac14_ac15_resume_exactly_300ms_after_the_last_adjustment() {
    let (_t, repo) = fixture_repo();
    let sp = spec(&repo, FOUR);
    let (mut m, b, _e) = manager();
    m.open(&sp).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.pull(100);
    let t0 = Instant::now();
    let st = m.seek(FOUR, 5000, t0).unwrap();
    assert!(st.playing && st.resume_pending, "button stays Pause during the delay");
    assert_eq!(peak(&b.pull(300)), 0.0, "silent right after the adjustment");
    // second adjustment inside the window resets it to 300 ms from *now*
    let st = m.nudge(FOUR, 100, t0 + Duration::from_millis(200)).unwrap();
    assert_eq!(st.position_ms, 5100);
    m.tick(t0 + Duration::from_millis(499));
    assert_eq!(peak(&b.pull(300)), 0.0, "t0+299 after the second change is still silent");
    assert!(m.snapshot(FOUR).unwrap().transport.resume_pending);
    m.tick(t0 + Duration::from_millis(500));
    assert!(!m.snapshot(FOUR).unwrap().transport.resume_pending);
    let voc = decode(&sp.dir, "vocals");
    let start = 5100 * RATE / 1000 + 0;
    assert_close(&b.pull(200), &stereo(&voc, start, 200));
}

#[test]
fn adjusting_while_paused_does_not_start_playback() {
    let (_t, repo) = fixture_repo();
    let (mut m, b, _e) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let t0 = Instant::now();
    m.seek(FOUR, 3000, t0).unwrap();
    m.tick(t0 + Duration::from_secs(5));
    let s = m.snapshot(FOUR).unwrap().transport;
    assert!(!s.playing && !s.resume_pending);
    assert_eq!(peak(&b.pull(1000)), 0.0);
}

// ================================================ owner: clipping

#[test]
fn owner_hard_clip_and_clip_indicator_hold() {
    let (_t, repo) = fixture_repo();
    let (mut m, b, _e) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    for n in ["vocals", "drums", "bass", "guitar"] {
        m.set_stem(FOUR, n, Some(12.0), true).unwrap();
    }
    m.play(FOUR).unwrap();
    let out = b.pull(4000);
    assert!(out.iter().all(|x| (-1.0..=1.0).contains(x)), "hard clipped to full scale");
    let t = Instant::now();
    m.tick(t);
    assert!(m.snapshot(FOUR).unwrap().transport.clipping);
    // the hold ends about a second after the last clipped sample
    m.pause(FOUR).unwrap();
    m.tick(t + Duration::from_millis(1500));
    assert!(!m.snapshot(FOUR).unwrap().transport.clipping);
}

#[test]
fn owner_gain_range_is_minus60_off_to_plus12() {
    assert_eq!(mixer::MIN_DB, -60.0);
    assert_eq!(mixer::MAX_DB, 12.0);
    assert_eq!(mixer::gain_factor(Some(-60.0), true), 0.0);
    assert_eq!(mixer::gain_factor(None, true), 0.0);
    assert_eq!(mixer::gain_factor(Some(0.0), false), 0.0);
    assert!((mixer::gain_factor(Some(0.0), true) - 1.0).abs() < 1e-6);
    assert!((mixer::gain_factor(Some(12.0), true) - 3.981).abs() < 1e-3);
    assert_eq!(mixer::clamp_db(99.0), Some(12.0));
    assert_eq!(mixer::clamp_db(-80.0), None);
    assert_eq!(mixer::clamp_db(1.26), Some(1.5));
}

// ====================================== owner: playback stops on track change / close

#[test]
fn owner_track_change_and_close_stop_playback() {
    let (_t, repo) = fixture_repo();
    let (mut m, b, _e) = manager();
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    assert!(b.is_open());
    m.open(&spec(&repo, SIX)).unwrap(); // select another track
    assert!(!b.is_open(), "the old track's output is closed");
    assert_eq!(peak(&b.pull(1000)), 0.0);
    assert_eq!(m.play(FOUR).unwrap_err(), "no editor session for 0199c0a0-0000-7000-8000-000000000201");
    // closing (a track without stems selected) stops too
    m.set_stem(SIX, "vocals", Some(0.0), true).unwrap();
    m.play(SIX).unwrap();
    m.close();
    assert!(!b.is_open());
}

// ============================================================ criteria 16, 17 (save)

#[test]
fn ac16_ac17_save_writes_one_backing_variant_with_volumes_applied() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    m.set_stem(FOUR, "vocals", Some(-6.0), true).unwrap();
    m.set_stem(FOUR, "bass", Some(3.5), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::Saved { file, clipped_samples, track, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert_eq!(file, "backings/backing.flac");
    assert_eq!(track.backings.len(), 1);
    let (v, b) = (decode(&sp.dir, "vocals"), decode(&sp.dir, "bass"));
    let (gv, gb) = (mixer::gain_factor(Some(-6.0), true), mixer::gain_factor(Some(3.5), true));
    let frames = v.frames.max(b.frames) as usize;
    let (sv, sb) = (stereo(&v, 0, frames), stereo(&b, 0, frames));
    let want: Vec<i32> = (0..frames * 2).map(|i| mixer::quantise((sv[i] * gv + sb[i] * gb).clamp(-1.0, 1.0), 16)).collect();
    let (info, got) = read_flac(&sp.dir.join(&file));
    assert_eq!((info.channels, info.sample_rate, info.bits_per_sample), (2, 22050, 16));
    assert_eq!(info.samples, Some(frames as u64));
    assert!(got == want, "saved samples differ from the configured mix");
    // unchecked stems are not in the file: only vocals+bass contribute (checked above exactly)
    let meta = json_of(&sp.dir);
    assert_eq!(meta["type"], "stem");
    assert_eq!(meta["audio"], Value::Null);
    assert_eq!(meta["backings"].as_array().unwrap().len(), 1);
    assert_eq!(meta["backings"][0]["id"], "backing");
    assert_eq!(meta["backings"][0]["name"], "Backing");
    assert_eq!(meta["backings"][0]["file"], "backings/backing.flac");
    assert_eq!(meta["backings"][0]["mix"]["stems"][2], json!({"name": "bass", "gain_db": 3.5, "unmuted": true}));
    assert!(stray_parts(&sp.dir).is_empty());
    // the stems were not touched
    assert!(sp.dir.join("stems/vocals.flac").is_file());
    // re-reading the repository sees a healthy track
    assert!(repo.scan().problems.is_empty());
}

#[test]
fn owner_resave_replaces_the_one_variant_and_trashes_the_old_file() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    let first = fs::read(sp.dir.join("backings/backing.flac")).unwrap();
    clear(&ev);
    m.tick(Instant::now()); // reap
    m.set_stem(FOUR, "drums", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    let second = fs::read(sp.dir.join("backings/backing.flac")).unwrap();
    assert_ne!(first, second);
    let meta = json_of(&sp.dir);
    assert_eq!(meta["backings"].as_array().unwrap().len(), 1, "one Backing variant only");
    let files: Vec<_> = fs::read_dir(sp.dir.join("backings")).unwrap().flatten().collect();
    assert_eq!(files.len(), 1);
    let trashed = trash_backing_files(&repo);
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert_eq!(fs::read(&trashed[0]).unwrap(), first, "the old backing is in the trash, intact");
    assert!(trashed[0].to_string_lossy().contains("-backing"));
}

#[test]
fn owner_reopen_restores_saved_mix_and_remembers_across_track_changes() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    m.set_stem(FOUR, "vocals", Some(-12.0), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    // fresh manager = app restart
    let (mut m2, _b2, _e2) = manager();
    let snap = m2.open(&spec(&repo, FOUR)).unwrap();
    assert_eq!(snap.stems[0].gain_db, Some(-12.0));
    assert!(snap.stems[0].unmuted && !snap.stems[1].unmuted);
}

#[test]
fn save_keeps_unknown_fields_and_library_edits_made_meanwhile() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    // an unknown field written by some other version + a library title edit during the save
    let path = sp.dir.join("track.json");
    let mut v = json_of(&sp.dir);
    v["future_field"] = json!({"keep": [1, 2, 3]});
    fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let path2 = path.clone();
    let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
        let mut v: Value = serde_json::from_slice(&fs::read(&path2).unwrap()).unwrap();
        v["title"] = json!("Renamed meanwhile");
        fs::write(&path2, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        f();
    });
    m.start_save(FOUR, repo.clone(), lock).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    let meta = json_of(&sp.dir);
    assert_eq!(meta["title"], "Renamed meanwhile");
    assert_eq!(meta["future_field"], json!({"keep": [1, 2, 3]}));
    assert_eq!(meta["backings"].as_array().unwrap().len(), 1);
}

// ============================================================ data safety

#[test]
fn safety_preexisting_user_file_named_backing_flac_is_never_overwritten() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    fs::create_dir(sp.dir.join("backings")).unwrap();
    fs::write(sp.dir.join("backings/backing.flac"), b"MY OWN FILE").unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::Saved { file, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert_ne!(file, "backings/backing.flac");
    assert_eq!(fs::read(sp.dir.join("backings/backing.flac")).unwrap(), b"MY OWN FILE");
    assert!(read_flac(&sp.dir.join(&file)).1.len() > 0);
    // the user file is not in the trash either, and a re-save replaces our file, not theirs
    clear(&ev);
    m.tick(Instant::now());
    m.set_stem(FOUR, "drums", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::Saved { file: file2, .. } = final_event(&ev) else { panic!() };
    assert_eq!(file2, file);
    assert_eq!(fs::read(sp.dir.join("backings/backing.flac")).unwrap(), b"MY OWN FILE");
    assert_eq!(json_of(&sp.dir)["backings"].as_array().unwrap().len(), 1);
}

#[test]
fn safety_user_files_with_case_variants_and_dangling_symlink_are_not_clobbered() {
    let (tmp, repo, sp, mut m, _b, ev) = ready_to_save();
    let outside = tmp.path().join("outside.flac");
    fs::write(&outside, b"OUTSIDE").unwrap();
    fs::create_dir(sp.dir.join("backings")).unwrap();
    std::os::unix::fs::symlink(&outside, sp.dir.join("backings/backing.flac")).unwrap();
    fs::write(sp.dir.join("backings/Backing-2.FLAC"), b"CASE").unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::Saved { file, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert_eq!(fs::read(&outside).unwrap(), b"OUTSIDE");
    assert!(fs::symlink_metadata(sp.dir.join("backings/backing.flac")).unwrap().file_type().is_symlink());
    assert_eq!(fs::read(sp.dir.join("backings/Backing-2.FLAC")).unwrap(), b"CASE");
    assert_ne!(file, "backings/backing.flac");
}

#[test]
fn safety_symlinked_backings_folder_is_refused_and_nothing_is_written_outside() {
    let (tmp, repo, sp, mut m, _b, ev) = ready_to_save();
    let outside = tmp.path().join("elsewhere");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, sp.dir.join("backings")).unwrap();
    let before = fs::read(sp.dir.join("track.json")).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::SaveFailed { message, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert!(message.contains("backings"), "{message}");
    let names: Vec<_> = fs::read_dir(&outside).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["keep.txt"]);
    assert_eq!(fs::read(sp.dir.join("track.json")).unwrap(), before);
    assert!(stray_parts(&sp.dir).is_empty());
    // the job is over; the editor is usable again
    m.tick(Instant::now());
    assert_eq!(m.snapshot(FOUR).unwrap().saving, None);
}

#[test]
fn safety_stems_renamed_on_disk_during_save_is_a_conflict_and_leaves_nothing() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    let path = sp.dir.join("track.json");
    let p2 = path.clone();
    let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
        let t = fs::read_to_string(&p2).unwrap().replace("\"name\": \"guitar\"", "\"name\": \"lead\"");
        fs::write(&p2, t).unwrap();
        f();
    });
    m.start_save(FOUR, repo.clone(), lock).unwrap();
    let EditorEvent::SaveFailed { message, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert!(message.starts_with("conflict:"), "{message}");
    assert!(!sp.dir.join("backings").exists());
    assert!(stray_parts(&sp.dir).is_empty());
    assert!(trash_backing_files(&repo).is_empty());
}

#[test]
fn safety_track_deleted_during_save_fails_cleanly() {
    let (tmp, repo, sp, mut m, _b, ev) = ready_to_save();
    let (dir, aside) = (sp.dir.clone(), tmp.path().join("moved-away"));
    let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
        fs::rename(&dir, &aside).unwrap();
        f();
    });
    m.start_save(FOUR, repo.clone(), lock).unwrap();
    let EditorEvent::SaveFailed { .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert!(!sp.dir.exists(), "a recreated track folder would be a defect");
    assert!(!repo.tracks_dir().join(FOUR).exists());
}

#[test]
fn safety_stem_file_removed_during_save_does_not_corrupt_anything() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    // the render has to read the stems; remove the checked one before the job starts reading
    fs::remove_file(sp.dir.join("stems/vocals.flac")).unwrap();
    let before = fs::read(sp.dir.join("track.json")).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let EditorEvent::SaveFailed { message, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert!(message.contains("vocals"), "{message}");
    assert_eq!(fs::read(sp.dir.join("track.json")).unwrap(), before);
    assert!(!sp.dir.join("backings").exists());
    assert!(stray_parts(&sp.dir).is_empty());
}

#[test]
fn safety_stem_replaced_by_other_sample_rate_during_session_must_not_save_a_mislabelled_file() {
    // The stems keep their names/files, so the "same stems" check passes; the render must
    // still not label audio of another rate with the session's rate.
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    write_flac(&sp.dir.join("stems/vocals.flac"), &vec![1000; 44100 * 2], 2, 16, 44100);
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    match final_event(&ev) {
        EditorEvent::SaveFailed { .. } => {}
        EditorEvent::Saved { file, .. } => {
            let (info, _) = read_flac(&sp.dir.join(file));
            panic!(
                "DEFECT: saved a {} Hz file that mixes a 44100 Hz stem as if it were 22050 Hz (no per-stem rate check in StemReader/render)",
                info.sample_rate
            );
        }
        e => panic!("{e:?}"),
    }
}

#[test]
fn safety_cancel_mid_save_keeps_the_existing_backing_and_track_json() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    m.tick(Instant::now());
    let old_file = fs::read(sp.dir.join("backings/backing.flac")).unwrap();
    let old_json = fs::read(sp.dir.join("track.json")).unwrap();
    // second save, cancelled from the first progress event
    let ev2: Events = Arc::new(Mutex::new(Vec::new()));
    let (e2, shared) = (ev2.clone(), backing_render::SaveShared::new());
    let sh = shared.clone();
    let sink: EventSink = Arc::new(move |e| {
        if matches!(e, EditorEvent::Saving { .. }) {
            sh.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        e2.lock().unwrap().push(e);
    });
    let params = backing_render::SaveParams {
        id: FOUR.into(),
        variant: "backing".into(),
        dir: sp.dir.clone(),
        stems: sp.stems.clone(),
        input: backing_render::RenderInput {
            stems: vec![backing_render::RenderStem { path: sp.dir.join("stems/drums.flac"), gain: 1.0 }],
            sample_rate: 22050,
            bits: 16,
            frames: 12 * 22050,
        },
        mix: json!({}),
        repo: repo.clone(),
        lock: no_lock(),
    };
    backing_render::spawn(params, shared, sink).join().unwrap();
    assert!(matches!(final_event(&ev2), EditorEvent::SaveCancelled { .. }));
    assert_eq!(fs::read(sp.dir.join("backings/backing.flac")).unwrap(), old_file);
    assert_eq!(fs::read(sp.dir.join("track.json")).unwrap(), old_json);
    assert!(stray_parts(&sp.dir).is_empty());
    assert!(trash_backing_files(&repo).is_empty(), "a cancelled save must not trash the old backing");
}

#[test]
fn safety_read_only_track_folder_fails_with_a_message_and_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    let before = fs::read(sp.dir.join("track.json")).unwrap();
    fs::set_permissions(&sp.dir, fs::Permissions::from_mode(0o555)).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let r = final_event(&ev);
    fs::set_permissions(&sp.dir, fs::Permissions::from_mode(0o755)).unwrap();
    let EditorEvent::SaveFailed { message, .. } = r else { panic!("{r:?}") };
    assert!(!message.is_empty());
    assert_eq!(fs::read(sp.dir.join("track.json")).unwrap(), before);
    assert!(!sp.dir.join("backings").exists());
    assert!(stray_parts(&sp.dir).is_empty());
    // after the folder is writable again the same session can save
    m.tick(Instant::now());
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }) || ev.lock().unwrap().len() > 1);
}

#[test]
fn safety_read_only_backings_folder_restores_the_old_backing() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    m.tick(Instant::now());
    clear(&ev);
    let old = fs::read(sp.dir.join("backings/backing.flac")).unwrap();
    let json_before = fs::read(sp.dir.join("track.json")).unwrap();
    fs::set_permissions(sp.dir.join("backings"), fs::Permissions::from_mode(0o555)).unwrap();
    m.set_stem(FOUR, "drums", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let r = final_event(&ev);
    fs::set_permissions(sp.dir.join("backings"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(r, EditorEvent::SaveFailed { .. }), "{r:?}");
    assert_eq!(fs::read(sp.dir.join("backings/backing.flac")).unwrap(), old, "old backing still in place");
    assert_eq!(fs::read(sp.dir.join("track.json")).unwrap(), json_before);
    assert!(stray_parts(&sp.dir).is_empty());
}

#[test]
fn safety_two_saves_at_once_are_refused_and_other_tracks_stay_untouched() {
    let (_t, repo, sp, mut m, _b, ev) = ready_to_save();
    let other = fs::read(dir_of(&repo, SIX).join("track.json")).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    let second = m.start_save(FOUR, repo.clone(), no_lock());
    let _ = final_event(&ev);
    if let Err(e) = second {
        assert_eq!(e, "A save is already running");
    }
    assert_eq!(fs::read(dir_of(&repo, SIX).join("track.json")).unwrap(), other);
    assert!(!dir_of(&repo, SIX).join("backings").exists());
}

// ===================================================== no real audio device from tests

#[test]
fn nodevice_e2e_hooks_builds_never_choose_the_real_device() {
    for env in [None, Some(""), Some("cpal"), Some("capture:relative.wav"), Some("capture:/tmp/x.wav"), Some("null")] {
        assert_ne!(backend_kind(true, env), BackendKind::Cpal, "env {env:?}");
    }
    assert_eq!(backend_kind(true, Some("capture:/tmp/x.wav")), BackendKind::Capture("/tmp/x.wav".into()));
    assert_eq!(backend_kind(false, None), BackendKind::Cpal, "release builds use the device");
}

#[test]
fn nodevice_only_gui_rs_selects_a_backend_and_tests_never_do() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut users = Vec::new();
    for dir in ["src", "tests"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n == "node_modules" || n == "fixtures") {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let text = fs::read_to_string(&p).unwrap();
                    let name = p.file_name().unwrap().to_string_lossy().into_owned();
                    if text.contains("select_backend()") && name != "audio_out.rs" && name != "acceptance_backing_editor.rs" {
                        users.push(name);
                    }
                }
            }
        }
    }
    assert_eq!(users, ["gui.rs"], "only the app start-up may call select_backend");
}

#[test]
fn nodevice_null_backend_capture_plays_into_a_file_not_a_device() {
    let (tmp, repo) = fixture_repo();
    let wav = tmp.path().join("capture.wav");
    let backend = audio_out::NullBackend::new(Some(wav.clone()));
    let sink: EventSink = Arc::new(|_| {});
    let mut m = EditorManager::new(Box::new(backend), sink);
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    m.close();
    let bytes = fs::read(&wav).unwrap();
    assert!(bytes.starts_with(b"RIFF") && bytes.len() > 44);
}

#[test]
fn nodevice_play_on_a_failing_backend_is_a_message_not_a_panic() {
    struct Broken;
    impl audio_out::OutputBackend for Broken {
        fn name(&self) -> &'static str { "broken" }
        fn open(&self, _r: u32, _f: Box<dyn FnMut(&mut [f32]) + Send>) -> Result<Box<dyn audio_out::OutputHandle>, String> {
            Err("The audio output cannot be opened: no device".into())
        }
    }
    let (_t, repo) = fixture_repo();
    let mut m = EditorManager::new(Box::new(Broken), Arc::new(|_| {}));
    m.open(&spec(&repo, FOUR)).unwrap();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let e = m.play(FOUR).unwrap_err();
    assert!(e.contains("cannot be opened"), "{e}");
    assert!(!m.snapshot(FOUR).unwrap().transport.playing, "a failed Play must not show Pause");
}
