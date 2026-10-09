//! Editor session tests: fixture copies of `tests/fixtures/library-editor`, a `ManualBackend`
//! (never a real device) and injected time.

use super::*;
use crate::audio_out::ManualBackend;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
const MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
const MISSING: &str = "0199c0a0-0000-7000-8000-000000000205";
const RATE: usize = 22050;

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

fn spec_of(id: &str) -> (tempfile::TempDir, OpenSpec) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor/tracks").join(id);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(id);
    copy_tree(&src, &dir);
    let v: Value = serde_json::from_slice(&fs::read(dir.join("track.json")).unwrap()).unwrap();
    let stems: Vec<StemEntry> = v["stems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| StemEntry { name: s["name"].as_str().unwrap().into(), file: s["file"].as_str().unwrap().into() })
        .collect();
    let missing = stems.iter().filter(|s| !dir.join(&s.file).is_file()).map(|s| s.file.clone()).collect();
    let variant = v["backings"]
        .as_array()
        .and_then(|a| a.first())
        .map(|b| serde_json::from_value::<BackingVariant>(b.clone()).unwrap());
    let spec = OpenSpec { id: id.into(), title: v["title"].as_str().unwrap().into(), dir, stems, missing, variant };
    (tmp, spec)
}

type Events = Arc<Mutex<Vec<EditorEvent>>>;

fn manager() -> (EditorManager, ManualBackend, Events) {
    let backend = ManualBackend::new();
    let events: Events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let sink: EventSink = Arc::new(move |e| ev.lock().unwrap().push(e));
    (EditorManager::new(Box::new(backend.clone()), sink), backend, events)
}

fn transports(events: &Events) -> Vec<TransportState> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            EditorEvent::Transport { state, .. } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

/// Decoded 16-bit stems of a spec, as stereo f32 per frame range.
fn decoded(spec: &OpenSpec, name: &str) -> StemPcm {
    let file = &spec.stems.iter().find(|s| s.name == name).unwrap().file;
    stem_audio::decode_i16(&spec.dir.join(file), &AtomicBool::new(false), &mut |_, _| {}).unwrap()
}

fn stereo(pcm: &StemPcm, start: usize, frames: usize) -> Vec<f32> {
    let mut v = vec![0.0; frames * 2];
    mixer::to_stereo_f32(pcm, start as u64, frames, &mut v);
    v
}

fn expected(parts: &[(&StemPcm, f32)], start: usize, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    for (pcm, gain) in parts {
        for (o, s) in out.iter_mut().zip(stereo(pcm, start, frames)) {
            *o += s * gain;
        }
    }
    out.iter().map(|x| x.clamp(-1.0, 1.0)).collect()
}

fn assert_close(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!((x - y).abs() < 1e-5, "sample {i}: {x} vs {y}");
    }
}

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

fn open_four() -> (tempfile::TempDir, OpenSpec, EditorManager, ManualBackend, Events) {
    let (tmp, spec) = spec_of(FOUR);
    let (mut m, b, ev) = manager();
    m.open(&spec).unwrap();
    (tmp, spec, m, b, ev)
}

#[test]
fn open_four_lanes() {
    let (_t, spec, mut m, _b, ev) = open_four();
    let snap = m.snapshot(FOUR).unwrap();
    assert_eq!(snap.stems.len(), 4);
    assert_eq!(snap.duration_ms, 12000);
    assert_eq!(snap.sample_rate, 22050);
    assert!(snap.stems.iter().all(|l| !l.unmuted && l.gain_db == Some(0.0)));
    assert_eq!(snap.variant, VariantTarget { id: "backing".into(), name: "Backing".into(), file: "backings/backing.flac".into(), exists: false });
    assert_eq!(m.play(FOUR).unwrap_err(), "Check a stem first");
    assert_eq!(m.play("other").unwrap_err(), "no editor session for other");
    let loading: Vec<usize> = ev
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| if let EditorEvent::Loading { done, total: 4, .. } = e { Some(*done) } else { None })
        .collect();
    assert_eq!(loading, vec![0, 1, 2, 3, 4]);
    assert_eq!(spec.stems.len(), 4);
}

#[test]
fn open_mixed_before_restores_variant_mix() {
    let (_t, spec) = spec_of(MIXED);
    let (mut m, _b, _e) = manager();
    let snap = m.open(&spec).unwrap();
    assert_eq!(
        snap.stems,
        vec![
            LaneState { name: "vocals".into(), gain_db: Some(-6.0), unmuted: true },
            LaneState { name: "guitar".into(), gain_db: Some(0.0), unmuted: false },
        ]
    );
    assert_eq!(snap.variant.id, "backing");
    assert!(snap.variant.exists);
}

#[test]
fn open_missing_stem_names_the_file() {
    let (_t, spec) = spec_of(MISSING);
    let (mut m, _b, _e) = manager();
    let err = m.open(&spec).unwrap_err();
    assert!(err.contains("stems/drums.flac"), "{err}");
    assert!(m.current().is_none());
}

#[test]
fn mix_parsing_is_lenient() {
    let names = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let mix = serde_json::json!({"stems": [
        {"name": "a", "gain_db": 13.2, "unmuted": true},
        {"name": "b", "gain_db": null, "unmuted": true},
        {"name": "zzz", "gain_db": 5, "unmuted": true},
    ]});
    let lanes = parse_mix(&mix, &names);
    assert_eq!(lanes[0].gain_db, Some(12.0));
    assert_eq!(lanes[1], LaneState { name: "b".into(), gain_db: None, unmuted: true });
    assert_eq!(lanes[2], LaneState { name: "c".into(), gain_db: Some(0.0), unmuted: false });
    assert_eq!(parse_mix(&serde_json::json!("junk"), &names)[0].gain_db, Some(0.0));
}

#[test]
fn playback_equals_expected_mix_and_gain_change_applies() {
    let (_t, spec, mut m, b, _e) = open_four();
    let (voc, bass) = (decoded(&spec, "vocals"), decoded(&spec, "bass"));
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.set_stem(FOUR, "bass", Some(-6.0), true).unwrap();
    assert!(!b.is_open());
    assert!(m.play(FOUR).unwrap().playing);
    assert!(b.is_open());
    let g = mixer::gain_factor(Some(-6.0), true);
    // 1300 frames crosses two chunk boundaries
    let out = b.pull(1300);
    assert_close(&out, &expected(&[(&voc, 1.0), (&bass, g)], 0, 1300));
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    let out = b.pull(100);
    assert_close(&out, &expected(&[(&voc, 1.0), (&bass, 1.0)], 1300, 100));
    assert_eq!(m.snapshot(FOUR).unwrap().transport.position_ms, 1400 * 1000 / RATE as u64);
}

#[test]
fn set_stem_snaps_gain() {
    let (_t, _s, mut m, _b, _e) = open_four();
    assert_eq!(m.set_stem(FOUR, "drums", Some(-0.3), true).unwrap().gain_db, Some(-0.5));
    assert_eq!(m.set_stem(FOUR, "drums", Some(40.0), true).unwrap().gain_db, Some(12.0));
    assert_eq!(m.set_stem(FOUR, "drums", Some(-80.0), true).unwrap().gain_db, None);
    assert_eq!(m.set_stem(FOUR, "drums", None, true).unwrap().gain_db, None);
    assert!(m.set_stem(FOUR, "nope", Some(0.0), true).is_err());
}

#[test]
fn solo_rules() {
    let (_t, spec, mut m, b, _e) = open_four();
    let (gui, voc) = (decoded(&spec, "guitar"), decoded(&spec, "vocals"));
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let st = m.lane_play(FOUR, "guitar").unwrap();
    assert!(st.playing);
    assert_eq!(st.solo.as_deref(), Some("guitar"));
    let snap = m.snapshot(FOUR).unwrap();
    let un: Vec<bool> = snap.stems.iter().map(|l| l.unmuted).collect();
    assert_eq!(un, vec![true, false, false, true]); // vocals untouched, guitar checked
    assert_close(&b.pull(600), &expected(&[(&gui, 1.0)], 0, 600));

    // moves to another stem
    let st = m.lane_play(FOUR, "vocals").unwrap();
    assert_eq!(st.solo.as_deref(), Some("vocals"));
    assert_close(&b.pull(100), &expected(&[(&voc, 1.0)], 600, 100));

    // pause keeps it
    assert_eq!(m.pause(FOUR).unwrap().solo.as_deref(), Some("vocals"));
    assert!(peak(&b.pull(100)) == 0.0);

    // lane play on the soloed stem ends it (and does not start playback while paused)
    let st = m.lane_play(FOUR, "vocals").unwrap();
    assert_eq!(st.solo, None);
    assert!(!st.playing);

    // end_solo
    m.lane_play(FOUR, "guitar").unwrap();
    assert_eq!(m.end_solo(FOUR).unwrap().solo, None);
    // mix play
    m.lane_play(FOUR, "guitar").unwrap();
    assert_eq!(m.play(FOUR).unwrap().solo, None);
    // stop
    m.lane_play(FOUR, "guitar").unwrap();
    let st = m.stop(FOUR).unwrap();
    assert_eq!((st.solo, st.playing, st.position_ms), (None, false, 0));
    assert!(!b.is_open());
    // unchecking the soloed stem
    m.lane_play(FOUR, "guitar").unwrap();
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    assert_eq!(m.snapshot(FOUR).unwrap().transport.solo.as_deref(), Some("guitar"));
    m.set_stem(FOUR, "guitar", Some(0.0), false).unwrap();
    assert_eq!(m.snapshot(FOUR).unwrap().transport.solo, None);
}

#[test]
fn solo_gain_is_the_lanes_own() {
    let (_t, spec, mut m, b, _e) = open_four();
    let gui = decoded(&spec, "guitar");
    m.set_stem(FOUR, "guitar", Some(-6.0), true).unwrap();
    m.lane_play(FOUR, "guitar").unwrap();
    let g = mixer::gain_factor(Some(-6.0), true);
    assert_close(&b.pull(300), &expected(&[(&gui, g)], 0, 300));
}

#[test]
fn clipping_flag_with_one_second_hold() {
    let (_t, _s, mut m, b, ev) = open_four();
    for n in ["vocals", "drums", "bass"] {
        m.set_stem(FOUR, n, Some(12.0), true).unwrap();
    }
    m.play(FOUR).unwrap();
    let t0 = Instant::now();
    let out = b.pull(5000);
    assert_eq!(peak(&out), 1.0);
    m.tick(t0 + Duration::from_millis(20));
    assert!(transports(&ev).last().unwrap().clipping);
    assert!(m.snapshot(FOUR).unwrap().transport.clipping);
    m.tick(t0 + Duration::from_millis(1019));
    assert!(transports(&ev).last().unwrap().clipping);
    m.tick(t0 + Duration::from_millis(1020));
    assert!(!transports(&ev).last().unwrap().clipping);
}

#[test]
fn no_clipping_flag_for_a_clean_mix() {
    let (_t, _s, mut m, b, _e) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.pull(2000);
    m.tick(Instant::now());
    assert!(!m.snapshot(FOUR).unwrap().transport.clipping);
}

#[test]
fn pause_is_silent_and_holds_position() {
    let (_t, _s, mut m, b, _e) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    assert!(peak(&b.pull(1000)) > 0.0);
    let st = m.pause(FOUR).unwrap();
    assert!(!st.playing);
    let pos = st.position_ms;
    assert_eq!(peak(&b.pull(1000)), 0.0);
    assert_eq!(m.snapshot(FOUR).unwrap().transport.position_ms, pos);
    assert!(b.is_open(), "output stays open while paused");
    // play continues from the same place
    m.play(FOUR).unwrap();
    assert!(peak(&b.pull(100)) > 0.0);
}

#[test]
fn seek_while_playing_silences_then_resumes() {
    let (_t, spec, mut m, b, _e) = open_four();
    let voc = decoded(&spec, "vocals");
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.pull(100);
    let t0 = Instant::now();
    let st = m.seek(FOUR, 5000, t0).unwrap();
    assert!(st.playing && st.resume_pending);
    assert_eq!(st.position_ms, 5000);
    assert_eq!(peak(&b.pull(200)), 0.0);
    m.tick(t0 + Duration::from_millis(299));
    assert_eq!(peak(&b.pull(200)), 0.0);
    m.tick(t0 + Duration::from_millis(300));
    assert!(!m.snapshot(FOUR).unwrap().transport.resume_pending);
    let start = 5000 * RATE / 1000;
    assert_close(&b.pull(300), &expected(&[(&voc, 1.0)], start, 300));
}

#[test]
fn seek_while_paused_just_moves() {
    let (_t, _s, mut m, _b, ev) = open_four();
    let st = m.seek(FOUR, 2500, Instant::now()).unwrap();
    assert_eq!((st.playing, st.resume_pending, st.position_ms), (false, false, 2500));
    assert_eq!(transports(&ev).last().unwrap().position_ms, 2500);
    // beyond the end is clamped
    assert_eq!(m.seek(FOUR, 99_000, Instant::now()).unwrap().position_ms, 12000);
}

#[test]
fn nudge_is_clamped() {
    let (_t, _s, mut m, _b, _e) = open_four();
    let now = Instant::now();
    assert_eq!(m.nudge(FOUR, -100, now).unwrap().position_ms, 0);
    assert_eq!(m.nudge(FOUR, 100, now).unwrap().position_ms, 100);
    assert_eq!(m.nudge(FOUR, 100, now).unwrap().position_ms, 200);
    m.seek(FOUR, 12000, now).unwrap();
    assert_eq!(m.nudge(FOUR, 100, now).unwrap().position_ms, 12000);
    assert_eq!(m.nudge(FOUR, -100, now).unwrap().position_ms, 11900);
}

#[test]
fn a_seek_in_the_callback_window_wins() {
    // the callback advances with compare_exchange, so a seek stored meanwhile is kept
    let (_t, _s, mut m, _b, _e) = open_four();
    let shared = m.session.as_ref().unwrap().shared.clone();
    shared.position.store(100, Ordering::SeqCst);
    assert!(shared.position.compare_exchange(50, 150, Ordering::SeqCst, Ordering::SeqCst).is_err());
    assert_eq!(shared.position.load(Ordering::SeqCst), 100);
    let _ = &mut m;
}

#[test]
fn end_of_track_acts_as_stop() {
    let (_t, _s, mut m, b, ev) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let now = Instant::now();
    m.play(FOUR).unwrap();
    m.seek(FOUR, 11_900, now).unwrap();
    m.tick(now + Duration::from_millis(300));
    let out = b.pull(5000); // 100 ms left, then silence
    assert!(peak(&out[..4000]) > 0.0);
    assert_eq!(peak(&out[4420..]), 0.0);
    m.tick(now + Duration::from_millis(320));
    let last = transports(&ev).last().unwrap().clone();
    assert!(!last.playing);
    assert_eq!(last.position_ms, 0);
    assert!(!b.is_open());
    // can play again from the start
    assert!(m.play(FOUR).unwrap().playing);
}

#[test]
fn transport_events_are_throttled_while_playing() {
    let (_t, _s, mut m, b, ev) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    let t0 = Instant::now();
    m.play(FOUR).unwrap();
    let before = transports(&ev).len();
    for i in 1..=50u64 {
        b.pull(441); // 20 ms
        m.tick(t0 + Duration::from_millis(20 * i));
    }
    let n = transports(&ev).len() - before;
    assert!((6..=10).contains(&n), "{n} events in one second");
    // once the position stops moving, ticks emit nothing
    m.tick(t0 + Duration::from_secs(5));
    let c = transports(&ev).len();
    m.tick(t0 + Duration::from_secs(6));
    assert_eq!(transports(&ev).len(), c);
}

#[test]
fn device_error_stops_playback() {
    let (_t, _s, mut m, b, ev) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    b.fail("device lost");
    m.tick(Instant::now());
    assert!(ev.lock().unwrap().iter().any(|e| matches!(e, EditorEvent::AudioError { message, .. } if message == "device lost")));
    assert!(!m.snapshot(FOUR).unwrap().transport.playing);
    assert!(!b.is_open());
}

#[test]
fn second_open_supersedes_a_load_in_progress() {
    let (_t, spec) = spec_of(FOUR);
    let (mut m, _b, _e) = manager();
    let t1 = m.begin_open();
    let cancel = t1.cancel.clone();
    // the load is cancelled by a new open after the first stem
    let sink = move |e: EditorEvent| {
        if matches!(e, EditorEvent::Loading { done: 1, .. }) {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    assert_eq!(load(&spec, &t1, &sink).err().as_deref(), Some(SUPERSEDED));

    // a finished load whose ticket was superseded is refused
    let t2 = m.begin_open();
    let loaded = load(&spec, &t2, &|_| {}).unwrap();
    let t3 = m.begin_open();
    assert_eq!(m.finish_open(&t2, &spec, loaded).err().as_deref(), Some(SUPERSEDED));
    assert!(m.current().is_none());
    let loaded = load(&spec, &t3, &|_| {}).unwrap();
    assert!(m.finish_open(&t3, &spec, loaded).is_ok());
    // close cancels a pending load too
    let t4 = m.begin_open();
    m.close();
    assert_eq!(load(&spec, &t4, &|_| {}).err().as_deref(), Some(SUPERSEDED));
}

#[test]
fn close_then_open_restores_the_remembered_mix() {
    let (_t, spec, mut m, b, _e) = open_four();
    m.set_stem(FOUR, "drums", Some(-12.5), true).unwrap();
    m.set_stem(FOUR, "bass", None, true).unwrap();
    m.play(FOUR).unwrap();
    m.close();
    assert!(!b.is_open());
    assert!(m.current().is_none());
    let snap = m.open(&spec).unwrap();
    assert_eq!(snap.stems[1], LaneState { name: "drums".into(), gain_db: Some(-12.5), unmuted: true });
    assert_eq!(snap.stems[2], LaneState { name: "bass".into(), gain_db: None, unmuted: true });
    assert!(!snap.stems[0].unmuted);
    assert!(!snap.transport.playing);
    assert_eq!(snap.transport.position_ms, 0);
}

#[test]
fn opening_another_track_closes_the_first() {
    let (_t, spec, mut m, b, _e) = open_four();
    m.set_stem(FOUR, "vocals", Some(0.0), true).unwrap();
    m.play(FOUR).unwrap();
    let (_t2, spec2) = spec_of(MIXED);
    m.open(&spec2).unwrap();
    assert!(!b.is_open());
    assert!(m.snapshot(FOUR).is_err());
    assert!(m.snapshot(MIXED).is_ok());
    let _ = spec;
}

// ------------------------------------------------------------------ save job

use crate::import_job::RepoLock;
use crate::repository::Repository;
use std::time::Instant as Clock;

/// A repository with the fixture track copied to `tracks/<id>`.
fn repo_spec(id: &str) -> (tempfile::TempDir, Repository, OpenSpec) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor/tracks").join(id);
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repository::new(tmp.path().join("repo"));
    repo.ensure_layout().unwrap();
    let dir = tmp.path().join("repo/tracks").join(id);
    copy_tree(&src, &dir);
    let (_t, mut spec) = spec_of(id);
    spec.dir = dir;
    (tmp, repo, spec)
}

fn no_lock() -> Arc<dyn RepoLock> {
    Arc::new(|f: &mut dyn FnMut()| f())
}

fn events_only() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

fn final_event(events: &Events) -> EditorEvent {
    let end = Clock::now() + Duration::from_secs(60);
    loop {
        let found = events.lock().unwrap().iter().find(|e| {
            matches!(e, EditorEvent::Saved { .. } | EditorEvent::SaveFailed { .. } | EditorEvent::SaveCancelled { .. })
        }).cloned();
        if let Some(e) = found {
            return e;
        }
        assert!(Clock::now() < end, "the save did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn parts_in(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".calliope-backing-"))
        .collect()
}

fn read_flac(path: &Path) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

#[test]
fn save_ignores_solo_and_replaces_the_variant() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let (mut m, _b, ev) = manager();
    m.open(&spec).unwrap();
    m.set_stem(FOUR, "vocals", Some(-6.0), true).unwrap();
    m.set_stem(FOUR, "drums", Some(0.0), true).unwrap();
    m.lane_play(FOUR, "vocals").unwrap(); // solo on vocals; drums stays checked
    assert_eq!(m.snapshot(FOUR).unwrap().transport.solo.as_deref(), Some("vocals"));
    let snap = m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(snap.saving.is_some());
    assert_eq!(m.start_save(FOUR, repo.clone(), no_lock()).unwrap_err(), "A save is already running");
    let EditorEvent::Saved { file, clipped_samples, .. } = final_event(&ev) else { panic!("{:?}", ev.lock().unwrap()) };
    assert_eq!(file, "backings/backing.flac");

    // exactly vocals(-6 dB) + drums(0 dB), as 16 bits
    let (v, d) = (decoded(&spec, "vocals"), decoded(&spec, "drums"));
    let (gv, gd) = (mixer::gain_factor(Some(-6.0), true), mixer::gain_factor(Some(0.0), true));
    let frames = v.frames.max(d.frames) as usize;
    let (sv, sd) = (stereo(&v, 0, frames), stereo(&d, 0, frames));
    let mut clipped = 0;
    let want: Vec<i32> = (0..frames * 2)
        .map(|i| {
            let x = sv[i] * gv + sd[i] * gd;
            clipped += u64::from(x.abs() > 1.0);
            mixer::quantise(x.clamp(-1.0, 1.0), 16)
        })
        .collect();
    let (info, got) = read_flac(&spec.dir.join(&file));
    assert_eq!((info.sample_rate, info.bits_per_sample, info.channels), (22050, 16, 2));
    assert!(got == want, "saved mix differs");
    assert_eq!(clipped_samples, clipped);
    assert!(parts_in(&spec.dir).is_empty());

    // the session now points at the saved variant and the save is over
    m.tick(Clock::now());
    let snap = m.snapshot(FOUR).unwrap();
    assert_eq!(snap.saving, None);
    assert_eq!(snap.variant, VariantTarget { id: "backing".into(), name: "Backing".into(), file, exists: true });
    let meta: Value = serde_json::from_slice(&fs::read(spec.dir.join("track.json")).unwrap()).unwrap();
    assert_eq!(meta["backings"][0]["mix"]["stems"][0], serde_json::json!({"name": "vocals", "gain_db": -6.0, "unmuted": true}));
    assert_eq!(meta["backings"][0]["mix"]["stems"][2]["unmuted"], false);

    // a second save replaces the same variant (no backing-2) and trashes the old file
    ev.lock().unwrap().clear();
    m.start_save(FOUR, repo, no_lock()).unwrap();
    let EditorEvent::Saved { file, .. } = final_event(&ev) else { panic!() };
    assert_eq!(file, "backings/backing.flac");
    let meta: Value = serde_json::from_slice(&fs::read(spec.dir.join("track.json")).unwrap()).unwrap();
    assert_eq!(meta["backings"].as_array().unwrap().len(), 1);
}

#[test]
fn save_refuses_without_a_checked_stem() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let (mut m, _b, ev) = manager();
    m.open(&spec).unwrap();
    assert_eq!(m.start_save(FOUR, repo.clone(), no_lock()).unwrap_err(), "Check a stem first");
    assert_eq!(m.start_save("other", repo, no_lock()).unwrap_err(), "no editor session for other");
    assert!(ev.lock().unwrap().iter().all(|e| !matches!(e, EditorEvent::Saving { .. })));
    assert!(parts_in(&spec.dir).is_empty());
}

#[test]
fn save_conflict_fails_and_leaves_no_part() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let (mut m, _b, ev) = manager();
    m.open(&spec).unwrap();
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    // the stems change on disk after the session opened
    let path = spec.dir.join("track.json");
    let text = fs::read_to_string(&path).unwrap().replace("\"name\": \"guitar\"", "\"name\": \"lead\"");
    fs::write(&path, &text).unwrap();
    m.start_save(FOUR, repo, no_lock()).unwrap();
    let EditorEvent::SaveFailed { message, .. } = final_event(&ev) else { panic!() };
    assert!(message.starts_with("conflict:"), "{message}");
    assert!(parts_in(&spec.dir).is_empty());
    assert!(!spec.dir.join("backings").exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    // the failed job is over: another save may start
    m.tick(Clock::now());
    assert_eq!(m.snapshot(FOUR).unwrap().saving, None);
}

#[test]
fn save_cancel_leaves_track_json_unchanged() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let ev = events_only();
    let path = spec.dir.join("track.json");
    let before = fs::read(&path).unwrap();
    // cancel from the first progress event, i.e. in the middle of the render
    let ev2 = ev.clone();
    let shared = crate::backing_render::SaveShared::new();
    let sh = shared.clone();
    let sink: EventSink = Arc::new(move |e| {
        if matches!(e, EditorEvent::Saving { .. }) {
            sh.cancel.store(true, Ordering::Relaxed);
        }
        ev2.lock().unwrap().push(e);
    });
    let params = crate::backing_render::SaveParams {
        id: FOUR.into(),
        variant: "backing".into(),
        dir: spec.dir.clone(),
        stems: spec.stems.clone(),
        input: crate::backing_render::RenderInput {
            stems: vec![crate::backing_render::RenderStem { path: spec.dir.join(&spec.stems[2].file), gain: 1.0 }],
            sample_rate: 22050,
            bits: 16,
            frames: 12 * 22050,
        },
        mix: serde_json::json!({}),
        repo,
        lock: no_lock(),
    };
    crate::backing_render::spawn(params, shared, sink).join().unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::SaveCancelled { .. }));
    assert!(parts_in(&spec.dir).is_empty());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!spec.dir.join("backings").exists());
}

#[test]
fn cancel_through_the_manager_finishes_the_job() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let (mut m, _b, ev) = manager();
    m.open(&spec).unwrap();
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo, no_lock()).unwrap();
    m.cancel_save(FOUR).unwrap();
    // either the cancel won or the (fast) job had already saved; never a leftover part
    let _ = final_event(&ev);
    assert!(parts_in(&spec.dir).is_empty());
    m.shutdown_save(Duration::from_secs(2));
}

#[test]
fn save_progress_is_monotonic_and_ends_at_one() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let (mut m, _b, ev) = manager();
    m.open(&spec).unwrap();
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo, no_lock()).unwrap();
    assert!(matches!(final_event(&ev), EditorEvent::Saved { .. }));
    let p: Vec<f32> = ev
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| if let EditorEvent::Saving { progress, .. } = e { Some(*progress) } else { None })
        .collect();
    assert!(!p.is_empty() && p.windows(2).all(|w| w[0] <= w[1]), "{p:?}");
    assert_eq!(p.last(), Some(&1.0));
}

#[test]
fn a_panic_in_the_save_thread_fails_the_save_and_frees_the_slot() {
    let (_t, repo, spec) = repo_spec(FOUR);
    let events = events_only();
    let (ev, armed) = (events.clone(), Arc::new(AtomicBool::new(true)));
    let sink: EventSink = Arc::new(move |e| {
        if matches!(e, EditorEvent::Saving { .. }) && armed.swap(false, Ordering::SeqCst) {
            panic!("simulated bug in the save thread");
        }
        ev.lock().unwrap().push(e);
    });
    let mut m = EditorManager::new(Box::new(ManualBackend::new()), sink);
    m.open(&spec).unwrap();
    m.set_stem(FOUR, "bass", Some(0.0), true).unwrap();
    m.start_save(FOUR, repo.clone(), no_lock()).unwrap();
    assert!(matches!(final_event(&events), EditorEvent::SaveFailed { .. }));
    assert!(parts_in(&spec.dir).is_empty());
    // the job counts as finished: another save can start
    m.start_save(FOUR, repo, no_lock()).unwrap();
    m.shutdown_save(Duration::from_secs(30));
}
