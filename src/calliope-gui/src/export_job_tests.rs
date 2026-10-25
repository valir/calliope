use super::*;
use crate::export_rules::export_source;
use crate::metronome::Click;
use crate::repository::Repository;
use crate::tempo_map::{SectionMeta, Segment};
use crate::tempo_store::{FileTempoStore, TempoStore};
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::AtomicUsize;

type Events = Arc<Mutex<Vec<ExportEvent>>>;

fn map(start_ms: f64, bpm: f64) -> TempoMap {
    TempoMap {
        beats_per_bar: 4,
        segments: vec![Segment {
            start_ms,
            bpm,
            detected_ms: None,
            on_beat: false,
            meta: SectionMeta::default(),
            found_ms: None,
        }],
    }
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

/// Every file (bytes) and folder under `dir`, by relative path.
fn snapshot(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let rel = e.path().strip_prefix(root).unwrap().display().to_string();
            if e.file_type().unwrap().is_dir() {
                out.insert(rel, None);
                walk(root, &e.path(), out);
            } else {
                out.insert(rel, Some(std::fs::read(e.path()).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn real_tools() -> Tools {
    Tools::discover(&std::env::var_os("PATH").unwrap_or_default())
}

struct Rig {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    dest: PathBuf,
    before: BTreeMap<String, Option<Vec<u8>>>,
    state: ExportState,
    finished: Arc<Mutex<Vec<Finished>>>,
}

impl Rig {
    fn new() -> Self {
        Self::with(ExportState::new(Box::new(real_tools)))
    }

    fn with(state: ExportState) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-export"),
            &repo,
        );
        let dest = tmp.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        let before = snapshot(&repo);
        Rig {
            _tmp: tmp,
            repo,
            dest,
            before,
            state,
            finished: Arc::default(),
        }
    }

    fn id(n: u32) -> String {
        format!("0199c0a0-0000-7000-8000-000000000{n}")
    }

    /// A base with a `ROLAND` folder in a fresh sub folder.
    fn rc5(&self, name: &str) -> PathBuf {
        let b = self.dest.join(name);
        std::fs::create_dir_all(b.join("ROLAND")).unwrap();
        b
    }

    fn plan(
        &self,
        n: u32,
        kind: ExportKind,
        memory: Option<u8>,
        chosen: &Path,
    ) -> Result<ExportPlan, String> {
        let r = Repository::new(&self.repo);
        let t = r.load_record(&Self::id(n)).unwrap();
        let dir = r.tracks_dir().join(&t.id);
        let src = dir.join(export_source(&t).unwrap().file);
        let tempo = FileTempoStore.load(&t.id, &dir).unwrap();
        self.state.plan(PlanInput {
            track: &t,
            track_dir: dir.clone(),
            source: src,
            kind,
            memory,
            chosen,
            repo_root: &self.repo,
            tempo: &tempo,
        })
    }

    fn tempo(&self, n: u32) -> TempoMap {
        let dir = Repository::new(&self.repo).tracks_dir().join(Self::id(n));
        FileTempoStore.load("t", &dir).unwrap()
    }

    fn start(
        &self,
        token: &str,
        overwrite: bool,
        tempo: &TempoMap,
    ) -> (Result<ExportSnapshot, String>, Events, Arc<AtomicUsize>) {
        let events: Events = Arc::default();
        let e2 = events.clone();
        let done = Arc::new(AtomicUsize::new(0));
        let d2 = done.clone();
        let fin = self.finished.clone();
        let r = self.state.start(
            token,
            overwrite,
            tempo,
            Arc::new(move |ev| e2.lock().unwrap().push(ev)),
            Box::new(move |f| {
                d2.fetch_add(1, Ordering::SeqCst);
                fin.lock().unwrap().push(f.clone());
                Recorded {
                    root: "the-root".into(),
                    records: Vec::new(),
                    info_error: Some("info trouble".into()),
                }
            }),
        );
        (r, events, done)
    }

    fn run(
        &self,
        n: u32,
        kind: ExportKind,
        memory: Option<u8>,
        base: &Path,
    ) -> (ExportSnapshot, Events, Arc<AtomicUsize>) {
        let plan = self.plan(n, kind, memory, base).unwrap();
        let (r, ev, done) = self.start(&plan.token, plan.exists, &self.tempo(n));
        r.unwrap();
        (wait_terminal(&self.state), ev, done)
    }

    fn assert_repo_untouched(&self) {
        assert!(
            snapshot(&self.repo) == self.before,
            "the repository changed"
        );
    }
}

fn wait_terminal(state: &ExportState) -> ExportSnapshot {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let s = state.snapshot().unwrap();
        if !matches!(s.phase, ExportPhase::Writing | ExportPhase::Finishing) {
            return s;
        }
        assert!(Instant::now() < end, "export did not finish: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_event(ev: &Events) {
    let end = Instant::now() + Duration::from_secs(20);
    while ev.lock().unwrap().is_empty() {
        assert!(Instant::now() < end, "no event");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn parts(dir: &Path) -> Vec<String> {
    snapshot(dir)
        .into_keys()
        .filter(|k| k.ends_with(".part"))
        .collect()
}

fn wav_samples(p: &Path) -> (Vec<u8>, Vec<f32>) {
    let b = std::fs::read(p).unwrap();
    let s = b[44..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    (b[..44].to_vec(), s)
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .map(|l| l.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false)
}

fn tags_of(p: &Path) -> BTreeMap<String, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format_tags:stream_tags",
            "-of",
            "json",
        ])
        .arg(p)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut m = BTreeMap::new();
    for t in [&v["format"]["tags"], &v["streams"][0]["tags"]] {
        for (k, val) in t.as_object().cloned().unwrap_or_default() {
            m.insert(k.to_lowercase(), val.as_str().unwrap().to_string());
        }
    }
    m
}

#[test]
fn rc5_writes_the_memory_file() {
    let rig = Rig::new();
    let base = rig.rc5("rc5");
    let (s, ev, done) = rig.run(401, ExportKind::Rc5, Some(7), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let target = base.join("ROLAND/WAVE/007_1/007_1.WAV");
    assert_eq!(s.path, target.display().to_string());
    let (head, samples) = wav_samples(&target);
    assert_eq!(&head[..4], b"RIFF");
    assert_eq!(&head[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([head[20], head[21]]), 3);
    assert_eq!(u16::from_le_bytes([head[22], head[23]]), 2);
    assert_eq!(u32::from_le_bytes(head[24..28].try_into().unwrap()), 44100);
    assert_eq!(&head[36..40], b"data");
    assert_eq!(samples.len() as u64, (110250 + 176400) * 2);
    assert_eq!(std::fs::metadata(&target).unwrap().len(), 44 + 286650 * 8);
    assert!(base.join("ROLAND/WAVE").is_dir() && base.join("ROLAND/WAVE/007_1").is_dir());
    assert!(parts(&base).is_empty());
    assert_eq!(done.load(Ordering::SeqCst), 1);
    let ev = ev.lock().unwrap();
    assert!(
        matches!(ev.first(), Some(ExportEvent::Writing { .. })),
        "{ev:?}"
    );
    assert!(ev.contains(&ExportEvent::Finishing), "{ev:?}");
    // The hook got the target, kind and memory; what it returned is in the event and snapshot.
    assert_eq!(
        *rig.finished.lock().unwrap(),
        vec![Finished {
            id: Rig::id(401),
            kind: ExportKind::Rc5,
            memory: Some(7),
            path: target.clone()
        }]
    );
    assert!(
        matches!(ev.last(), Some(ExportEvent::Done { root, records, info_error, .. })
            if root == "the-root" && records.is_empty() && info_error.as_deref() == Some("info trouble")),
        "{ev:?}"
    );
    assert_eq!(s.info_error.as_deref(), Some("info trouble"));
    assert_eq!(s.progress, 1.0);
    rig.assert_repo_untouched();
}

#[test]
fn set_first_beat_export_is_shorter_and_progress_reaches_one() {
    let rig = Rig::new();
    let base = rig.rc5("rc5");
    let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
    let mut tempo = rig.tempo(401);
    tempo.segments[0].start_ms = 500.0;
    tempo.segments[0].found_ms = Some(500.0);
    let (r, _ev, _done) = rig.start(&plan.token, plan.exists, &tempo);
    r.unwrap();
    let s = wait_terminal(&rig.state);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert_eq!(s.progress, 1.0);
    let (_, samples) = wav_samples(&base.join("ROLAND/WAVE/007_1/007_1.WAV"));
    assert_eq!(samples.len() as u64, (110250 + 176400 - 22050) * 2);
}

#[test]
fn rc5_refusals_write_nothing() {
    let rig = Rig::new();
    let check = |base: &Path, want: String| {
        let before = snapshot(base);
        let e = rig.plan(401, ExportKind::Rc5, Some(7), base).unwrap_err();
        assert_eq!(e, want);
        assert!(snapshot(base) == before);
    };
    let plain = rig.dest.join("plain");
    std::fs::create_dir(&plain).unwrap();
    check(
        &plain,
        format!(
            "No ROLAND folder in {}: is the RC-5 mounted?",
            plain.display()
        ),
    );
    let gone = rig.dest.join("gone");
    assert_eq!(
        rig.plan(401, ExportKind::Rc5, Some(7), &gone).unwrap_err(),
        format!("{} is not an existing folder", gone.display())
    );
    assert!(!gone.exists());
    let wave_file = rig.rc5("wavefile");
    std::fs::write(wave_file.join("ROLAND/WAVE"), b"x").unwrap();
    check(
        &wave_file,
        format!(
            "{} is not a folder",
            wave_file.join("ROLAND/WAVE").display()
        ),
    );
    let folder = rig.rc5("folder");
    std::fs::create_dir_all(folder.join("ROLAND/WAVE/007_1/007_1.WAV")).unwrap();
    check(
        &folder,
        format!(
            "{} is a folder",
            folder.join("ROLAND/WAVE/007_1/007_1.WAV").display()
        ),
    );
    let inside = rig.repo.join("inside");
    std::fs::create_dir_all(inside.join("ROLAND")).unwrap();
    check(&inside, "Cannot export into the repository folder".into());
    std::fs::remove_dir_all(&inside).unwrap();
    rig.assert_repo_untouched();
}

#[test]
fn overwrite_rules() {
    let rig = Rig::new();
    let base = rig.rc5("rc5");
    let dir = base.join("ROLAND/WAVE/007_1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("007_1.WAV"), b"old").unwrap();
    std::fs::write(dir.join("notes.txt"), b"keep me").unwrap();
    let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
    assert!(plan.exists);
    let (r, _, _) = rig.start(&plan.token, false, &rig.tempo(401));
    assert!(r.unwrap_err().ends_with("007_1.WAV already exists"));
    assert_eq!(std::fs::read(dir.join("007_1.WAV")).unwrap(), b"old");
    assert!(parts(&base).is_empty());
    let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
    let (r, _, _) = rig.start(&plan.token, true, &rig.tempo(401));
    r.unwrap();
    assert_eq!(wait_terminal(&rig.state).phase, ExportPhase::Done);
    assert_eq!(
        std::fs::metadata(dir.join("007_1.WAV")).unwrap().len(),
        44 + 286650 * 8
    );
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"keep me");
    assert!(parts(&base).is_empty());
    rig.assert_repo_untouched();
}

#[test]
fn mp3_and_ogg_names_and_tags() {
    let rig = Rig::new();
    let (s, _, _) = rig.run(401, ExportKind::Mp3, None, &rig.dest);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let mp3 = rig.dest.join("The Example Band - Stem Mix.mp3");
    assert!(mp3.is_file());
    let t = tags_of(&mp3);
    assert_eq!(t["title"], "Stem Mix");
    assert_eq!(t["track"], "3");
    assert_eq!(t["date"], "2021");
    let (s, _, _) = rig.run(409, ExportKind::Ogg, None, &rig.dest);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let ogg = rig.dest.join("AC_DC_ - Live_ A_B_.ogg");
    assert!(ogg.is_file());
    let t = tags_of(&ogg);
    assert_eq!(t["title"], "Live: A/B?");
    assert_eq!(t["artist"], "AC/DC*");
    assert!(
        !t.contains_key("track") && !t.contains_key("tracknumber"),
        "{t:?}"
    );
    assert!(parts(&rig.dest).is_empty());
    rig.assert_repo_untouched();
}

#[test]
fn cancel_leaves_nothing() {
    let rig = Rig {
        state: ExportState::new(Box::new(real_tools)).with_pace(5.0),
        ..Rig::new()
    };
    // No WAVE yet: both folders are ours and go again.
    let base = rig.rc5("fresh");
    let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
    let (r, ev, done) = rig.start(&plan.token, false, &rig.tempo(401));
    let job = r.unwrap().job;
    wait_event(&ev);
    assert!(base.join("ROLAND/WAVE/007_1").is_dir());
    assert_eq!(rig.state.cancel(&job).unwrap().job, job);
    let s = wait_terminal(&rig.state);
    assert_eq!(s.phase, ExportPhase::Cancelled);
    assert!(!base.join("ROLAND/WAVE").exists());
    assert!(base.join("ROLAND").is_dir());
    assert!(ev.lock().unwrap().contains(&ExportEvent::Cancelled));
    assert_eq!(done.load(Ordering::SeqCst), 0);
    assert!(rig.finished.lock().unwrap().is_empty());
    // An existing WAVE stays.
    let base = rig.rc5("kept");
    std::fs::create_dir_all(base.join("ROLAND/WAVE")).unwrap();
    let plan = rig.plan(401, ExportKind::Rc5, Some(8), &base).unwrap();
    let (r, ev, _) = rig.start(&plan.token, false, &rig.tempo(401));
    let job = r.unwrap().job;
    wait_event(&ev);
    rig.state.cancel(&job);
    assert_eq!(wait_terminal(&rig.state).phase, ExportPhase::Cancelled);
    assert!(base.join("ROLAND/WAVE").is_dir());
    assert!(!base.join("ROLAND/WAVE/008_1").exists());
    assert!(parts(&base).is_empty());
    // Cancelling a finished or unknown job changes nothing.
    assert!(rig.state.cancel("nope").is_none());
    assert_eq!(
        rig.state.cancel(&job).unwrap().phase,
        ExportPhase::Cancelled
    );
    rig.assert_repo_untouched();
}

#[test]
fn failure_leaves_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::new();
    if !is_root() {
        let base = rig.rc5("ro");
        let roland = base.join("ROLAND");
        std::fs::set_permissions(&roland, std::fs::Permissions::from_mode(0o555)).unwrap();
        let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
        let (r, _, _) = rig.start(&plan.token, false, &rig.tempo(401));
        std::fs::set_permissions(&roland, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = r.unwrap_err();
        assert!(
            e.starts_with("Cannot write to ") && e.contains("Permission denied"),
            "{e}"
        );
        assert!(!roland.join("WAVE").exists());
        // An MP3 folder that cannot be written.
        let ro = rig.dest.join("ro-mp3");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let plan = rig.plan(401, ExportKind::Mp3, None, &ro).unwrap();
        let (r, _, _) = rig.start(&plan.token, false, &rig.tempo(401));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(r.unwrap_err().contains("Permission denied"));
        assert!(snapshot(&ro).is_empty());
    }
    // An encoder that dies half way.
    let fake = rig.dest.join("ffmpeg-fake");
    let real = real_tools().ffmpeg.unwrap();
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in *pipe:0*) head -c 200000 >/dev/null; echo 'disk is full' >&2; exit 1;; esac\nexec {} \"$@\"\n",
            real.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake2 = fake.clone();
    let state = ExportState::new(Box::new(move || Tools {
        ffmpeg: Some(fake2.clone()),
        ..real_tools()
    }));
    let rig2 = Rig {
        state,
        ..Rig::new()
    };
    let out = rig2.dest.join("out");
    std::fs::create_dir(&out).unwrap();
    let (s, ev, done) = rig2.run(401, ExportKind::Mp3, None, &out);
    assert_eq!(s.phase, ExportPhase::Failed);
    assert_eq!(s.message.as_deref(), Some("Cannot encode: disk is full"));
    assert!(
        matches!(ev.lock().unwrap().last(), Some(ExportEvent::Failed { message }) if message.contains("disk is full"))
    );
    assert_eq!(done.load(Ordering::SeqCst), 0);
    assert!(snapshot(&out).is_empty());
    rig.assert_repo_untouched();
    rig2.assert_repo_untouched();
}

#[test]
fn one_at_a_time_and_stale_tokens() {
    let rig = Rig {
        state: ExportState::new(Box::new(real_tools)).with_pace(5.0),
        ..Rig::new()
    };
    let a = rig.rc5("a");
    let b = rig.rc5("b");
    assert_eq!(
        rig.start("x99", false, &rig.tempo(401)).0.unwrap_err(),
        STALE_MESSAGE
    );
    let plan = rig.plan(401, ExportKind::Rc5, Some(1), &a).unwrap();
    let (r, ev, _) = rig.start(&plan.token, false, &rig.tempo(401));
    let job = r.unwrap().job;
    wait_event(&ev);
    assert_eq!(
        rig.plan(401, ExportKind::Rc5, Some(2), &b).unwrap_err(),
        BUSY_MESSAGE
    );
    // The token is used up.
    assert_eq!(
        rig.start(&plan.token, false, &rig.tempo(401))
            .0
            .unwrap_err(),
        STALE_MESSAGE
    );
    rig.state.cancel(&job);
    wait_terminal(&rig.state);
    // A newer plan replaces an older one.
    let p1 = rig.plan(401, ExportKind::Rc5, Some(1), &a).unwrap();
    let p2 = rig.plan(401, ExportKind::Rc5, Some(2), &a).unwrap();
    assert_eq!(
        rig.start(&p1.token, false, &rig.tempo(401)).0.unwrap_err(),
        STALE_MESSAGE
    );
    let _ = p2;
    rig.assert_repo_untouched();
}

#[test]
fn no_tempo_is_refused_and_the_map_at_start_is_used() {
    let rig = Rig::new();
    let base = rig.rc5("rc5");
    assert_eq!(
        rig.plan(410, ExportKind::Rc5, Some(7), &base).unwrap_err(),
        NO_TEMPO_REASON
    );
    assert!(snapshot(&base).len() == 1);
    let plan = rig.plan(406, ExportKind::Rc5, Some(7), &base).unwrap();
    let before = snapshot(&base);
    let (r, _, _) = rig.start(&plan.token, false, &TempoMap::default());
    assert_eq!(r.unwrap_err(), NO_TEMPO_REASON);
    assert!(snapshot(&base) == before);
    // The first beat moved since the plan: 500 ms -> 1500 ms.
    let plan = rig.plan(406, ExportKind::Rc5, Some(7), &base).unwrap();
    let (r, _, _) = rig.start(&plan.token, false, &map(1500.0, 120.0));
    r.unwrap();
    assert_eq!(wait_terminal(&rig.state).phase, ExportPhase::Done);
    let (_, s) = wav_samples(&base.join("ROLAND/WAVE/007_1/007_1.WAV"));
    let click = Click::new(44100).samples().to_vec();
    let left: Vec<f32> = s.chunks_exact(2).map(|f| f[0]).collect();
    // lead = 110250 frames of silence; the ticks start at 66150 (1500 ms) and not at 22050.
    assert_eq!(&left[66150..66150 + click.len()], &click[..]);
    assert!(left[22050..22050 + click.len()].iter().all(|&v| v == 0.0));
    assert_eq!(s.len(), 286650 * 2);
    rig.assert_repo_untouched();
}

#[test]
fn shutdown_mid_export() {
    let rig = Rig {
        state: ExportState::new(Box::new(real_tools)).with_pace(5.0),
        ..Rig::new()
    };
    assert!(
        rig.state.shutdown(Duration::from_millis(10)),
        "nothing running"
    );
    let base = rig.rc5("rc5");
    let plan = rig.plan(401, ExportKind::Rc5, Some(7), &base).unwrap();
    let (r, ev, _) = rig.start(&plan.token, false, &rig.tempo(401));
    r.unwrap();
    wait_event(&ev);
    assert!(rig.state.shutdown(Duration::from_secs(2)));
    assert!(parts(&base).is_empty());
    assert!(!base.join("ROLAND/WAVE").exists());
    assert_eq!(rig.state.snapshot().unwrap().phase, ExportPhase::Cancelled);
    rig.assert_repo_untouched();
}

#[test]
fn require_checks_tools_and_encoders() {
    let none = ExportState::new(Box::new(Tools::default));
    assert_eq!(none.require(ExportKind::Rc5).unwrap_err(), TOOLS_MESSAGE);
    let rig = Rig::new();
    assert!(rig.state.require(ExportKind::Mp3).is_ok());
    assert!(rig.state.require(ExportKind::Ogg).is_ok());
    // A fake ffmpeg without encoders.
    use std::os::unix::fs::PermissionsExt;
    let fake = rig.dest.join("ffmpeg");
    std::fs::write(&fake, "#!/bin/sh\necho ' A..... aac AAC'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let f2 = fake.clone();
    let st = ExportState::new(Box::new(move || Tools {
        ffmpeg: Some(f2.clone()),
        ..real_tools()
    }));
    assert!(st.require(ExportKind::Rc5).is_ok());
    assert_eq!(
        st.require(ExportKind::Mp3).unwrap_err(),
        "Your ffmpeg has no MP3 encoder (libmp3lame): install an ffmpeg build that includes it"
    );
    assert!(st
        .require(ExportKind::Ogg)
        .unwrap_err()
        .contains("Ogg Vorbis encoder (libvorbis)"));
}
