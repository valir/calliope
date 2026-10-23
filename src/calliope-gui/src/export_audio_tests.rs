use super::*;
use crate::tempo_map::{SectionMeta, Segment};
use std::process::Command;
use std::time::Instant;

fn map(start_ms: f64, bpm: f64) -> TempoMap {
    TempoMap {
        beats_per_bar: 4,
        segments: vec![Segment { start_ms, bpm, detected_ms: None, on_beat: false, meta: SectionMeta::default() }],
    }
}

// ---- count_in_plan ----

#[test]
fn plan_worked_examples() {
    let p = count_in_plan(&map(500.0, 120.0), 44100).unwrap();
    assert_eq!((p.beat_frames, p.lead_frames), (22050, 88200));
    assert_eq!(p.ticks, [22050, 44100, 66150, 77175, 88200, 99225]);
    assert_eq!(p.first_beat_frame, 110250);
    assert_eq!(p.bpm, 120.0);
    let p = count_in_plan(&map(3000.0, 100.0), 44100).unwrap();
    assert_eq!((p.beat_frames, p.lead_frames), (26460, 105840));
    assert_eq!(p.ticks, [132300, 158760, 185220, 198450, 211680, 224910]);
    assert_eq!(p.first_beat_frame, 238140);
}

#[test]
fn plan_empty_map_and_first_beat_at_zero() {
    assert_eq!(count_in_plan(&TempoMap::default(), 44100), None);
    let p = count_in_plan(&map(0.0, 120.0), 44100).unwrap();
    assert_eq!(p.ticks, [0, 22050, 44100, 55125, 66150, 77175]);
    assert_eq!(p.first_beat_frame, p.lead_frames);
}

#[test]
fn plan_odd_beats_round_half_away_from_zero() {
    let bpm = (0..100)
        .map(|i| 120.0 + f64::from(i) / 10.0)
        .find(|b| ((44100.0f64 * 60.0 / b).round() as u64) % 2 == 1)
        .unwrap();
    let p = count_in_plan(&map(0.0, bpm), 44100).unwrap();
    let beat = (44100.0f64 * 60.0 / bpm).round() as u64;
    assert_eq!(p.beat_frames, beat);
    assert_eq!(p.lead_frames, 4 * beat);
    let half = (2.5 * beat as f64).round() as u64;
    // 2.5 * odd beat ends in .5 and goes up
    assert_eq!(beat % 2, 1, "the test needs an odd beat ({beat})");
    assert_eq!(p.ticks[3], (5 * beat).div_ceil(2));
    assert_eq!(p.ticks[3], half);
    assert_eq!(p.ticks[5], (7 * beat).div_ceil(2));
}

#[test]
fn plan_extreme_tempos_other_rates_and_meter() {
    let p = count_in_plan(&map(0.0, 300.0), 44100).unwrap();
    assert_eq!(p.beat_frames, 8820);
    let p = count_in_plan(&map(0.0, 20.0), 44100).unwrap();
    assert_eq!((p.beat_frames, p.lead_frames), (132300, 529200));
    let p = count_in_plan(&map(0.0, 100.0), 48000).unwrap();
    assert_eq!((p.beat_frames, p.lead_frames), (28800, 115200));
    let mut three = map(500.0, 120.0);
    three.beats_per_bar = 3;
    assert_eq!(count_in_plan(&three, 44100), count_in_plan(&map(500.0, 120.0), 44100));
}

#[test]
fn plan_later_first_beat_moves_the_ticks_only() {
    let a = count_in_plan(&map(500.0, 120.0), 44100).unwrap();
    let b = count_in_plan(&map(1500.0, 120.0), 44100).unwrap();
    assert_eq!(a.lead_frames, b.lead_frames);
    for (x, y) in a.ticks.iter().zip(b.ticks) {
        assert_eq!(y - x, 44100);
    }
}

// ---- mix_ticks ----

#[test]
fn mix_ticks_blocks_equal_one_block_and_leave_the_rest_alone() {
    let plan = count_in_plan(&map(0.0, 120.0), 22050).unwrap();
    let click = Click::new(22050).samples().to_vec();
    let total = 4 * 22050usize;
    let src: Vec<f32> = (0..total * 2).map(|i| ((i % 97) as f32 - 48.0) / 400.0).collect();
    let mut one = src.clone();
    let c1 = mix_ticks(&mut one, 2, 0, &plan, &click);
    // Uneven blocks, some boundaries inside a click.
    let mut many = src.clone();
    let mut pos = 0usize;
    let mut c2 = 0;
    for len in [1000usize, 7, 333, 11025, 5, 4096].iter().cycle() {
        if pos >= total {
            break;
        }
        let n = (*len).min(total - pos);
        c2 += mix_ticks(&mut many[pos * 2..(pos + n) * 2], 2, pos as u64, &plan, &click);
        pos += n;
    }
    assert_eq!(one, many);
    assert_eq!(c1, c2);
    // Bit-exact away from the ticks.
    let under = |f: usize| plan.ticks.iter().any(|&t| (f as u64) >= t && (f as u64) < t + click.len() as u64);
    for f in 0..total {
        if !under(f) {
            assert_eq!(one[f * 2], src[f * 2]);
            assert_eq!(one[f * 2 + 1], src[f * 2 + 1]);
        }
    }
    assert_ne!(one, src);
}

#[test]
fn mix_ticks_clamps_and_counts() {
    let plan = CountInPlan { bpm: 120.0, beat_frames: 10, lead_frames: 40, first_beat_frame: 40, ticks: [2, 100, 200, 300, 400, 500] };
    let click = vec![0.5f32; 4];
    let mut b = vec![0.9f32; 20]; // 10 mono frames
    let n = mix_ticks(&mut b, 1, 0, &plan, &click);
    assert_eq!(n, 4);
    assert_eq!(&b[..2], &[0.9, 0.9]);
    assert_eq!(&b[2..6], &[1.0, 1.0, 1.0, 1.0]);
    assert_eq!(b[6], 0.9);
    let mut neg = vec![-0.9f32; 10];
    let click = vec![-0.5f32; 4];
    assert_eq!(mix_ticks(&mut neg, 1, 0, &plan, &click), 4);
    assert_eq!(neg[3], -1.0);
}

// ---- args ----

#[test]
fn encoder_args_shape() {
    let tags = vec![("title", "T".to_string()), ("artist", "B".to_string())];
    let part = Path::new("/d/.x.part");
    for (kind, q) in [(ExportKind::Mp3, "2"), (ExportKind::Ogg, "6")] {
        let a: Vec<String> = encoder_args(kind, 48000, 2, &tags, part).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(a.contains(&"-y".to_string()));
        assert!(!a.contains(&"-n".to_string()));
        assert_eq!(a.last().unwrap(), "file:/d/.x.part");
        let pos = |x: &str| a.iter().position(|s| s == x).unwrap();
        assert_eq!(a[pos("-map_metadata") + 1], "-1");
        assert_eq!(a[pos("-q:a") + 1], q);
        assert_eq!(a[pos("-ar") + 1], "48000");
        assert!(a.contains(&"title=T".to_string()) && a.contains(&"artist=B".to_string()));
        assert!(a.contains(&"pipe:0".to_string()));
    }
    let d: Vec<String> = decoder_args(Path::new("/a b/x.flac"), 44100, 2).iter().map(|s| s.to_string_lossy().into_owned()).collect();
    assert!(d.contains(&"file:/a b/x.flac".to_string()) && d.contains(&"pipe:1".to_string()));
    assert!(!d.contains(&"-t".to_string()), "the decoder must not truncate the source");
}

// ---- helpers for the real ffmpeg ----

fn fixture(n: u32) -> (tempfile::TempDir, PathBuf) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-export/tracks");
    let id = format!("0199c0a0-0000-7000-8000-000000000{n}");
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join(&id);
    copy_dir(&from.join(&id), &to);
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

fn reference(src: &Path, rate: u32, ch: u32) -> Vec<f32> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-ac"])
        .arg(ch.to_string())
        .arg("-ar")
        .arg(rate.to_string())
        .arg("pipe:1")
        .output()
        .unwrap();
    assert!(out.status.success());
    out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn pipeline(src: &Path, rate: u32, ch: u32, plan: CountInPlan, part: &Path) -> Pipeline {
    Pipeline { ffmpeg: "ffmpeg".into(), source: src.to_path_buf(), rate, channels: ch, plan, part: part.to_path_buf(), pace: None }
}

fn no_progress() -> impl FnMut(u64) {
    |_| {}
}

/// Renders to a WAV in a temp dir and returns the samples (after checking the header).
fn render_wav(src: &Path, start_ms: f64, bpm: f64) -> (RunStats, Vec<f32>, CountInPlan) {
    let plan = count_in_plan(&map(start_ms, bpm), 44100).unwrap();
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("out.wav");
    let w = WavWriter::create(&part, 44100).unwrap();
    let p = pipeline(src, 44100, 2, plan.clone(), &part);
    let stats = run(&p, Output::Wav(w), &AtomicBool::new(false), &mut no_progress()).unwrap();
    let bytes = std::fs::read(&part).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[36..40], b"data");
    let n = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    assert_eq!(bytes.len(), 44 + n);
    let s = bytes[44..].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    (stats, s, plan)
}

fn goertzel(x: &[f32], hz: f32, rate: u32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * hz / rate as f32;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &v in x {
        let s = v + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / x.len().max(1) as f32
}

fn left(x: &[f32]) -> Vec<f32> {
    x.chunks_exact(2).map(|f| f[0]).collect()
}

/// `lead` zero frames ++ the reference decode, ticks mixed in at their frames.
fn expected(src: &Path, plan: &CountInPlan) -> Vec<f32> {
    let mut e = vec![0.0f32; plan.lead_frames as usize * 2];
    e.extend(reference(src, 44100, 2));
    let click = Click::new(44100).samples().to_vec();
    mix_ticks(&mut e, 2, 0, plan, &click);
    e
}

#[test]
fn wav_equals_lead_plus_original_with_ticks() {
    for (n, file, start, bpm) in [(401, "backings/backing.flac", 0.0, 120.0), (406, "backings/backing.flac", 500.0, 120.0)] {
        let (_d, dir) = fixture(n);
        let src = dir.join(file);
        let (stats, got, plan) = render_wav(&src, start, bpm);
        let want = expected(&src, &plan);
        assert_eq!(got.len(), want.len(), "{n}");
        assert!(got == want, "{n}: samples differ");
        assert_eq!(stats.frames, (got.len() / 2) as u64);
        assert_eq!(stats.frames, 88200 + 176400, "{n}");
        assert!(stats.clipped < 50);
    }
}

#[test]
fn long_intro_ticks_fall_in_silence() {
    let (_d, dir) = fixture(407);
    let src = dir.join("intro.flac");
    let (stats, got, plan) = render_wav(&src, 3000.0, 100.0);
    assert_eq!(plan.first_beat_frame, 238140);
    assert_eq!(stats.frames, 105840 + 264600);
    let click = Click::new(44100).samples().to_vec();
    let mut want = vec![0.0f32; 238140 * 2];
    mix_ticks(&mut want, 2, 0, &plan, &click);
    assert!(got[..238140 * 2] == want[..], "silence + exactly the six ticks until the first beat");
    for &t in &plan.ticks {
        let w = &left(&got)[t as usize..t as usize + click.len()];
        assert!(goertzel(w, 1500.0, 44100) > 0.05);
    }
    let after = &left(&got)[238140..238140 + 20000];
    assert!(goertzel(after, 440.0, 44100) > 10.0 * goertzel(after, 1000.0, 44100));
    let before = &left(&got)[238140 - 5000..238140];
    assert!(before.iter().all(|&v| v == 0.0));
}

#[test]
fn mono_becomes_stereo_and_48k_is_resampled() {
    let (_d, dir) = fixture(404);
    let (stats, got, _) = render_wav(&dir.join("backing.flac"), 0.0, 120.0);
    assert_eq!(stats.frames, 88200 + 176400);
    assert!(got.chunks_exact(2).all(|f| f[0] == f[1]));
    assert!(got.iter().any(|&v| v.abs() > 0.1));

    let (_d, dir) = fixture(405);
    let (stats, got, plan) = render_wav(&dir.join("backing.flac"), 0.0, 100.0);
    assert_eq!(plan.lead_frames, 105840);
    let want = 105840 + (4.0f64 * 44100.0).round() as u64;
    assert!(stats.frames.abs_diff(want) <= 1, "{} vs {want}", stats.frames);
    let l = left(&got);
    let after = &l[plan.lead_frames as usize + 20000..plan.lead_frames as usize + 40000];
    assert!(goertzel(after, 1000.0, 44100) > 5.0 * goertzel(after, 440.0, 44100));
    assert!(goertzel(after, 1000.0, 44100) > 0.02);
}

// ---- encoders ----

fn probe(path: &Path, entries: &str) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", entries, "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn encodes_mp3_and_ogg_with_tags() {
    for (n, start) in [(401u32, 0.0f64), (406, 500.0)] {
        for kind in [ExportKind::Mp3, ExportKind::Ogg] {
            let (_d, dir) = fixture(n);
            let src = dir.join("backings/backing.flac");
            let out = tempfile::tempdir().unwrap();
            let ext = if kind == ExportKind::Mp3 { "mp3" } else { "ogg" };
            let part = out.path().join(format!(".t.{ext}.part"));
            std::fs::File::create(&part).unwrap();
            let plan = count_in_plan(&map(start, 120.0), 44100).unwrap();
            let tags = vec![
                ("title", "Stem Mix".to_string()),
                ("artist", "The Example Band".to_string()),
                ("album", "Export Tests".to_string()),
                ("track", "3".to_string()),
                ("date", "2021".to_string()),
            ];
            let args = encoder_args(kind, 44100, 2, &tags, &part);
            let p = pipeline(&src, 44100, 2, plan, &part);
            let stats = run(&p, Output::Encode { ffmpeg: "ffmpeg".into(), args }, &AtomicBool::new(false), &mut no_progress())
                .unwrap_or_else(|e| panic!("{n} {kind:?}: {e}"));
            assert_eq!(stats.frames, 264600);
            let back = reference(&part, 44100, 2);
            let secs = (back.len() / 2) as f64 / 44100.0;
            assert!((secs - 6.0).abs() <= 0.03, "{n} {kind:?}: {secs}");
            let v = probe(&part, "format_tags:stream_tags");
            let mut tags = v["format"]["tags"].as_object().cloned().unwrap_or_default();
            tags.extend(v["streams"][0]["tags"].as_object().cloned().unwrap_or_default());
            let get = |k: &str| {
                tags.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str().unwrap().to_string())
            };
            assert_eq!(get("title").as_deref(), Some("Stem Mix"));
            assert_eq!(get("artist").as_deref(), Some("The Example Band"));
            assert_eq!(get("album").as_deref(), Some("Export Tests"));
            assert_eq!(get("track").or_else(|| get("tracknumber")).as_deref(), Some("3"), "{kind:?}");
            assert_eq!(get("date").as_deref(), Some("2021"));
        }
    }
}

// ---- fakes ----

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| s.contains(") Z"))
}

#[test]
fn cancel_returns_promptly_and_leaves_no_child() {
    let d = tempfile::tempdir().unwrap();
    let pidfile = d.path().join("pid");
    let argv = d.path().join("argv");
    let fake = script(
        d.path(),
        "ffmpeg",
        &format!("echo $$ > {}\nprintf '%s\\n' \"$@\" > {}\nexec sleep 60", pidfile.display(), argv.display()),
    );
    let plan = count_in_plan(&map(0.0, 120.0), 44100).unwrap();
    let part = d.path().join("out.wav");
    let w = WavWriter::create(&part, 44100).unwrap();
    let mut p = pipeline(Path::new("/x/song.flac"), 44100, 2, plan, &part);
    p.ffmpeg = fake;
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c2.store(true, Ordering::SeqCst);
    });
    let t0 = Instant::now();
    let e = run(&p, Output::Wav(w), &cancel, &mut no_progress()).unwrap_err();
    t.join().unwrap();
    assert_eq!(e, CANCELLED);
    assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
    let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    assert!(!alive(pid));
    let a = std::fs::read_to_string(&argv).unwrap();
    assert!(a.contains("file:/x/song.flac") && a.contains("pcm_f32le"), "{a}");
}

#[test]
fn encoder_failure_names_its_message_and_decoder_failure_too() {
    let d = tempfile::tempdir().unwrap();
    let (_f, dir) = fixture(401);
    let src = dir.join("backings/backing.flac");
    let enc = script(d.path(), "ffmpeg", "echo 'No space left on device' >&2\nexit 1");
    let part = d.path().join("out.mp3");
    let plan = count_in_plan(&map(0.0, 120.0), 44100).unwrap();
    let p = pipeline(&src, 44100, 2, plan.clone(), &part);
    let args = encoder_args(ExportKind::Mp3, 44100, 2, &[], &part);
    let e = run(&p, Output::Encode { ffmpeg: enc, args }, &AtomicBool::new(false), &mut no_progress()).unwrap_err();
    assert_eq!(e, "Cannot encode: No space left on device");

    // A decoder that fails.
    let bad = script(d.path(), "bad", "echo 'broken file' >&2\nexit 1");
    let mut p = pipeline(&src, 44100, 2, plan, &part);
    p.ffmpeg = bad;
    let w = WavWriter::create(&part, 44100).unwrap();
    let e = run(&p, Output::Wav(w), &AtomicBool::new(false), &mut no_progress()).unwrap_err();
    assert_eq!(e, "Cannot decode backing.flac: broken file");
}

#[test]
fn progress_is_reported_up_to_the_end() {
    let (_d, dir) = fixture(401);
    let src = dir.join("backings/backing.flac");
    let plan = count_in_plan(&map(0.0, 120.0), 44100).unwrap();
    let d = tempfile::tempdir().unwrap();
    let part = d.path().join("o.wav");
    let w = WavWriter::create(&part, 44100).unwrap();
    let p = pipeline(&src, 44100, 2, plan, &part);
    let mut last = 0;
    let mut calls = 0;
    run(&p, Output::Wav(w), &AtomicBool::new(false), &mut |f| {
        assert!(f >= last);
        last = f;
        calls += 1;
    })
    .unwrap();
    assert_eq!(last, 264600);
    assert!(calls > 10);
}
