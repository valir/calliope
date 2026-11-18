//! Headless acceptance for gui-import-normalize: every import measures the converted mix and sends
//! (and keeps) the normalized one. The REAL `calliope-stems` runs on 127.0.0.1 with the stub
//! separator, which keeps a copy of every input in `work/stub-inputs/NNN.flac`: that is "the mix
//! sent". Audio is synthesised here (stereo 44.1 kHz FLAC through flacenc, so the ffmpeg
//! conversion is lossless); the URL is the fake yt-dlp; ffmpeg/ffprobe are the local tools.
//! Sources and a sibling "outside" folder must never change and no `*.part` may be left behind.
#![allow(dead_code, clippy::all)]

use calliope_gui::import_job::*;
use calliope_gui::loudness::{Meter, Outcome};
use calliope_gui::repository::Repository;
use calliope_gui::tools::{self, Tools};

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
const WATCH: &str = "https://media.example/watch?v=ok";

fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn tool(name: &str) -> PathBuf {
    tools::find_in_path(name, &std::env::var_os("PATH").unwrap_or_default())
        .unwrap_or_else(|| panic!("{name} is needed"))
}
fn stems_binary() -> PathBuf {
    let bin = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("calliope-stems");
    assert!(
        bin.is_file(),
        "{} is missing: cargo build -p calliope-stems",
        bin.display()
    );
    bin
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// path -> (size, mtime ns, content hash) of every file below `dir`.
fn fingerprint(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
            let md = fs::symlink_metadata(&p).unwrap();
            if md.is_dir() {
                out.insert(rel, "dir".into());
                walk(base, &p, out);
            } else {
                let bytes = fs::read(&p).unwrap_or_default();
                let mut h: u64 = 0xcbf29ce484222325;
                for b in &bytes {
                    h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
                }
                let mt = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                out.insert(rel, format!("{} {mt} {h:x}", bytes.len()));
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(dir, dir, &mut m);
    m
}

// ------------------------------------------------------------------ signals

fn write_flac(path: &Path, pcm: &[f32], bits: usize) {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let max = ((1i64 << (bits - 1)) - 1) as f64;
    let samples: Vec<i32> = pcm
        .iter()
        .map(|&v| (v as f64 * (max + 1.0)).round().clamp(-max - 1.0, max) as i32)
        .collect();
    let config = flacenc::config::Encoder::default().into_verified().unwrap();
    let src = flacenc::source::MemSource::from_samples(&samples, 2, bits, RATE as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, src, config.block_size).unwrap();
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).unwrap();
    fs::write(path, sink.as_slice()).unwrap();
}

/// (interleaved samples as stored, bits) of a FLAC file.
fn read_flac(path: &Path) -> (Vec<i32>, u32) {
    let mut r = claxon::FlacReader::open(path).unwrap();
    let bits = r.streaminfo().bits_per_sample;
    (r.samples().map(|s| s.unwrap()).collect(), bits)
}

fn measure_pcm(pcm: &[f32]) -> calliope_gui::loudness::Measure {
    let mut m = Meter::new(RATE).unwrap();
    m.add(pcm).unwrap();
    m.finish()
}

fn measure_file(path: &Path) -> calliope_gui::loudness::Measure {
    let (s, bits) = read_flac(path);
    let scale = (1u64 << (bits - 1)) as f32;
    measure_pcm(&s.iter().map(|&x| x as f32 / scale).collect::<Vec<_>>())
}

/// A stereo 1 kHz sine of `secs`, scaled so that it measures exactly `lufs` (16-bit rounding aside).
fn sine_at(secs: f64, lufs: f64) -> Vec<f32> {
    let frames = (secs * RATE as f64) as usize;
    let pcm: Vec<f32> = (0..frames)
        .flat_map(|n| {
            let v = (0.1 * (2.0 * PI * 1000.0 * n as f64 / RATE as f64).sin()) as f32;
            [v, v]
        })
        .collect();
    let g = 10f64.powf((lufs - measure_pcm(&pcm).lufs.unwrap()) / 20.0) as f32;
    pcm.into_iter().map(|v| v * g).collect()
}

/// 25 ms bursts of a 1 kHz sine at -3 dBFS with 10 ms raised-cosine fades, once per second for 6 s.
fn bursts() -> Vec<f32> {
    let (burst, fade) = (RATE as usize / 40, RATE as usize / 100);
    let peak = 10f64.powf(-3.0 / 20.0);
    let mut pcm = vec![0f32; 6 * RATE as usize * 2];
    for sec in 0..6usize {
        for n in 0..burst {
            let edge = n.min(burst - 1 - n);
            let env = if edge < fade {
                0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos()
            } else {
                1.0
            };
            let v = (peak * env * (2.0 * PI * 1000.0 * n as f64 / RATE as f64).sin()) as f32;
            let at = (sec * RATE as usize + n) * 2;
            pcm[at] = v;
            pcm[at + 1] = v;
        }
    }
    pcm
}

fn silence(secs: f64) -> Vec<f32> {
    vec![0.0; (secs * RATE as f64) as usize * 2]
}

fn flac_of(dir: &Path, name: &str, pcm: &[f32]) -> PathBuf {
    let p = dir.join(name);
    write_flac(&p, pcm, 16);
    p
}

// ------------------------------------------------------------------ server

struct RealServer {
    child: Child,
    url: String,
    inputs: PathBuf,
    _outer: tempfile::TempDir,
}

impl Drop for RealServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl RealServer {
    fn start() -> RealServer {
        let outer = tempfile::tempdir().unwrap();
        let work = outer.path().join("work");
        let inputs = work.join("stub-inputs");
        fs::create_dir_all(&inputs).unwrap();
        fs::write(work.join("stub-mode"), "ok\n").unwrap();
        let mut child = Command::new(stems_binary())
            .arg("--separator")
            .arg(project().join("tests/support/stub-separator"))
            .args(["--listen", "127.0.0.1:0", "--work-dir"])
            .arg(&work)
            .env_remove("STUB_SEPARATOR_MODE")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        let addr: SocketAddr = loop {
            line.clear();
            assert!(
                lines.read_line(&mut line).unwrap() > 0,
                "server exited before listening"
            );
            if let Some(rest) = line.split("listening addr=").nth(1) {
                break rest.split_whitespace().next().unwrap().parse().unwrap();
            }
        };
        assert!(addr.ip().is_loopback());
        std::thread::spawn(move || {
            let mut s = String::new();
            while lines.read_line(&mut s).unwrap_or(0) > 0 {
                s.clear();
            }
        });
        RealServer {
            child,
            url: format!("http://{addr}"),
            inputs,
            _outer: outer,
        }
    }

    /// The mixes sent so far, in order.
    fn sent(&self) -> Vec<PathBuf> {
        names_in(&self.inputs)
            .into_iter()
            .map(|n| self.inputs.join(n))
            .collect()
    }
}

// ------------------------------------------------------------------ rig

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    sources: PathBuf,
    tools: Tools,
    lock: Arc<dyn RepoLock>,
    state: ImportState,
    events: Arc<Mutex<Vec<ImportEvent>>>,
    sink: Sink,
    sources_before: BTreeMap<String, String>,
}

impl Rig {
    fn new() -> Rig {
        Rig::with(None)
    }

    fn paced(ms: u64) -> Rig {
        Rig::with(Some(Duration::from_millis(ms)))
    }

    fn with(pace: Option<Duration>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let outside = dir.path().join("outside");
        let sources = dir.path().join("sources");
        for d in [&outside, &sources] {
            fs::create_dir_all(d).unwrap();
        }
        Repository::new(&root).ensure_layout().unwrap();
        fs::write(outside.join("precious.txt"), "do not touch").unwrap();
        let tools = Tools {
            yt_dlp: Some(project().join("tests/support/bin/yt-dlp")),
            ffmpeg: Some(tool("ffmpeg")),
            ffprobe: Some(tool("ffprobe")),
        };
        let repo_lock = Arc::new(Mutex::new(()));
        let lock: Arc<dyn RepoLock> = Arc::new(move |f: &mut dyn FnMut()| {
            let _g = repo_lock.lock().unwrap_or_else(|e| e.into_inner());
            f()
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let e2 = events.clone();
        let sink: Sink = Arc::new(move |ev| e2.lock().unwrap().push(ev));
        let mut rig = Rig {
            _dir: dir,
            root,
            outside,
            sources,
            tools: tools.clone(),
            lock: lock.clone(),
            state: ImportState::new(tools, lock, Duration::from_millis(40)),
            events,
            sink,
            sources_before: BTreeMap::new(),
        };
        if let Some(p) = pace {
            rig.state = rig.new_state().with_normalize_pace(p);
        }
        rig
    }

    fn new_state(&self) -> ImportState {
        ImportState::new(
            self.tools.clone(),
            self.lock.clone(),
            Duration::from_millis(40),
        )
    }

    /// Call after the sources are written.
    fn seal(&mut self) {
        self.sources_before = fingerprint(&self.sources);
    }

    fn file(&self, path: &Path, kind: FileKind) -> JobSnapshot {
        self.state
            .start_file(&self.root, kind, path, self.sink.clone())
            .unwrap()
    }

    fn wait(&self, what: &str, pred: impl Fn(&JobSnapshot) -> bool) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(s) = self.state.snapshot() {
                assert!(
                    s.phase != Phase::Failed || pred(&s),
                    "failed while waiting for {what}: {:?}",
                    s.error
                );
                if pred(&s) {
                    return s;
                }
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn idle(&self, what: &str) -> JobSnapshot {
        self.wait(what, |_| !self.state.is_running())
    }
    fn ready(&self) -> JobSnapshot {
        let s = self.idle("ready");
        assert_eq!(s.phase, Phase::Ready, "{:?}", s.error);
        s
    }

    /// Extraction to the end; returns the saved snapshot.
    fn extract(&self, ready: &JobSnapshot, server: &RealServer, keep: bool) -> JobSnapshot {
        self.state
            .start_extraction(
                &ready.job,
                ready.metadata.clone().unwrap(),
                Some(&server.url),
                keep,
            )
            .unwrap();
        self.wait("saved", |s| s.phase == Phase::Saved)
    }

    fn normalizing_events(&self) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ImportEvent::Normalizing { .. }))
            .count()
    }

    fn job_dir(&self, prefix: &str) -> PathBuf {
        let dirs: Vec<String> = names_in(&self.root.join("import-tmp"))
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .collect();
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        self.root.join("import-tmp").join(&dirs[0])
    }

    fn tracks(&self) -> Vec<String> {
        names_in(&self.root.join("tracks"))
    }

    /// Sources and the outside folder are untouched; no `*.part` anywhere in `import-tmp`.
    fn check_clean(&self) {
        assert_eq!(
            fs::read_to_string(self.outside.join("precious.txt")).unwrap(),
            "do not touch"
        );
        assert_eq!(names_in(&self.outside), ["precious.txt"]);
        assert_eq!(fingerprint(&self.sources), self.sources_before);
        let parts: Vec<String> = fingerprint(&self.root.join("import-tmp"))
            .into_keys()
            .filter(|k| k.ends_with(".part"))
            .collect();
        assert!(parts.is_empty(), "{parts:?}");
    }
}

/// The mix sent measures -14 LUFS (within 0.1) with a true peak of at most -1 dBTP.
fn assert_at_target(mix: &Path) {
    let m = measure_file(mix);
    let l = m.lufs.unwrap();
    assert!((l + 14.0).abs() <= 0.1, "{l} LUFS");
    assert!(m.true_peak.unwrap() <= -0.95, "{:?}", m.true_peak);
}

fn video_with_sine(path: &Path, lufs: f64) {
    // a mono-in-stereo 1 kHz sine: LUFS = 20 log10 amplitude of the sine's channel
    let vol = format!("volume={}dB,pan=stereo|c0=c0|c1=c0", lufs + 18.0618);
    let out = Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=64x64:r=5:d=5"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=5:sample_rate=44100",
        ])
        .args(["-af", &vol, "-map", "0:v", "-map", "1:a"])
        .args(["-c:v", "mpeg4", "-c:a", "aac"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ------------------------------------------------------------------ tests

#[test]
fn ac1_quiet_file_is_sent_at_minus_14() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let src = flac_of(&rig.sources, "quiet.flac", &sine_at(6.0, -24.0));
    rig.seal();
    rig.file(&src, FileKind::Audio);
    let ready = rig.ready();
    let r = ready.loudness.clone().unwrap();
    assert_eq!(r.outcome, Outcome::Normalized);
    assert!((r.gain_db - 10.0).abs() < 0.1, "{}", r.gain_db);
    assert!((r.lufs.unwrap() + 14.0).abs() < 0.1);
    assert!(rig.normalizing_events() > 0);
    rig.extract(&ready, &server, false);
    let sent = server.sent();
    assert_eq!(sent.len(), 1);
    assert_at_target(&sent[0]);
    rig.check_clean();
}

#[test]
fn ac2_peaks_limit_the_gain() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let src = flac_of(&rig.sources, "bursts.flac", &bursts());
    rig.seal();
    rig.file(&src, FileKind::Audio);
    let ready = rig.ready();
    let r = ready.loudness.clone().unwrap();
    assert_eq!(r.outcome, Outcome::Limited);
    assert!(r.lufs.unwrap() < -14.1, "{:?}", r.lufs);
    assert!(r.true_peak.unwrap() <= -0.95, "{:?}", r.true_peak);
    rig.extract(&ready, &server, false);
    let m = measure_file(&server.sent()[0]);
    assert!(m.true_peak.unwrap() <= -0.95, "{:?}", m.true_peak);
    assert!(m.lufs.unwrap() < -14.1, "{:?}", m.lufs);
    rig.check_clean();
}

#[test]
fn ac3_a_file_at_the_target_is_sent_unchanged() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let pcm = sine_at(6.0, -14.0);
    let src = flac_of(&rig.sources, "target.flac", &pcm);
    rig.seal();
    rig.file(&src, FileKind::Audio);
    let ready = rig.ready();
    let r = ready.loudness.clone().unwrap();
    assert_eq!(r.outcome, Outcome::AtTarget);
    assert_eq!(r.gain_db, 0.0);
    rig.extract(&ready, &server, false);
    assert_eq!(read_flac(&server.sent()[0]), read_flac(&src));
    rig.check_clean();
}

#[test]
fn ac4_silence_and_short_files_are_imported_as_is() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let quiet = flac_of(&rig.sources, "silence.flac", &silence(6.0));
    let short = flac_of(
        &rig.sources,
        "short.flac",
        &sine_at(6.0, -20.0)[..(0.3 * RATE as f64) as usize * 2],
    );
    rig.seal();
    for (i, src) in [&quiet, &short].into_iter().enumerate() {
        rig.file(src, FileKind::Audio);
        let ready = rig.ready();
        assert_eq!(
            ready.loudness.clone().unwrap().outcome,
            Outcome::TooQuiet,
            "{src:?}"
        );
        rig.extract(&ready, &server, false);
        assert_eq!(server.sent().len(), i + 1);
        assert_eq!(read_flac(&server.sent()[i]), read_flac(src));
    }
    rig.check_clean();
}

#[test]
fn ac5_the_kept_original_is_the_mix_sent() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let src = flac_of(&rig.sources, "quiet.flac", &sine_at(6.0, -22.0));
    rig.seal();
    rig.file(&src, FileKind::Audio);
    let ready = rig.ready();
    let saved = rig.extract(&ready, &server, true);
    let id = saved.track.unwrap().id;
    let original = rig.root.join("tracks").join(&id).join("original.flac");
    assert_at_target(&original);
    assert_eq!(
        fs::read(&original).unwrap(),
        fs::read(&server.sent()[0]).unwrap()
    );
    rig.check_clean();
}

#[test]
fn ac6_url_video_and_directory_are_normalized_first() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    let video = rig.sources.join("clip.mp4");
    video_with_sine(&video, -24.0);
    let dir = rig.sources.join("album");
    fs::create_dir_all(&dir).unwrap();
    let files = vec![
        flac_of(&dir, "a-quiet.flac", &sine_at(6.0, -24.0)),
        flac_of(&dir, "b-target.flac", &sine_at(6.0, -14.0)),
        flac_of(&dir, "c-silence.flac", &silence(6.0)),
    ];
    rig.seal();

    // video
    rig.file(&video, FileKind::Video);
    let ready = rig.ready();
    assert_eq!(ready.loudness.clone().unwrap().outcome, Outcome::Normalized);
    rig.extract(&ready, &server, false);
    assert_at_target(&server.sent()[0]);

    // URL (the fake yt-dlp's file): the mix sent is the job's normalized copy
    rig.state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .unwrap();
    let ready = rig.ready();
    assert!(ready.loudness.is_some());
    let jdir = rig.job_dir("url-");
    let mix = fs::read(if jdir.join("normalized.flac").is_file() {
        jdir.join("normalized.flac")
    } else {
        jdir.join("audio.flac")
    })
    .unwrap();
    let phases = rig.phase_names();
    let at = |p: &str| phases.iter().rposition(|x| x == p).unwrap();
    assert!(at("preparing") < at("normalizing") && at("normalizing") < at("ready"));
    rig.extract(&ready, &server, false);
    assert_eq!(mix, fs::read(&server.sent()[1]).unwrap());

    // directory: each file passes Normalizing before Uploading; the summary has the results
    rig.state
        .start_batch(
            &rig.root,
            &dir,
            files.clone(),
            Some(&server.url),
            false,
            rig.sink.clone(),
        )
        .unwrap();
    let done = rig.idle("the batch");
    assert_eq!(done.phase, Phase::Finished);
    let items = done.batch.unwrap().items;
    let outcomes: Vec<Outcome> = items
        .iter()
        .map(|i| match &i.outcome {
            BatchOutcome::Saved { loudness, .. } => loudness.outcome,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        outcomes,
        [Outcome::Normalized, Outcome::AtTarget, Outcome::TooQuiet]
    );
    let sent = server.sent();
    assert_eq!(sent.len(), 5);
    assert_at_target(&sent[2]);
    assert_eq!(read_flac(&sent[3]), read_flac(&files[1]));
    assert_eq!(read_flac(&sent[4]), read_flac(&files[2]));
    let phases = rig.phase_names();
    let mut seen_norm = 0;
    let mut uploads = 0;
    for p in phases.iter().skip_while(|p| *p != "batch-file") {
        match p.as_str() {
            "batch-file" => seen_norm = 0,
            "normalizing" => seen_norm += 1,
            "uploading" => {
                assert!(seen_norm > 0, "uploading before normalizing");
                uploads += 1;
            }
            _ => {}
        }
    }
    assert_eq!(uploads, 3);
    rig.check_clean();
}

#[test]
fn ac7_cancel_while_normalizing_leaves_nothing() {
    let server = RealServer::start();
    // a local file
    let mut rig = Rig::paced(50);
    let src = flac_of(&rig.sources, "quiet.flac", &sine_at(6.0, -24.0));
    rig.seal();
    let snap = rig.file(&src, FileKind::Audio);
    rig.wait("normalizing", |s| s.phase == Phase::Normalizing);
    rig.state.cancel(&snap.job).unwrap();
    assert_eq!(rig.idle("cancel").phase, Phase::Cancelled);
    assert!(names_in(&rig.root.join("import-tmp")).is_empty());
    assert!(rig.tracks().is_empty());
    rig.check_clean();

    // a URL: the download and audio.flac stay for a resume, nothing of the step does
    let mut rig = Rig::paced(50);
    rig.seal();
    let snap = rig
        .state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .unwrap();
    rig.wait("normalizing", |s| s.phase == Phase::Normalizing);
    rig.state.cancel(&snap.job).unwrap();
    assert_eq!(rig.idle("cancel").phase, Phase::Cancelled);
    let names = names_in(&rig.job_dir("url-"));
    assert!(names.contains(&"audio.flac".to_string()), "{names:?}");
    for gone in [".normalized.flac.part", "normalized.flac", "normalize.json"] {
        assert!(!names.contains(&gone.to_string()), "{names:?}");
    }
    assert!(rig.tracks().is_empty());
    assert!(server.sent().is_empty());
    rig.check_clean();
}

#[test]
fn ac8_a_resumed_import_is_not_normalized_again() {
    let server = RealServer::start();
    let mut rig = Rig::new();
    rig.seal();
    rig.state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .unwrap();
    let first = rig.ready();
    let dir = rig.job_dir("url-");
    let stamp = |name: &str| {
        let p = dir.join(name);
        (
            fs::read(&p).unwrap(),
            fs::metadata(&p).unwrap().modified().unwrap(),
        )
    };
    let before = (stamp("normalized.flac"), stamp("normalize.json"));

    // the app is closed in the edit pane and started again on the same repository
    let state = rig.new_state();
    let events: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let e2 = events.clone();
    state
        .start_url(
            &rig.root,
            WATCH,
            true,
            Arc::new(move |e| e2.lock().unwrap().push(e)),
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    while state.is_running() {
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let snap = state.snapshot().unwrap();
    assert_eq!(snap.phase, Phase::Ready, "{:?}", snap.error);
    assert!(!events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, ImportEvent::Normalizing { .. })));
    assert_eq!(snap.loudness, first.loudness);
    assert_eq!(before, (stamp("normalized.flac"), stamp("normalize.json")));

    state
        .start_extraction(
            &snap.job,
            snap.metadata.clone().unwrap(),
            Some(&server.url),
            false,
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    while state.snapshot().unwrap().phase != Phase::Saved {
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read(server.sent().last().unwrap()).unwrap(),
        before.0 .0
    );
    rig.check_clean();
}

impl Rig {
    /// The phase names of every event so far (`serde` tag), in order.
    fn phase_names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["phase"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }
}
