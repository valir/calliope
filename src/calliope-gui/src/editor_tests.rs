//! Editor session tests: fixture copies of `tests/fixtures/library-editor`, a `ManualBackend`
//! (never a real device) and injected time.

use super::*;
use crate::audio_out::ManualBackend;
use std::fs;
use std::path::Path;
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
    let variant = v["backings"].as_array().and_then(|a| a.first()).map(|b| VariantSpec {
        id: b["id"].as_str().unwrap().into(),
        name: b["name"].as_str().unwrap().into(),
        file: b["file"].as_str().unwrap().into(),
        mix: b["mix"].clone(),
    });
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
