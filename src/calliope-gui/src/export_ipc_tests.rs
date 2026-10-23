use super::*;
use crate::export_job::{ExportPhase, ExportSnapshot};
use crate::metronome::Click;
use crate::picker::ScriptedPicker;
use crate::tempo_store::FileTempoStore;
use crate::tools::Tools;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Rig {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    st: RepoState,
    picker: Arc<ScriptedPicker>,
    answers: PathBuf,
    settings: SettingsStore,
    export: ExportState,
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
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

fn real_tools() -> Tools {
    Tools::discover(&std::env::var_os("PATH").unwrap_or_default())
}

impl Rig {
    fn new() -> Self {
        Self::with(ExportState::new(Box::new(real_tools)))
    }

    fn with(export: ExportState) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        copy_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-export"), &repo);
        let home = tmp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let answers = tmp.path().join("answers");
        std::fs::write(&answers, "").unwrap();
        let picker = Arc::new(ScriptedPicker::new(answers.clone()));
        let st = RepoState::new(repo.clone(), Box::new(picker.clone()));
        let settings = SettingsStore::open(tmp.path().join("settings.json"));
        Rig { _tmp: tmp, repo, home, st, picker, answers, settings, export }
    }

    fn id(n: u32) -> String {
        format!("0199c0a0-0000-7000-8000-000000000{n}")
    }

    fn answer(&self, lines: &[String]) {
        std::fs::write(&self.answers, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    }

    fn base(&self, name: &str) -> PathBuf {
        let b = self.home.join(name);
        std::fs::create_dir_all(b.join("ROLAND")).unwrap();
        b
    }

    fn info(&self, n: u32) -> ExportInfo {
        do_export_info(&self.st, &self.settings, &FileTempoStore, &Self::id(n)).unwrap()
    }

    fn prepare(&self, n: u32, t: TargetReq) -> Result<Option<ExportPlan>, String> {
        do_prepare_export(&self.st, &self.settings, &self.export, &FileTempoStore, Some(self.home.clone()), &Self::id(n), t)
    }

    fn rc5(memory: &str) -> TargetReq {
        TargetReq::Rc5 { memory: memory.into() }
    }

    fn start(&self, token: &str, overwrite: bool) -> Result<ExportSnapshot, String> {
        let sink: Sink = Arc::new(|_| {});
        let path = self.settings_path();
        do_start_export(
            &self.st,
            &self.export,
            &FileTempoStore,
            token,
            overwrite,
            sink,
            Box::new(move |n| remember_memory(&SettingsStore::open(path), n)),
        )
    }

    fn settings_path(&self) -> PathBuf {
        self._tmp.path().join("settings.json")
    }

    fn wait(&self) -> ExportSnapshot {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            let s = self.export.snapshot().unwrap();
            if !matches!(s.phase, ExportPhase::Writing | ExportPhase::Finishing) {
                return s;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn info_for_each_fixture() {
    let rig = Rig::new();
    let i = rig.info(401);
    assert!(i.available && i.reason.is_none());
    assert_eq!(i.source.as_deref(), Some("backings/backing.flac"));
    assert_eq!(i.count_in, Some(CountInInfo { bpm: 120.0, first_beat_ms: 0.0, lead_ms: 2000.0 }));
    assert_eq!(i.memory, 1);
    assert_eq!(rig.info(402).reason.as_deref(), Some("No mix saved yet: save one in the Editor"));
    assert!(!rig.info(402).available && rig.info(402).source.is_none() && rig.info(402).count_in.is_none());
    let i = rig.info(403);
    assert!(i.available);
    assert_eq!(i.source.as_deref(), Some("backing.mp3"));
    assert!(rig.info(404).available);
    assert_eq!(rig.info(405).count_in, Some(CountInInfo { bpm: 100.0, first_beat_ms: 0.0, lead_ms: 2400.0 }));
    assert_eq!(rig.info(406).count_in.unwrap().first_beat_ms, 500.0);
    let i = rig.info(407);
    assert_eq!(i.count_in, Some(CountInInfo { bpm: 100.0, first_beat_ms: 3000.0, lead_ms: 2400.0 }));
    assert_eq!(
        rig.info(408).reason.as_deref(),
        Some("The saved mix (backings/backing.flac) is missing: save it again in the Editor")
    );
    assert!(rig.info(409).available);
    let i = rig.info(410);
    assert!(!i.available);
    assert_eq!(i.reason.as_deref(), Some(NO_TEMPO_REASON));
    assert_eq!(i.source.as_deref(), Some("backings/backing.flac"));
    assert!(i.count_in.is_none());
    assert!(do_export_info(&rig.st, &rig.settings, &FileTempoStore, "nope").is_err());
}

#[test]
fn prepare_refuses_no_tempo_and_no_backing_without_a_dialog() {
    let rig = Rig::new();
    let base = rig.base("rc5");
    rig.answer(&[format!("export-rc5 {}", base.display())]);
    assert_eq!(rig.prepare(410, Rig::rc5("7")).unwrap_err(), NO_TEMPO_REASON);
    assert_eq!(rig.prepare(402, Rig::rc5("7")).unwrap_err(), "No mix saved yet: save one in the Editor");
    assert!(rig.picker.starts().is_empty());
    assert!(!base.join("ROLAND/WAVE").exists());
}

#[test]
fn prepare_checks_before_the_dialog() {
    let rig = Rig::new();
    for bad in ["abc", "0", "100", ""] {
        assert_eq!(rig.prepare(401, Rig::rc5(bad)).unwrap_err(), "Enter a memory number from 1 to 99");
    }
    assert!(rig.picker.starts().is_empty());
    // An ffmpeg that lists no encoders.
    use std::os::unix::fs::PermissionsExt;
    let fake = rig.home.join("ffmpeg");
    std::fs::write(&fake, "#!/bin/sh\necho ' A..... aac AAC'\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let f2 = fake.clone();
    let rig = Rig::with(ExportState::new(Box::new(move || Tools { ffmpeg: Some(f2.clone()), ..real_tools() })));
    let e = rig.prepare(401, TargetReq::Mp3).unwrap_err();
    assert!(e.contains("libmp3lame"), "{e}");
    assert!(rig.prepare(401, TargetReq::Ogg).unwrap_err().contains("libvorbis"));
    assert!(rig.picker.starts().is_empty());
    // No tools at all.
    let rig = Rig::with(ExportState::new(Box::new(Tools::default)));
    assert!(rig.prepare(401, Rig::rc5("1")).unwrap_err().starts_with("Install ffmpeg"));
    assert!(rig.picker.starts().is_empty());
}

#[test]
fn prepare_uses_and_remembers_folders() {
    let rig = Rig::new();
    let base = rig.base("rc5");
    rig.answer(&[format!("export-rc5 {}", base.display())]);
    let plan = rig.prepare(401, Rig::rc5(" 7 ")).unwrap().unwrap();
    assert_eq!(plan.path, base.join("ROLAND/WAVE/007_1/007_1.WAV").display().to_string());
    assert!(!plan.exists);
    assert_eq!(rig.picker.starts(), [(DialogKind::ExportRc5, Some(rig.home.clone()))]);
    assert_eq!(rig.settings.get().rc5_dir, Some(base.clone()));
    assert_eq!(rig.settings.get().export_dir, None);

    // Next time the dialog opens in the remembered base.
    let other = rig.base("other");
    rig.answer(&[format!("export-rc5 {}", other.display())]);
    rig.prepare(401, Rig::rc5("8")).unwrap().unwrap();
    assert_eq!(rig.picker.starts()[1], (DialogKind::ExportRc5, Some(base.clone())));
    assert_eq!(rig.settings.get().rc5_dir, Some(other.clone()));

    // A missing remembered base: the dialog opens at home and the setting is kept.
    let moved = rig.home.join("moved");
    std::fs::rename(&other, &moved).unwrap();
    let plain = rig.home.join("plain");
    std::fs::create_dir(&plain).unwrap();
    rig.answer(&[format!("export-rc5 {}", plain.display())]);
    let e = rig.prepare(401, Rig::rc5("8")).unwrap_err();
    assert!(e.starts_with("No ROLAND folder in"), "{e}");
    assert_eq!(rig.picker.starts()[2], (DialogKind::ExportRc5, Some(rig.home.clone())));
    // A refused base is not remembered; the old (missing) one stays.
    assert_eq!(rig.settings.get().rc5_dir, Some(other));

    // MP3: its own folder setting and dialog kind.
    let out = rig.home.join("out");
    std::fs::create_dir(&out).unwrap();
    rig.answer(&[format!("export-audio {}", out.display()), format!("export-audio {}", out.display())]);
    let plan = rig.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    assert_eq!(plan.path, out.join("The Example Band - Stem Mix.mp3").display().to_string());
    assert_eq!(rig.picker.starts()[3], (DialogKind::ExportAudio, Some(rig.home.clone())));
    assert_eq!(rig.settings.get().export_dir, Some(out.clone()));
    rig.prepare(401, TargetReq::Ogg).unwrap().unwrap();
    assert_eq!(rig.picker.starts()[4], (DialogKind::ExportAudio, Some(out)));
    // After a restart.
    assert_eq!(SettingsStore::open(rig.settings_path()).get().rc5_dir, rig.settings.get().rc5_dir);
}

#[test]
fn dialog_cancel_is_none() {
    let rig = Rig::new();
    rig.answer(&["export-rc5 CANCEL".into(), "export-audio CANCEL".into()]);
    assert_eq!(rig.prepare(401, Rig::rc5("1")).unwrap(), None);
    assert_eq!(rig.prepare(401, TargetReq::Mp3).unwrap(), None);
    assert_eq!(rig.picker.starts().len(), 2);
    let s = rig.settings.get();
    assert!(s.rc5_dir.is_none() && s.export_dir.is_none() && s.rc5_memory.is_none());
}

#[test]
fn start_rereads_the_tempo() {
    let rig = Rig::new();
    let base = rig.base("rc5");
    rig.answer(&[format!("export-rc5 {}", base.display())]);
    let plan = rig.prepare(406, Rig::rc5("7")).unwrap().unwrap();
    // The first beat moves between prepare and start (500 ms -> 1500 ms).
    let pj = rig.repo.join("tracks").join(Rig::id(406)).join("player.json");
    std::fs::write(
        &pj,
        r#"{"schema_version": 6, "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 1500, "bpm": 120}]}}"#,
    )
    .unwrap();
    rig.start(&plan.token, false).unwrap();
    assert_eq!(rig.wait().phase, ExportPhase::Done);
    let b = std::fs::read(base.join("ROLAND/WAVE/007_1/007_1.WAV")).unwrap();
    let left: Vec<f32> = b[44..].chunks_exact(8).map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]])).collect();
    let click = Click::new(44100).samples().to_vec();
    assert_eq!(&left[66150..66150 + click.len()], &click[..]);
    assert!(left[22050..22050 + click.len()].iter().all(|&v| v == 0.0));
    // Stale and emptied-map starts.
    assert_eq!(rig.start("x1", false).unwrap_err(), STALE_MESSAGE);
    rig.answer(&[format!("export-rc5 {}", base.display())]);
    let plan = rig.prepare(406, Rig::rc5("8")).unwrap().unwrap();
    std::fs::write(&pj, r#"{"schema_version": 6}"#).unwrap();
    assert_eq!(rig.start(&plan.token, false).unwrap_err(), NO_TEMPO_REASON);
    assert!(!base.join("ROLAND/WAVE/008_1").exists());
}

#[test]
fn memory_is_remembered_after_done() {
    let rig = Rig::new();
    let base = rig.base("rc5");
    rig.answer(&[format!("export-rc5 {}", base.display())]);
    let plan = rig.prepare(401, Rig::rc5("7")).unwrap().unwrap();
    assert_eq!(rig.settings.get().rc5_memory, None);
    rig.start(&plan.token, false).unwrap();
    assert_eq!(rig.wait().phase, ExportPhase::Done);
    // The settings file was written by the finished export (a second store reads it).
    let reopened = SettingsStore::open(rig.settings_path());
    assert_eq!(reopened.get().rc5_memory, Some(7));
    assert_eq!(do_export_info(&rig.st, &reopened, &FileTempoStore, &Rig::id(401)).unwrap().memory, 8);
    reopened.set_rc5_memory(Some(99)).unwrap();
    assert_eq!(do_export_info(&rig.st, &reopened, &FileTempoStore, &Rig::id(401)).unwrap().memory, 99);

    // An MP3 export does not touch the memory.
    let out = rig.home.join("out");
    std::fs::create_dir(&out).unwrap();
    rig.answer(&[format!("export-audio {}", out.display())]);
    let plan = rig.prepare(403, TargetReq::Mp3).unwrap().unwrap();
    let ran = Arc::new(AtomicUsize::new(0));
    let r2 = ran.clone();
    do_start_export(
        &rig.st,
        &rig.export,
        &FileTempoStore,
        &plan.token,
        false,
        Arc::new(|_| {}),
        Box::new(move |_| {
            r2.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    assert_eq!(rig.wait().phase, ExportPhase::Done);
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert!(out.join("The Example Band - Plain MP3.mp3").is_file());
}

#[test]
fn start_uses_the_track_dir_of_the_plan_when_the_root_changed() {
    let rig = Rig::new();
    let out = rig.home.join("out2");
    std::fs::create_dir(&out).unwrap();
    rig.answer(&[format!("export-audio {}", out.display())]);
    let plan = rig.prepare(403, TargetReq::Mp3).unwrap().unwrap();
    let other = rig._tmp.path().join("other-root");
    std::fs::create_dir_all(&other).unwrap();
    *rig.st.lock() = other;
    rig.start(&plan.token, false).unwrap();
    assert_eq!(rig.wait().phase, ExportPhase::Done);
    assert!(out.join("The Example Band - Plain MP3.mp3").is_file());
}
