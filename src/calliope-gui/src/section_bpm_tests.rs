//! Piece, hint, FLAC encoding and the job against the REAL `calliope-stems` on 127.0.0.1 (stub
//! beat tracker; `stub-tracker-last.flac` in its work dir is the piece that was really sent).
//! Never a real audio device or another host.

use super::*;
use crate::tempo_detect::{NO_STEADY, TOO_FEW};
use crate::test_server::TestServer;

fn map(bpms_at: &[(f64, f64)], beats_per_bar: u32) -> TempoMap {
    let segs: Vec<_> = bpms_at
        .iter()
        .map(|&(start_ms, bpm)| serde_json::json!({"start_ms": start_ms, "bpm": bpm}))
        .collect();
    serde_json::from_value(serde_json::json!({"beats_per_bar": beats_per_bar, "segments": segs}))
        .unwrap()
}

#[test]
fn piece_from_the_playhead_up_to_three_bars() {
    let m = map(&[(500.0, 100.0), (19000.0, 120.0)], 4);
    assert_eq!(piece(&m, 0, 10500.0, 60000.0), Ok((10500.0, 17700.0)));
    // stops at the next section
    assert_eq!(piece(&m, 0, 16000.0, 60000.0), Ok((16000.0, 19000.0)));
    // less than a bar, and exactly a bar
    assert_eq!(piece(&m, 0, 17500.0, 60000.0), Err(TOO_SHORT.to_string()));
    assert_eq!(piece(&m, 0, 16600.0, 60000.0), Ok((16600.0, 19000.0)));
    // the last section ends at the track end
    assert_eq!(piece(&m, 1, 30000.0, 33000.0), Ok((30000.0, 33000.0)));
    assert_eq!(piece(&m, 1, 32000.0, 33000.0), Err(TOO_SHORT.to_string()));
}

#[test]
fn piece_starts_at_the_section_when_the_playhead_is_elsewhere() {
    let m = map(&[(500.0, 100.0), (19000.0, 120.0)], 4);
    assert_eq!(piece(&m, 0, 200.0, 60000.0), Ok((500.0, 7700.0)));
    assert_eq!(piece(&m, 1, 5000.0, 60000.0), Ok((19000.0, 25000.0)));
    // the playhead past the section end is not inside it
    assert_eq!(piece(&m, 0, 25000.0, 60000.0), Ok((500.0, 7700.0)));
}

#[test]
fn piece_follows_the_bar_length() {
    let m = map(&[(0.0, 100.0)], 3);
    assert_eq!(piece(&m, 0, 1000.0, 60000.0), Ok((1000.0, 1000.0 + 5400.0)));
}

#[test]
fn hint_marks_octave_errors() {
    assert_eq!(hint(100.0, 200.0), Some(Hint::Double));
    assert_eq!(hint(100.0, 185.0), Some(Hint::Double));
    assert_eq!(hint(100.0, 183.0), None);
    assert_eq!(hint(100.0, 50.0), Some(Hint::Half));
    assert_eq!(hint(100.0, 46.5), Some(Hint::Half));
    assert_eq!(hint(100.0, 93.4), None);
}

/// Left 330 Hz, right a sample counter, so any offset error shows.
fn pcm(rate: u32, channels: u32, frames: u64) -> Pcm {
    let mut samples = Vec::new();
    for i in 0..frames {
        let t = i as f64 / rate as f64;
        samples.push(((t * 330.0 * std::f64::consts::TAU).sin() * 12000.0) as i16);
        for c in 1..channels {
            samples.push((i as i64 * 7 + c as i64) as i16);
        }
    }
    Pcm {
        rate,
        channels,
        frames,
        samples,
    }
}

fn decode(bytes: &[u8]) -> (claxon::metadata::StreamInfo, Vec<i32>) {
    let mut r = claxon::FlacReader::new(bytes).unwrap();
    let info = r.streaminfo();
    (info, r.samples().map(|s| s.unwrap()).collect())
}

#[test]
fn encode_piece_is_bit_exact() {
    for (rate, ch) in [(22050, 2), (22050, 1), (44100, 2), (44100, 1)] {
        let p = pcm(rate, ch, rate as u64 * 2);
        let (from, to) = (1234, rate as u64 + 777);
        let (info, got) = decode(&encode_piece(&p, from, to).unwrap());
        assert_eq!((info.sample_rate, info.channels), (rate, ch));
        assert_eq!(info.samples, Some(to - from));
        let want: Vec<i32> = p.samples[(from * ch as u64) as usize..(to * ch as u64) as usize]
            .iter()
            .map(|&s| s as i32)
            .collect();
        assert_eq!(got, want, "{rate} Hz, {ch} channels");
    }
}

#[test]
fn encode_piece_refuses_an_empty_piece() {
    let p = pcm(22050, 1, 1000);
    assert!(encode_piece(&p, 10, 10).is_err());
    assert!(encode_piece(&p, 10, 5000).is_err());
}

// ------------------------------------------------------------------ the job

const FROM: u64 = 22050;
const TO: u64 = 22050 * 4;

struct Run {
    outcome: BpmOutcome,
    events: Vec<PlayerEvent>,
    dir: tempfile::TempDir,
    pcm: Arc<Pcm>,
}

fn job(url: &str, dir: &Path, pcm: &Arc<Pcm>) -> BpmJob {
    let view = BpmDetectView {
        serial: 1,
        index: 0,
        at: 0.0,
        from_ms: 1000.0,
        to_ms: 4000.0,
        source: "backing:backing".into(),
        phase: DetectPhase::Checking,
    };
    BpmJob {
        id: "t1".into(),
        dir: dir.to_path_buf(),
        serial: 1,
        view,
        pcm: pcm.clone(),
        from_f: FROM,
        to_f: TO,
        beats_per_bar: 4,
        params: BpmParams {
            url: url.into(),
            poll: Duration::from_millis(50),
            stall: Duration::from_secs(10),
        },
    }
}

/// Runs a job to its end; `stop_at_upload` sets the cancel flag at the first `uploading` event.
fn run_job(url: &str, stop_at_upload: bool) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let pcm = Arc::new(pcm(22050, 2, 22050 * 6));
    let shared = BpmShared::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (ev, sh) = (events.clone(), shared.clone());
    let sink: EventSink = Arc::new(move |e| {
        if stop_at_upload {
            if let PlayerEvent::BpmDetect { detect, .. } = &e {
                if detect.phase == DetectPhase::Uploading {
                    sh.cancel.store(true, Ordering::SeqCst);
                }
            }
        }
        ev.lock().unwrap().push(e);
    });
    spawn(job(url, dir.path(), &pcm), shared.clone(), sink)
        .join()
        .unwrap();
    assert!(shared.is_finished());
    let events = events.lock().unwrap().clone();
    Run {
        outcome: shared.outcome().unwrap(),
        events,
        dir,
        pcm,
    }
}

fn no_part_files(dir: &Path) {
    let left: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".calliope-beats-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

fn failed(mode: &str) -> String {
    let server = TestServer::start_beats(mode, &[]);
    let r = run_job(&server.url, false);
    no_part_files(r.dir.path());
    match r.outcome {
        BpmOutcome::Failed(m) => m,
        o => panic!("{mode}: {o:?}"),
    }
}

#[test]
fn the_piece_is_sent_and_the_bpm_comes_back() {
    let server = TestServer::start_beats("bpm:93.4", &[]);
    let r = run_job(&server.url, false);
    assert_eq!(
        r.outcome,
        BpmOutcome::Done {
            bpm: 93.4,
            beats: 5
        }
    );
    no_part_files(r.dir.path());
    // the audio that was sent is the slice
    let sent = fs::read(server.outer.path().join("work/stub-tracker-last.flac")).unwrap();
    let (info, _) = decode(&sent);
    assert_eq!(info.samples, Some(TO - FROM));
    assert_eq!(sent, encode_piece(&r.pcm, FROM, TO).unwrap());
    // phases in order, ending with detecting
    let mut phases: Vec<DetectPhase> = Vec::new();
    for e in &r.events {
        let PlayerEvent::BpmDetect { id, detect } = e else {
            panic!("{e:?}");
        };
        assert_eq!((id.as_str(), detect.serial), ("t1", 1));
        if phases.last() != Some(&detect.phase) {
            phases.push(detect.phase);
        }
    }
    assert_eq!(phases[..2], [DetectPhase::Checking, DetectPhase::Uploading]);
    assert!(phases.iter().all(|p| matches!(
        p,
        DetectPhase::Checking
            | DetectPhase::Uploading
            | DetectPhase::Waiting
            | DetectPhase::Detecting
    )));
    assert!(phases.contains(&DetectPhase::Waiting) || phases.contains(&DetectPhase::Detecting));
}

#[test]
fn server_answers_become_exact_messages() {
    assert_eq!(failed("few"), TOO_FEW);
    assert_eq!(failed("irregular"), NO_STEADY);
    assert!(failed("fail").starts_with("Beat detection failed on the server: "));

    let r = run_job("http://127.0.0.1:1", false);
    assert_eq!(
        r.outcome,
        BpmOutcome::Failed("Cannot reach the edge-AI server at http://127.0.0.1:1".into())
    );
    no_part_files(r.dir.path());

    let server = TestServer::start("ok");
    let r = run_job(&server.url, false);
    assert_eq!(
        r.outcome,
        BpmOutcome::Failed(crate::beat_detect::NO_BEATS_TASK.into())
    );
}

#[test]
fn cancelling_after_the_upload_deletes_the_server_job_and_the_part() {
    let server = TestServer::start_beats("slow-bpm:120", &["--beat-timeout-ms", "20000"]);
    let r = run_job(&server.url, true);
    assert_eq!(r.outcome, BpmOutcome::Cancelled);
    no_part_files(r.dir.path());
    // no event after the cancel flag was set
    let last = r.events.last().unwrap();
    assert!(
        matches!(last, PlayerEvent::BpmDetect { detect, .. } if detect.phase == DetectPhase::Uploading)
    );
    let jobs = server.outer.path().join("work/jobs");
    let end = Instant::now() + Duration::from_secs(10);
    while fs::read_dir(&jobs).map(|d| d.count()).unwrap_or(0) > 0 {
        assert!(Instant::now() < end, "the server job is still there");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn piece_frames_round_and_clamp() {
    assert_eq!(
        piece_frames(22050, 1_000_000, 10500.0, 17700.0),
        (231_525, 390_285)
    );
    assert_eq!(piece_frames(44100, 100, 0.0, 5000.0), (0, 100));
}
