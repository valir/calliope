//! Rust-side acceptance tests of specs/gui-export.md (plan section 7), through the `do_*`
//! functions of `export_ipc`. Temporary directories only (a copy of `tests/fixtures/library-export`,
//! temp destinations), the scripted dialog, the local ffmpeg/ffprobe; no real drive, no audio device.
//! Every length is "the original + 5 beats at the first section's BPM" (owner answer 1).

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use calliope_gui::export_ipc::{
    do_export_info, do_prepare_export, do_start_export, remember_memory, ExportInfo,
};
use calliope_gui::export_job::{
    ExportPhase, ExportPlan, ExportSnapshot, ExportState, Sink, TargetReq,
};
use calliope_gui::export_rules::NO_TEMPO_REASON;
use calliope_gui::ipc::RepoState;
use calliope_gui::picker::ScriptedPicker;
use calliope_gui::repository::{Repository, SaveTrackRequest};
use calliope_gui::settings::SettingsStore;
use calliope_gui::tempo_store::FileTempoStore;
use calliope_gui::tools::Tools;

const BAND: &str = "The Example Band";

type Tree = BTreeMap<String, Option<Vec<u8>>>;

/// Every file (bytes) and folder under `dir`, by relative path.
fn tree(dir: &Path) -> Tree {
    fn walk(root: &Path, dir: &Path, out: &mut Tree) {
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
    let mut out = Tree::new();
    walk(dir, dir, &mut out);
    out
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

fn real_tools() -> Tools {
    Tools::discover(&std::env::var_os("PATH").unwrap_or_default())
}

fn id(n: u32) -> String {
    format!("0199c0a0-0000-7000-8000-000000000{n}")
}

struct Env {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    answers: PathBuf,
    settings_file: PathBuf,
    before: Tree,
    st: RepoState,
    picker: Arc<ScriptedPicker>,
    export: ExportState,
}

impl Env {
    fn new() -> Self {
        Self::with(ExportState::new(Box::new(real_tools)))
    }

    fn with(export: ExportState) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-export"),
            &repo,
        );
        let home = tmp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let answers = tmp.path().join("answers");
        std::fs::write(&answers, "").unwrap();
        let picker = Arc::new(ScriptedPicker::new(answers.clone()));
        let st = RepoState::new(repo.clone(), Box::new(picker.clone()));
        let settings_file = tmp.path().join("settings.json");
        let before = tree(&repo);
        Env {
            _tmp: tmp,
            repo,
            home,
            answers,
            settings_file,
            before,
            st,
            picker,
            export,
        }
    }

    /// The settings as the file holds them: a restart before every call.
    fn settings(&self) -> SettingsStore {
        SettingsStore::open(self.settings_file.clone())
    }

    fn repo_unchanged(&self) {
        assert!(tree(&self.repo) == self.before, "the repository changed");
    }

    /// Like `repo_unchanged`, but `track.json` may differ in its `exports` (a successful export
    /// records its info there); every other file is byte-identical and nothing is added or removed.
    fn repo_unchanged_except_exports(&self) {
        let now = tree(&self.repo);
        assert_eq!(
            now.keys().collect::<Vec<_>>(),
            self.before.keys().collect::<Vec<_>>(),
            "files were added or removed"
        );
        for (k, v) in &now {
            let was = &self.before[k];
            if k.ends_with("/track.json") {
                let strip = |b: &Option<Vec<u8>>| {
                    let mut j: serde_json::Value =
                        serde_json::from_slice(b.as_ref().unwrap()).unwrap();
                    j.as_object_mut().unwrap().remove("exports");
                    j
                };
                assert_eq!(strip(v), strip(was), "{k} changed beyond its exports");
            } else {
                assert!(v == was, "{k} changed");
            }
        }
    }

    /// A folder with a `ROLAND` folder in it, like an RC-5 drive.
    fn rc5(&self, name: &str) -> PathBuf {
        let b = self.home.join(name);
        std::fs::create_dir_all(b.join("ROLAND")).unwrap();
        b
    }

    fn folder(&self, name: &str) -> PathBuf {
        let b = self.home.join(name);
        std::fs::create_dir_all(&b).unwrap();
        b
    }

    fn info(&self, n: u32) -> ExportInfo {
        do_export_info(&self.st, &self.settings(), &FileTempoStore, &id(n)).unwrap()
    }

    /// Scripts the folder dialog of `target` to answer `base`.
    fn answer(&self, target: &TargetReq, base: &Path) {
        let kind = if matches!(target, TargetReq::Rc5 { .. }) {
            "export-rc5"
        } else {
            "export-audio"
        };
        let old = std::fs::read_to_string(&self.answers).unwrap();
        std::fs::write(&self.answers, format!("{old}{kind} {}\n", base.display())).unwrap();
    }

    fn prepare(&self, n: u32, target: TargetReq) -> Result<Option<ExportPlan>, String> {
        do_prepare_export(
            &self.st,
            &self.settings(),
            &self.export,
            &FileTempoStore,
            Some(self.home.clone()),
            &id(n),
            target,
        )
    }

    fn start(&self, token: &str, overwrite: bool) -> Result<ExportSnapshot, String> {
        let path = self.settings_file.clone();
        let sink: Sink = Arc::new(|_| {});
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

    fn wait(&self) -> ExportSnapshot {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            let s = self.export.snapshot().unwrap();
            if !matches!(s.phase, ExportPhase::Writing | ExportPhase::Finishing) {
                return s;
            }
            assert!(Instant::now() < end, "the export did not end: {s:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Dialog -> plan -> start (overwriting when the plan says the target exists) -> end.
    fn export(&self, n: u32, target: TargetReq, base: &Path) -> (ExportPlan, ExportSnapshot) {
        self.answer(&target, base);
        let plan = self.prepare(n, target).unwrap().expect("a plan");
        self.start(&plan.token, plan.exists).unwrap();
        let s = self.wait();
        (plan, s)
    }
}

fn rc5(m: &str) -> TargetReq {
    TargetReq::Rc5 { memory: m.into() }
}

fn wav_path(base: &Path, n: u32) -> PathBuf {
    base.join(format!("ROLAND/WAVE/{n:03}_1/{n:03}_1.WAV"))
}

/// Checks the 44-byte float32 stereo header exactly and returns (rate, left, right).
fn read_wav(p: &Path) -> (u32, Vec<f32>, Vec<f32>) {
    let b = std::fs::read(p).unwrap();
    let n = (b.len() - 44) as u32;
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + n);
    assert_eq!(&b[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(b[16..20].try_into().unwrap()), 16);
    assert_eq!(
        u16::from_le_bytes(b[20..22].try_into().unwrap()),
        3,
        "IEEE float"
    );
    assert_eq!(
        u16::from_le_bytes(b[22..24].try_into().unwrap()),
        2,
        "stereo"
    );
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), rate * 8);
    assert_eq!(u16::from_le_bytes(b[32..34].try_into().unwrap()), 8);
    assert_eq!(u16::from_le_bytes(b[34..36].try_into().unwrap()), 32);
    assert_eq!(&b[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes(b[40..44].try_into().unwrap()),
        n,
        "no chunk after the samples"
    );
    let s: Vec<f32> = b[44..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let l = s.chunks_exact(2).map(|f| f[0]).collect();
    let r = s.chunks_exact(2).map(|f| f[1]).collect();
    (rate, l, r)
}

/// Any audio file decoded by ffmpeg to stereo f32 at 44.1 kHz: (left, right).
fn decode(p: &Path) -> (Vec<f32>, Vec<f32>) {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(p)
        .args([
            "-map",
            "0:a:0",
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ac",
            "2",
            "-ar",
            "44100",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    (
        s.chunks_exact(2).map(|f| f[0]).collect(),
        s.chunks_exact(2).map(|f| f[1]).collect(),
    )
}

fn probe(p: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format_tags:stream_tags:stream=codec_name",
            "-of",
            "json",
        ])
        .arg(p)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

fn tag(v: &serde_json::Value, key: &str) -> Option<String> {
    [&v["format"]["tags"], &v["streams"][0]["tags"]]
        .iter()
        .find_map(|t| {
            t.as_object()?
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str().unwrap().to_string())
        })
}

fn goertzel(x: &[f32], hz: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * hz / 44100.0;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &v in x {
        let s = v + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt() / x.len().max(1) as f32
}

fn assert_seconds(frames: usize, want: f64, tol: f64) {
    let s = frames as f64 / 44100.0;
    assert!((s - want).abs() <= tol, "{s} s, expected {want} s");
}

// ---- which tracks can be exported ----

#[test]
fn ac_stem_track_with_mix_is_exportable() {
    let e = Env::new();
    let i = e.info(401);
    assert!(i.available && i.reason.is_none());
    assert_eq!(i.source.as_deref(), Some("backings/backing.flac"));
    for (target, kind, base) in [
        (rc5("7"), "export-rc5", e.rc5("rc5")),
        (TargetReq::Mp3, "export-audio", e.folder("mp3")),
        (TargetReq::Ogg, "export-audio", e.folder("ogg")),
    ] {
        std::fs::write(&e.answers, format!("{kind} {}\n", base.display())).unwrap();
        assert!(e.prepare(401, target).unwrap().is_some(), "{kind}");
    }
    e.repo_unchanged();
}

#[test]
fn ac_stem_track_without_mix_is_not_exportable() {
    let e = Env::new();
    for (n, why) in [
        (402, "No mix saved yet: save one in the Editor"),
        (
            408,
            "The saved mix (backings/backing.flac) is missing: save it again in the Editor",
        ),
    ] {
        let i = e.info(n);
        assert!(!i.available);
        assert_eq!(i.reason.as_deref(), Some(why));
        for t in [rc5("7"), TargetReq::Mp3, TargetReq::Ogg] {
            assert_eq!(e.prepare(n, t).unwrap_err(), why);
        }
    }
    assert!(
        e.picker.starts().is_empty(),
        "no dialog for a track that cannot be exported"
    );
    e.repo_unchanged();
}

#[test]
fn ac_track_without_tempo_is_not_exportable() {
    let e = Env::new();
    let i = e.info(410);
    assert!(!i.available && i.count_in.is_none());
    assert_eq!(i.reason.as_deref(), Some(NO_TEMPO_REASON));
    let base = e.rc5("rc5");
    for t in [rc5("7"), TargetReq::Mp3, TargetReq::Ogg] {
        assert_eq!(e.prepare(410, t).unwrap_err(), NO_TEMPO_REASON);
    }
    assert!(e.picker.starts().is_empty());
    assert!(!base.join("ROLAND/WAVE").exists());
    e.repo_unchanged();
}

#[test]
fn ac_plain_backing_is_exportable() {
    let e = Env::new();
    let i = e.info(403);
    assert!(i.available);
    assert_eq!(i.source.as_deref(), Some("backing.mp3"));
    let base = e.rc5("rc5");
    let (_, s) = e.export(403, rc5("3"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (rate, l, _) = read_wav(&wav_path(&base, 3));
    assert_eq!(rate, 44100);
    assert_seconds(l.len(), 6.5, 0.03);
    e.repo_unchanged_except_exports();
}

// ---- the RC-5 ----

#[test]
fn ac_rc5_memory_7() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let (plan, s) = e.export(401, rc5("7"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert_eq!(plan.path, wav_path(&base, 7).display().to_string());
    let (rate, l, r) = read_wav(&wav_path(&base, 7));
    assert_eq!(rate, 44100);
    // 5 beats at 120 BPM (2.5 s) + the 4 s original.
    assert_eq!(l.len(), 110250 + 176400);
    let (ol, or) = decode(
        &e.repo
            .join("tracks")
            .join(id(401))
            .join("backings/backing.flac"),
    );
    assert_eq!(ol.len(), 176400);
    // The original is unchanged after the lead; the ticks are all inside the lead.
    assert!(
        l[110250..] == ol[..] && r[110250..] == or[..],
        "the original differs"
    );
    assert!(l[..110250].iter().any(|&v| v != 0.0), "no ticks");
    assert_eq!(
        std::fs::read_dir(wav_path(&base, 7).parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_rc5_without_roland() {
    let e = Env::new();
    let base = e.folder("plain");
    let before = tree(&base);
    e.answer(&rc5("7"), &base);
    let err = e.prepare(401, rc5("7")).unwrap_err();
    assert_eq!(
        err,
        format!(
            "No ROLAND folder in {}: is the RC-5 mounted?",
            base.display()
        )
    );
    assert!(tree(&base) == before && before.is_empty());
    assert_eq!(
        e.settings().get().rc5_dir,
        None,
        "a refused folder is not remembered"
    );
    e.repo_unchanged();
}

#[test]
fn ac_rc5_overwrite_confirmed() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let dir = wav_path(&base, 7).parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(wav_path(&base, 7), b"old").unwrap();
    std::fs::write(dir.join("notes.txt"), b"mine").unwrap();
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    assert!(plan.exists, "the confirmation can name {}", plan.path);
    // Without the confirmation nothing is replaced.
    assert!(e
        .start(&plan.token, false)
        .unwrap_err()
        .contains("already exists"));
    assert_eq!(std::fs::read(wav_path(&base, 7)).unwrap(), b"old");
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    e.start(&plan.token, true).unwrap();
    assert_eq!(e.wait().phase, ExportPhase::Done);
    let (_, l, _) = read_wav(&wav_path(&base, 7));
    assert_eq!(l.len(), 110250 + 176400);
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_overwrite_declined() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let dir = wav_path(&base, 7).parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(wav_path(&base, 7), b"old").unwrap();
    let before = tree(&base);
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    assert!(plan.exists);
    // The user answers No: `start_export` is never called. Nothing changed.
    assert!(tree(&base) == before);
    assert!(e.export.snapshot().is_none());
    e.repo_unchanged();
}

#[test]
fn ac_rc5_mono_becomes_stereo() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let (_, s) = e.export(404, rc5("2"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (rate, l, r) = read_wav(&wav_path(&base, 2));
    assert_eq!(rate, 44100);
    assert!(l == r, "both channels carry the same signal");
    assert!(l.iter().any(|v| v.abs() > 0.1));
    assert_eq!(l.len(), 110250 + 176400);
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_rc5_bad_memory_numbers() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let before = tree(&base);
    e.answer(&rc5("7"), &base);
    for bad in ["0", "100", "abc", "", "7.5", "-3"] {
        assert_eq!(
            e.prepare(401, rc5(bad)).unwrap_err(),
            "Enter a memory number from 1 to 99",
            "{bad:?}"
        );
    }
    assert!(e.picker.starts().is_empty(), "refused before any dialog");
    assert!(tree(&base) == before);
    e.repo_unchanged();
}

#[test]
fn ac_rc5_resamples_48k() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let (_, s) = e.export(405, rc5("5"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (rate, l, _) = read_wav(&wav_path(&base, 5));
    assert_eq!(rate, 44100);
    // 5 beats at 100 BPM (3 s) + the 4 s original.
    let want = 132300 + 176400;
    assert!(l.len().abs_diff(want) <= 1, "{} vs {want}", l.len());
    let tone = &l[132300 + 20000..132300 + 40000];
    assert!(goertzel(tone, 1000.0) > 5.0 * goertzel(tone, 440.0));
    e.repo_unchanged_except_exports();
}

// ---- MP3 and OGG ----

#[test]
fn ac_mp3_export() {
    let e = Env::new();
    let out = e.folder("out");
    let (plan, s) = e.export(401, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let file = out.join(format!("{BAND} - Stem Mix.mp3"));
    assert_eq!(plan.path, file.display().to_string());
    let v = probe(&file);
    assert_eq!(v["streams"][0]["codec_name"], "mp3");
    assert_eq!(tag(&v, "title").as_deref(), Some("Stem Mix"));
    assert_eq!(tag(&v, "artist").as_deref(), Some(BAND));
    assert_eq!(tag(&v, "album").as_deref(), Some("Export Tests"));
    assert_eq!(tag(&v, "track").as_deref(), Some("3"));
    assert_eq!(tag(&v, "date").as_deref(), Some("2021"));
    assert_seconds(decode(&file).0.len(), 6.5, 0.03);
    // A track with an odd name and no album.
    let (_, s) = e.export(409, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let odd = out.join("AC_DC_ - Live_ A_B_.mp3");
    assert!(odd.is_file());
    assert_eq!(tag(&probe(&odd), "track"), None);
    assert_eq!(
        std::fs::read_dir(&out).unwrap().count(),
        2,
        "no part file is left"
    );
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_ogg_export() {
    let e = Env::new();
    let out = e.folder("out");
    let (_, s) = e.export(401, TargetReq::Ogg, &out);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let file = out.join(format!("{BAND} - Stem Mix.ogg"));
    let v = probe(&file);
    assert_eq!(v["streams"][0]["codec_name"], "vorbis");
    assert_eq!(tag(&v, "title").as_deref(), Some("Stem Mix"));
    assert_eq!(tag(&v, "artist").as_deref(), Some(BAND));
    assert_eq!(tag(&v, "album").as_deref(), Some("Export Tests"));
    assert_seconds(decode(&file).0.len(), 6.5, 0.03);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    e.repo_unchanged_except_exports();
}

// ---- cancel and failure ----

#[test]
fn ac_cancel_leaves_nothing() {
    let e = Env::with(ExportState::new(Box::new(real_tools)).with_pace(5.0));
    let base = e.rc5("rc5");
    let before = tree(&base);
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    let snap = e.start(&plan.token, false).unwrap();
    assert_eq!(snap.phase, ExportPhase::Writing);
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        wav_path(&base, 7).parent().unwrap().exists(),
        "the export is under way"
    );
    assert!(e.export.cancel(&snap.job).is_some());
    let s = e.wait();
    assert_eq!(s.phase, ExportPhase::Cancelled);
    assert!(tree(&base) == before, "nothing is left in the destination");
    e.repo_unchanged();
}

#[test]
fn ac_unwritable_destination() {
    let e = Env::new();
    let root = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"));
    if root {
        eprintln!("skipped: running as root");
        return;
    }
    let out = e.folder("ro");
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o555)).unwrap();
    e.answer(&TargetReq::Mp3, &out);
    let plan = e.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    let err = e.start(&plan.token, false).unwrap_err();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        err.starts_with("Cannot write to ") && err.contains("Permission denied"),
        "{err}"
    );
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    e.repo_unchanged();
}

// ---- remembered folders and memory ----

#[test]
fn ac_remembered_after_restart() {
    let e = Env::new();
    let rc5_a = e.rc5("rc5-a");
    let out = e.folder("out");
    let (_, s) = e.export(401, rc5("7"), &rc5_a);
    assert_eq!(s.phase, ExportPhase::Done);
    let (_, s) = e.export(403, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Done);

    // "Restart": a store opened again from the same file, new dialog and state objects.
    let settings = e.settings();
    assert_eq!(settings.get().rc5_dir, Some(rc5_a.clone()));
    assert_eq!(settings.get().export_dir, Some(out.clone()));
    assert_eq!(settings.get().rc5_memory, Some(7));
    // The exported track shows its recorded memory, any other one the suggestion after it.
    let memory = |n| {
        do_export_info(&e.st, &settings, &FileTempoStore, &id(n))
            .unwrap()
            .memory
    };
    assert_eq!((memory(401), memory(403)), (7, 8));

    let answers = e.home.join("answers2");
    std::fs::write(
        &answers,
        format!(
            "export-rc5 {}\nexport-audio {}\n",
            rc5_a.display(),
            out.display()
        ),
    )
    .unwrap();
    let picker = Arc::new(ScriptedPicker::new(answers));
    let st = RepoState::new(e.repo.clone(), Box::new(picker.clone()));
    let export = ExportState::new(Box::new(real_tools));
    let prep = |n, t| {
        do_prepare_export(
            &st,
            &settings,
            &export,
            &FileTempoStore,
            Some(e.home.clone()),
            &id(n),
            t,
        )
        .unwrap()
        .unwrap()
    };
    prep(401, rc5("8"));
    prep(403, TargetReq::Ogg);
    use calliope_gui::picker::DialogKind;
    assert_eq!(
        picker.starts(),
        [
            (DialogKind::ExportRc5, Some(rc5_a.clone())),
            (DialogKind::ExportAudio, Some(out.clone()))
        ]
    );

    // 99 stays 99 (for a track without a recorded memory).
    settings.set_rc5_memory(Some(99)).unwrap();
    assert_eq!(
        do_export_info(&e.st, &e.settings(), &FileTempoStore, &id(403))
            .unwrap()
            .memory,
        99
    );
    e.repo_unchanged_except_exports();
}

// ---- the count-in ----

#[test]
fn ac_count_in_before_the_first_beat() {
    // (track, file, first beat frame in the original, lead frames, original length in frames)
    for (n, first, lead) in [(406u32, 22050usize, 110250usize), (407, 132300, 132300)] {
        let e = Env::new();
        let wav_base = e.rc5("rc5");
        let out = e.folder("out");
        let (_, s) = e.export(n, rc5("7"), &wav_base);
        assert_eq!(s.phase, ExportPhase::Done, "{n} {s:?}");
        let (_, wl, wr) = read_wav(&wav_path(&wav_base, 7));
        let (_, s) = e.export(n, TargetReq::Mp3, &out);
        assert_eq!(s.phase, ExportPhase::Done, "{n} {s:?}");
        let mp3 = std::fs::read_dir(&out)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let (ml, _) = decode(&mp3);

        let original = e.repo.join("tracks").join(id(n));
        let src = if n == 406 {
            original.join("backings/backing.flac")
        } else {
            original.join("intro.flac")
        };
        let (ol, or) = decode(&src);
        // Length: the original + 5 beats.
        assert_eq!(wl.len(), lead + ol.len(), "{n}");
        assert_seconds(ml.len(), (lead + ol.len()) as f64 / 44100.0, 0.03);
        // The ticks sit at first + {0, 1, 2, 2.5, 3, 3.5} beats; the last tick is at first + 3.5 beats, the first beat at the end of the 5-beat lead.
        let beat = lead / 5;
        let ticks: Vec<usize> = [0.0, 1.0, 2.0, 2.5, 3.0, 3.5]
            .iter()
            .map(|o| first + (o * beat as f64).round() as usize)
            .collect();
        assert_eq!(
            ticks[5],
            first + (3.5 * beat as f64).round() as usize,
            "the last tick"
        );
        assert!(
            ticks[5] + 1400 < lead + first,
            "the 5th beat of the lead is silent before the first beat"
        );
        assert_eq!(
            first + 5 * beat,
            lead + first,
            "the first beat is 5 beats after the start"
        );
        // WAV: silence + exactly six ticks until the original starts; the original after the lead
        // is bit-exact except under a tick that overlaps it.
        for (k, v) in wl[..lead].iter().enumerate() {
            if !ticks.iter().any(|&t| k >= t && k < t + 1400) {
                assert_eq!(*v, 0.0, "{n}: frame {k} should be silent");
            }
        }
        for &t in &ticks {
            assert!(
                goertzel(&wl[t..t + 1300], 1500.0) > 0.05,
                "{n}: no tick at {t}"
            );
        }
        let untouched = |k: usize| !ticks.iter().any(|&t| k >= t && k < t + 1400);
        for k in (lead..wl.len()).step_by(7) {
            if untouched(k) {
                assert_eq!(wl[k], ol[k - lead], "{n}: frame {k}");
                assert_eq!(wr[k], or[k - lead], "{n}: frame {k}");
            }
        }
        // MP3: the same ticks in the lossy copy (only those in the digital silence are checked).
        for &t in ticks.iter().filter(|&&t| t + 3000 < lead.max(first)) {
            let quiet = &ml[t + 3000..t + 4300];
            assert!(
                goertzel(&ml[t + 50..t + 1350], 1500.0) > 10.0 * goertzel(quiet, 1500.0).max(1e-6),
                "{n}: mp3 tick at {t}"
            );
        }
        e.repo_unchanged_except_exports();
    }
}

// ==== QA additions: owner decisions and edge cases ====

fn write_tempo(e: &mut Env, n: u32, json: serde_json::Value) {
    let p = e.repo.join("tracks").join(id(n)).join("player.json");
    std::fs::write(p, serde_json::to_vec(&json).unwrap()).unwrap();
    e.before = tree(&e.repo);
}

fn tempo(bpb: u32, start_ms: u32, bpm: f64) -> serde_json::Value {
    serde_json::json!({"schema_version": 6, "tempo": {"beats_per_bar": bpb,
        "segments": [{"start_ms": start_ms, "bpm": bpm}]}})
}

/// Onset of the ticks: the first sample that differs from the expected (silence ++ original),
/// searched in `[from, to)`. `orig` is the interleaved-free left channel of the source.
fn tick_onset(out: &[f32], orig: &[f32], lead: usize, from: usize, to: usize) -> Option<usize> {
    (from..to.min(out.len())).find(|&k| out[k] != if k >= lead { orig[k - lead] } else { 0.0 })
}

fn probe_stream(p: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,sample_rate,channels:format=duration",
            "-of",
            "json",
        ])
        .arg(p)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn real_ffmpeg() -> PathBuf {
    real_tools().ffmpeg.unwrap()
}

fn ticks_for(first: usize, beat: usize) -> Vec<usize> {
    [0.0, 1.0, 2.0, 2.5, 3.0, 3.5]
        .iter()
        .map(|o| first + (o * beat as f64).round() as usize)
        .collect()
}

#[test]
fn ac_count_in_odd_beat_length_rounds_once_and_ticks_are_exact() {
    let mut e = Env::new();
    write_tempo(&mut e, 401, tempo(4, 333, 120.7));
    let base = e.rc5("rc5");
    let (_, s) = e.export(401, rc5("7"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (_, l, _) = read_wav(&wav_path(&base, 7));
    let (ol, _) = decode(
        &e.repo
            .join("tracks")
            .join(id(401))
            .join("backings/backing.flac"),
    );
    let beat = (44100.0f64 * 60.0 / 120.7).round() as usize;
    let first = (333.0f64 * 44.1).round() as usize;
    assert_eq!(
        l.len(),
        5 * beat + 176400,
        "original + exactly 5 beats, the beat rounded once"
    );
    for &t in &ticks_for(first, beat) {
        let o = tick_onset(&l, &ol, 5 * beat, t.saturating_sub(200), t + 400)
            .unwrap_or_else(|| panic!("no tick near {t}"));
        assert!(o >= t && o <= t + 3, "tick at {t} starts at {o}");
    }
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_count_in_ignores_speed_and_bar_length() {
    let mut e = Env::new();
    let mut t = tempo(3, 0, 120.0);
    t["speed"] = serde_json::json!(0.5);
    write_tempo(&mut e, 401, t);
    let base = e.rc5("rc5");
    let (_, s) = e.export(401, rc5("7"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (_, l, _) = read_wav(&wav_path(&base, 7));
    assert_eq!(
        l.len(),
        110250 + 176400,
        "always 5 beats at the first section's BPM, speed 100 %"
    );
    let info = e.info(401);
    assert_eq!(info.count_in.unwrap().lead_ms, 2500.0);
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_count_in_uses_the_first_section_only_and_the_map_as_it_is_at_start() {
    let mut e = Env::new();
    write_tempo(
        &mut e,
        401,
        serde_json::json!({"schema_version": 6, "tempo": {"beats_per_bar": 4,
        "segments": [{"start_ms": 0, "bpm": 120.0}, {"start_ms": 1000, "bpm": 60.0}]}}),
    );
    let base = e.rc5("rc5");
    let (_, s) = e.export(401, rc5("1"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert_eq!(read_wav(&wav_path(&base, 1)).1.len(), 110250 + 176400);

    // The first beat moves between prepare and start: the ticks follow the new map.
    e.answer(&rc5("2"), &base);
    let plan = e.prepare(401, rc5("2")).unwrap().unwrap();
    write_tempo(&mut e, 401, tempo(4, 1000, 120.0));
    e.start(&plan.token, plan.exists).unwrap();
    assert_eq!(e.wait().phase, ExportPhase::Done);
    let (_, l, _) = read_wav(&wav_path(&base, 2));
    let (ol, _) = decode(
        &e.repo
            .join("tracks")
            .join(id(401))
            .join("backings/backing.flac"),
    );
    assert_eq!(l.len(), 110250 + 176400);
    for &t in &ticks_for(44100, 22050) {
        let o = tick_onset(&l, &ol, 110250, t.saturating_sub(200), t + 400)
            .unwrap_or_else(|| panic!("no tick near {t}"));
        assert!(o >= t && o <= t + 3, "tick at {t} starts at {o}");
    }
    assert!(
        tick_onset(&l, &ol, 110250, 0, 44100).is_none(),
        "nothing before the moved first tick"
    );

    // The map is emptied between prepare and start: refused, nothing written.
    e.answer(&rc5("3"), &base);
    let plan = e.prepare(401, rc5("3")).unwrap().unwrap();
    std::fs::remove_file(e.repo.join("tracks").join(id(401)).join("player.json")).unwrap();
    e.before = tree(&e.repo);
    assert!(e.start(&plan.token, false).is_err());
    assert!(!wav_path(&base, 3).exists() && !wav_path(&base, 3).parent().unwrap().exists());
    e.repo_unchanged();
}

#[test]
fn ac_rc5_memory_numbers_at_the_boundaries() {
    let e = Env::new();
    let base = e.rc5("rc5");
    for (m, folder) in [
        ("1", "001_1"),
        ("99", "099_1"),
        (" 7 ", "007_1"),
        ("007", "007_1"),
    ] {
        let (plan, s) = e.export(409, rc5(m), &base);
        assert_eq!(s.phase, ExportPhase::Done, "{m} {s:?}");
        let want = base.join(format!("ROLAND/WAVE/{folder}/{}.WAV", folder));
        assert_eq!(plan.path, want.display().to_string());
        assert!(want.is_file());
        read_wav(&want);
        // "007" after " 7 " asks for confirmation; e.export overwrites on exists.
    }
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_rc5_refusals_write_nothing() {
    let e = Env::new();
    let missing = e.home.join("not-there");
    let file = e.home.join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let roland_file = e.folder("rf");
    std::fs::write(roland_file.join("ROLAND"), b"x").unwrap();
    let wave_file = e.rc5("wf");
    std::fs::create_dir_all(wave_file.join("ROLAND")).unwrap();
    std::fs::write(wave_file.join("ROLAND/WAVE"), b"x").unwrap();
    let target_dir = e.rc5("td");
    std::fs::create_dir_all(wav_path(&target_dir, 7)).unwrap();
    let roland_itself = e.rc5("drive").join("ROLAND");
    let cases = [
        &missing,
        &file,
        &roland_file,
        &wave_file,
        &target_dir,
        &roland_itself,
    ];
    let before: Vec<Tree> = cases
        .iter()
        .map(|b| if b.is_dir() { tree(b) } else { Tree::new() })
        .collect();
    for b in cases {
        e.answer(&rc5("7"), b);
        assert!(e.prepare(401, rc5("7")).is_err(), "{}", b.display());
    }
    assert!(!missing.exists());
    for (b, t) in cases.iter().zip(before) {
        if b.is_dir() {
            assert!(tree(b) == t, "{} changed", b.display());
        }
    }
    let err = {
        e.answer(&rc5("7"), &roland_itself);
        e.prepare(401, rc5("7")).unwrap_err()
    };
    assert!(err.contains("is the RC-5 mounted?"), "{err}");
    assert_eq!(
        e.settings().get().rc5_dir,
        None,
        "no refused folder is remembered"
    );
    e.repo_unchanged();
}

#[test]
fn ac_destination_inside_the_repository_is_refused() {
    let mut e = Env::new();
    let inside = e.repo.join("tracks");
    e.answer(&TargetReq::Mp3, &inside);
    assert!(e.prepare(401, TargetReq::Mp3).is_err());
    let rc = e.repo.join("drive");
    std::fs::create_dir_all(rc.join("ROLAND")).unwrap();
    e.before = tree(&e.repo);
    e.answer(&rc5("7"), &rc);
    assert!(e.prepare(401, rc5("7")).is_err());
    // The repository root itself, too.
    e.answer(&TargetReq::Ogg, &e.repo);
    assert!(e.prepare(401, TargetReq::Ogg).is_err());
    assert_eq!(e.settings().get().export_dir, None);
    e.repo_unchanged();
}

#[test]
fn ac_mp3_and_ogg_keep_the_source_rate_and_channels() {
    let e = Env::new();
    let out = e.folder("out");
    for (n, rate, ch, secs) in [(405u32, "48000", 2, 7.0), (404, "44100", 1, 6.5)] {
        for (t, ext, codec) in [
            (TargetReq::Mp3, "mp3", "mp3"),
            (TargetReq::Ogg, "ogg", "vorbis"),
        ] {
            let (plan, s) = e.export(n, t, &out);
            assert_eq!(s.phase, ExportPhase::Done, "{n} {ext} {s:?}");
            let v = probe_stream(Path::new(&plan.path));
            let st = &v["streams"][0];
            assert_eq!(st["codec_name"], codec);
            assert_eq!(st["sample_rate"], rate, "{n} {ext}");
            assert_eq!(st["channels"], ch, "{n} {ext}");
            assert_seconds(decode(Path::new(&plan.path)).0.len(), secs, 0.04);
        }
    }
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_encoder_settings_and_tags_reach_ffmpeg() {
    let e = Env::new();
    let log = e.home.join("argv.log");
    let fake = e.home.join("ffmpeg");
    script(
        &fake,
        &format!(
            "echo \"$*\" >> {}\nexec {} \"$@\"",
            log.display(),
            real_ffmpeg().display()
        ),
    );
    let f2 = fake.clone();
    let e = Env {
        export: ExportState::new(Box::new(move || Tools {
            ffmpeg: Some(f2.clone()),
            ..real_tools()
        })),
        ..e
    };
    let out = e.folder("out");
    for t in [TargetReq::Mp3, TargetReq::Ogg] {
        let (_, s) = e.export(401, t, &out);
        assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    }
    let log = std::fs::read_to_string(&log).unwrap();
    let enc = |name: &str| {
        log.lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("no {name} in {log}"))
            .to_string()
    };
    let mp3 = enc("libmp3lame");
    assert!(mp3.contains("-q:a 2"), "MP3 VBR quality 2: {mp3}");
    assert!(mp3.contains("-metadata title=Stem Mix") && mp3.contains("artist=The Example Band"));
    assert!(
        mp3.contains("album=Export Tests") && mp3.contains("track=3") && mp3.contains("date=2021")
    );
    let ogg = enc("libvorbis");
    assert!(ogg.contains("-q:a 6"), "Ogg quality 6: {ogg}");
    assert!(ogg.contains("track=3") && ogg.contains("date=2021"));
    // The mp3 is VBR: it carries a Xing header.
    let b = std::fs::read(out.join(format!("{BAND} - Stem Mix.mp3"))).unwrap();
    assert!(
        b.windows(4).take(2000).any(|w| w == b"Xing"),
        "no VBR (Xing) header"
    );
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_file_names_with_invalid_characters_and_an_empty_band() {
    let mut e = Env::new();
    let p = e.repo.join("tracks").join(id(401)).join("track.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["band"] = "Led<Zep>|\\x".into();
    v["title"] = "a/b\\c*?\"d:e".into();
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let p2 = e.repo.join("tracks").join(id(403)).join("track.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p2).unwrap()).unwrap();
    v["band"] = "".into();
    v["title"] = "Solo".into();
    std::fs::write(&p2, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    e.before = tree(&e.repo);
    let out = e.folder("out");
    let (plan, s) = e.export(401, TargetReq::Ogg, &out);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let f = Path::new(&plan.path);
    assert_eq!(
        f.parent().unwrap(),
        out,
        "the file stays in the chosen folder"
    );
    let name = f.file_name().unwrap().to_str().unwrap();
    assert!(!name.chars().any(|c| "/\\<>:\"|?*".contains(c)), "{name}");
    assert!(name.ends_with(".ogg") && name.contains(" - "));
    assert_eq!(
        tag(&probe(f), "title").as_deref(),
        Some("a/b\\c*?\"d:e"),
        "tags keep the real text"
    );
    let (plan, s) = e.export(403, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert_eq!(Path::new(&plan.path).file_name().unwrap(), "Solo.mp3");
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_mp3_overwrite_needs_confirmation_and_replaces() {
    let e = Env::new();
    let out = e.folder("out");
    let f = out.join(format!("{BAND} - Stem Mix.mp3"));
    std::fs::write(&f, b"old").unwrap();
    e.answer(&TargetReq::Mp3, &out);
    let plan = e.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    assert!(plan.exists && plan.path == f.display().to_string());
    assert!(e
        .start(&plan.token, false)
        .unwrap_err()
        .contains("already exists"));
    assert_eq!(std::fs::read(&f).unwrap(), b"old");
    // The refused token is spent or still valid; either way a fresh plan then Yes replaces it.
    e.answer(&TargetReq::Mp3, &out);
    let plan = e.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    e.start(&plan.token, true).unwrap();
    assert_eq!(e.wait().phase, ExportPhase::Done);
    assert_seconds(decode(&f).0.len(), 6.5, 0.03);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 1);
    // A used token cannot start a second export.
    assert!(e.start(&plan.token, true).is_err());
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_missing_tools_and_encoders_are_refused_before_the_dialog() {
    let e = Env::new();
    let out = e.folder("out");
    let base = e.rc5("rc5");
    // No ffmpeg at all.
    let none = Env {
        export: ExportState::new(Box::new(Tools::default)),
        ..e
    };
    for (t, d) in [
        (rc5("7"), &base),
        (TargetReq::Mp3, &out),
        (TargetReq::Ogg, &out),
    ] {
        none.answer(&t, d);
        let err = none.prepare(401, t).unwrap_err();
        assert!(err.contains("Install ffmpeg"), "{err}");
    }
    assert!(none.picker.starts().is_empty());
    // An ffmpeg without the encoders.
    let fake = none.home.join("ffmpeg");
    script(
        &fake,
        &format!(
            "case \"$*\" in *-encoders*) echo ' A..... aac AAC';; *) exec {} \"$@\";; esac",
            real_ffmpeg().display()
        ),
    );
    let f2 = fake.clone();
    let e = Env {
        export: ExportState::new(Box::new(move || Tools {
            ffmpeg: Some(f2.clone()),
            ..real_tools()
        })),
        ..none
    };
    let err = e.prepare(401, TargetReq::Mp3).unwrap_err();
    assert!(
        err.contains("MP3 encoder") && err.contains("libmp3lame"),
        "{err}"
    );
    let err = e.prepare(401, TargetReq::Ogg).unwrap_err();
    assert!(err.contains("libvorbis"), "{err}");
    assert!(
        e.picker.starts().is_empty(),
        "the user is not asked for a folder for nothing"
    );
    // The RC-5 export needs no encoder.
    let (_, s) = e.export(401, rc5("7"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_failing_encoder_fails_with_its_reason_and_leaves_nothing() {
    let e = Env::new();
    let fake = e.home.join("ffmpeg");
    script(&fake, &format!("case \"$*\" in *pipe:0*) head -c 200000 >/dev/null; echo 'disk is full' >&2; exit 1;; esac\nexec {} \"$@\"", real_ffmpeg().display()));
    let f2 = fake.clone();
    let e = Env {
        export: ExportState::new(Box::new(move || Tools {
            ffmpeg: Some(f2.clone()),
            ..real_tools()
        })),
        ..e
    };
    let out = e.folder("out");
    std::fs::write(out.join("keep.txt"), b"mine").unwrap();
    let before = tree(&out);
    let (_, s) = e.export(401, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Failed);
    assert!(s.message.unwrap_or_default().contains("disk is full"));
    assert!(tree(&out) == before, "no partial file");
    e.repo_unchanged();
}

#[test]
fn ac_cancelled_mp3_leaves_nothing_and_a_second_export_is_refused_meanwhile() {
    let e = Env::with(ExportState::new(Box::new(real_tools)).with_pace(5.0));
    let out = e.folder("out");
    std::fs::write(out.join("keep.txt"), b"mine").unwrap();
    let before = tree(&out);
    e.answer(&TargetReq::Mp3, &out);
    let plan = e.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    let snap = e.start(&plan.token, false).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        tree(&out) != before,
        "a part file exists while the export runs"
    );
    e.answer(&TargetReq::Ogg, &out);
    assert_eq!(
        e.prepare(403, TargetReq::Ogg).unwrap_err(),
        "An export is running"
    );
    assert!(e.export.cancel(&snap.job).is_some());
    assert_eq!(e.wait().phase, ExportPhase::Cancelled);
    assert!(tree(&out) == before, "nothing is left but what was there");
    e.repo_unchanged();
}

#[test]
fn ac_long_intro_is_kept_and_a_stem_or_other_files_are_never_exported() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let (_, s) = e.export(407, rc5("7"), &base);
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    let (_, l, _) = read_wav(&wav_path(&base, 7));
    let (lead, first) = (132300usize, 132300usize);
    assert_eq!(l.len(), lead + 264600);
    // The original's 3 s of silence and its tone are where the original put them.
    let tone = |a: usize| goertzel(&l[a..a + 20000], 440.0);
    assert!(tone(lead + first + 5000) > 20.0 * tone(lead + first - 30000).max(1e-6));
    assert!(
        tone(lead + first + 5000) > 0.02,
        "{}",
        tone(lead + first + 5000)
    );
    // 401 holds a guitar stem; the export is the backing (330/550 Hz), not the stem.
    let (_, s) = e.export(401, rc5("8"), &base);
    assert_eq!(s.phase, ExportPhase::Done);
    let (_, l, r) = read_wav(&wav_path(&base, 8));
    let w = 110250 + 5000..110250 + 25000;
    assert!(goertzel(&l[w.clone()], 330.0) > 5.0 * goertzel(&l[w.clone()], 550.0));
    assert!(goertzel(&r[w.clone()], 550.0) > 5.0 * goertzel(&r[w], 330.0));
    e.repo_unchanged_except_exports();
}

// ---- the export info (specs/gui-export-info.md) ----

fn exports_of(e: &Env, n: u32) -> calliope_gui::export_record::TrackExports {
    Repository::new(&e.repo)
        .load_record(&id(n))
        .unwrap()
        .exports
}

fn track_json(e: &Env, n: u32) -> std::path::PathBuf {
    e.repo.join("tracks").join(id(n)).join("track.json")
}

/// Writes `exports` into a track's `track.json` by hand and takes that as the new baseline.
fn write_exports(e: &mut Env, n: u32, exports: serde_json::Value) {
    let p = track_json(e, n);
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["exports"] = exports;
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
    e.before = tree(&e.repo);
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))
}

const STALE: &str = "2020-01-01T00:00:00Z";

#[test]
fn ac_info_never_exported_has_none() {
    let e = Env::new();
    for n in [401, 403, 404, 405] {
        assert_eq!(exports_of(&e, n), Default::default(), "{n}");
        assert_eq!(e.info(n).last, Default::default());
    }
    assert_eq!(e.info(401).memory, 1);
}

#[test]
fn ac_info_rc5_recorded() {
    let e = Env::new();
    let base = e.rc5("rc5");
    let (_, s) = e.export(401, rc5("7"), &base);
    assert_eq!((s.phase, s.info_error), (ExportPhase::Done, None));
    let x = exports_of(&e, 401);
    let r = x.rc5.clone().unwrap();
    assert_eq!(r.memory, 7);
    assert!(
        r.at.len() == 20 && r.at.ends_with('Z'),
        "an RFC 3339 UTC time: {}",
        r.at
    );
    assert!(x.mp3.is_none() && x.ogg.is_none());
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_info_memory_from_record() {
    let e = Env::new();
    let base = e.rc5("rc5");
    e.export(401, rc5("7"), &base);
    // The exported track shows its own memory, another one the next after the last used.
    assert_eq!(e.info(401).memory, 7);
    assert_eq!(e.info(401).last.rc5.unwrap().memory, 7);
    assert_eq!(e.info(403).memory, 8);
}

#[test]
fn ac_info_rc5_reexport_same_memory() {
    let mut e = Env::new();
    write_exports(
        &mut e,
        401,
        serde_json::json!({"rc5": {"memory": 7, "at": STALE}}),
    );
    let base = e.rc5("rc5");
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    assert!(plan.exists || !wav_path(&base, 7).exists());
    assert!(plan.path.ends_with("ROLAND/WAVE/007_1/007_1.WAV"));
    e.start(&plan.token, plan.exists).unwrap();
    assert_eq!(e.wait().phase, ExportPhase::Done);
    // The same export again: the file exists now, the confirmation names it, Yes replaces it.
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    assert!(plan.exists && plan.path.ends_with("ROLAND/WAVE/007_1/007_1.WAV"));
    write_exports(
        &mut e,
        401,
        serde_json::json!({"rc5": {"memory": 7, "at": STALE}}),
    );
    e.start(&plan.token, true).unwrap();
    assert_eq!(e.wait().phase, ExportPhase::Done);
    let r = exports_of(&e, 401).rc5.unwrap();
    assert_eq!(r.memory, 7);
    assert_ne!(r.at, STALE, "the time was updated");
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_info_rc5_other_memory() {
    let mut e = Env::new();
    write_exports(
        &mut e,
        401,
        serde_json::json!({"rc5": {"memory": 7, "at": STALE}}),
    );
    let base = e.rc5("rc5");
    let (_, s) = e.export(401, rc5("12"), &base);
    assert_eq!(s.phase, ExportPhase::Done);
    let r = exports_of(&e, 401).rc5.unwrap();
    assert_eq!(r.memory, 12);
    assert_ne!(r.at, STALE);
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_info_rc5_clears_previous_holder() {
    let mut e = Env::new();
    write_exports(
        &mut e,
        401,
        serde_json::json!({
            "rc5": {"memory": 7, "at": STALE},
            "mp3": {"path": "/music/a.mp3", "at": STALE},
            "ogg": {"path": "/music/a.ogg", "at": STALE}
        }),
    );
    let base = e.rc5("rc5");
    let (_, s) = e.export(403, rc5("7"), &base);
    assert_eq!((s.phase, s.info_error), (ExportPhase::Done, None));
    assert_eq!(exports_of(&e, 403).rc5.unwrap().memory, 7);
    let a = exports_of(&e, 401);
    assert!(a.rc5.is_none(), "A lost its RC-5 entry");
    assert_eq!(a.mp3.unwrap().path, "/music/a.mp3");
    assert_eq!(a.ogg.unwrap().path, "/music/a.ogg");
    // A's other fields are untouched, `modified` included.
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(track_json(&e, 401)).unwrap()).unwrap();
    assert_eq!(j["modified"], "2026-10-23T10:00:00Z");
    assert!(j["exports"].get("rc5").is_none());
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_info_file_paths() {
    let e = Env::new();
    let d = e.folder("music");
    let (_, s) = e.export(401, TargetReq::Mp3, &d);
    assert_eq!((s.phase, s.info_error), (ExportPhase::Done, None));
    let x = exports_of(&e, 401);
    assert_eq!(
        x.mp3.unwrap().path,
        d.join(format!("{BAND} - Stem Mix.mp3"))
            .display()
            .to_string()
    );
    assert!(x.ogg.is_none() && x.rc5.is_none());
    let d2 = e.folder("music2");
    e.export(401, TargetReq::Ogg, &d2);
    let x = exports_of(&e, 401);
    assert_eq!(
        x.ogg.clone().unwrap().path,
        d2.join(format!("{BAND} - Stem Mix.ogg"))
            .display()
            .to_string()
    );
    assert!(x
        .mp3
        .clone()
        .unwrap()
        .path
        .starts_with(&d.display().to_string()));
    assert_eq!(e.info(401).last, x);
    e.repo_unchanged_except_exports();
}

#[test]
fn ac_info_mp3_dialog_starts_in_recorded_folder() {
    let e = Env::new();
    let d = e.folder("music");
    let other = e.folder("other");
    e.export(401, TargetReq::Mp3, &d);
    e.export(403, TargetReq::Mp3, &other); // the remembered folder is `other` now
    let start_of = |n: u32, t: TargetReq, answer: &Path| {
        e.answer(&t, answer);
        e.prepare(n, t).unwrap().unwrap();
        e.picker.starts().last().unwrap().1.clone()
    };
    assert_eq!(start_of(401, TargetReq::Mp3, &other), Some(d.clone()));
    // OGG has no entry: it starts in the remembered folder (the last pick, `other`).
    assert_eq!(start_of(401, TargetReq::Ogg, &d), Some(other.clone()));
    // Both the recorded and the remembered folder are gone: home.
    let gone = e.folder("gone");
    e.export(404, TargetReq::Mp3, &gone);
    std::fs::remove_dir_all(&gone).unwrap();
    assert_eq!(start_of(404, TargetReq::Mp3, &other), Some(e.home.clone()));
}

#[test]
fn ac_info_nothing_recorded_on_cancel_refusal_failure() {
    let entry = serde_json::json!({
        "rc5": {"memory": 7, "at": STALE},
        "mp3": {"path": "/music/a.mp3", "at": STALE}
    });
    // Cancelled.
    let mut e = Env::with(ExportState::new(Box::new(real_tools)).with_pace(5.0));
    write_exports(&mut e, 401, entry.clone());
    let base = e.rc5("rc5");
    e.answer(&rc5("9"), &base);
    let plan = e.prepare(401, rc5("9")).unwrap().unwrap();
    let snap = e.start(&plan.token, false).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    e.export.cancel(&snap.job);
    assert_eq!(e.wait().phase, ExportPhase::Cancelled);
    e.repo_unchanged();
    // Refused: no ROLAND folder, a bad memory.
    let e2 = Env::new();
    let plain = e2.folder("plain");
    e2.answer(&rc5("9"), &plain);
    assert!(e2.prepare(401, rc5("9")).is_err());
    assert!(e2.prepare(401, rc5("100")).is_err());
    e2.repo_unchanged();
    // Overwrite declined: the plan is made, `start` is never called.
    let mut e3 = Env::new();
    write_exports(&mut e3, 401, entry.clone());
    let b3 = e3.rc5("rc5");
    std::fs::create_dir_all(wav_path(&b3, 9).parent().unwrap()).unwrap();
    std::fs::write(wav_path(&b3, 9), b"old").unwrap();
    e3.answer(&rc5("9"), &b3);
    assert!(e3.prepare(401, rc5("9")).unwrap().unwrap().exists);
    e3.repo_unchanged();
    // Failed: the encoder dies.
    let e4 = Env::new();
    let fake = e4.home.join("ffmpeg");
    script(
        &fake,
        &format!(
            "case \"$*\" in *pipe:0*) head -c 200000 >/dev/null; echo 'disk is full' >&2; exit 1;; esac\nexec {} \"$@\"",
            real_ffmpeg().display()
        ),
    );
    let mut e4 = Env {
        export: ExportState::new(Box::new(move || Tools {
            ffmpeg: Some(fake.clone()),
            ..real_tools()
        })),
        ..e4
    };
    write_exports(&mut e4, 401, entry);
    let out = e4.folder("out");
    let (_, s) = e4.export(401, TargetReq::Mp3, &out);
    assert_eq!(s.phase, ExportPhase::Failed);
    e4.repo_unchanged();
}

#[test]
fn ac_info_kept_by_library_save() {
    let e = Env::new();
    let d = e.folder("music");
    let repo = Repository::new(&e.repo);
    let rev = repo.load_record(&id(401)).unwrap().revision;
    e.export(401, TargetReq::Mp3, &d);
    let rec = repo.load_record(&id(401)).unwrap();
    assert_eq!(rec.revision, rev, "an export doesn't change the revision");
    let before = rec.exports.clone();
    let edits = calliope_gui::track_meta::TrackEdits {
        band: rec.band.clone(),
        album: rec.album.clone(),
        title: rec.title.clone(),
        composers: rec.composers.clone(),
        year: Some(2024),
        source_url: None,
        copyright: None,
    };
    // A save made from the record loaded before the export still goes through.
    let saved = repo
        .save_track(
            SaveTrackRequest {
                id: id(401),
                revision: rev,
                edits,
                tablatures: vec![],
                position: None,
                album_position: None,
                group: vec![],
            },
            &|_| None,
        )
        .unwrap();
    assert_eq!(saved.track.year, Some(2024));
    assert_eq!(saved.track.exports, before);
    assert_eq!(exports_of(&e, 401), before);
}

#[test]
fn ac_info_not_saved_is_reported() {
    if is_root() {
        eprintln!("skipped: running as root");
        return;
    }
    let mut e = Env::new();
    write_exports(
        &mut e,
        403,
        serde_json::json!({"rc5": {"memory": 7, "at": STALE}}),
    );
    let base = e.rc5("rc5");
    e.answer(&rc5("7"), &base);
    let plan = e.prepare(401, rc5("7")).unwrap().unwrap();
    let dir = e.repo.join("tracks").join(id(401));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    e.start(&plan.token, false).unwrap();
    let s = e.wait();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert!(wav_path(&base, 7).is_file(), "the file is exported");
    let m = s.info_error.expect("the problem is reported");
    assert!(m.starts_with("The export info wasn't saved: "), "{m}");
    assert!(exports_of(&e, 401).rc5.is_none());
    // The other holder was still cleared.
    assert!(exports_of(&e, 403).rc5.is_none());
}

#[test]
fn ac_info_older_builds_keep_the_info() {
    use calliope_gui::track_meta::{apply_edits, parse, to_json_pretty, TrackEdits};
    let mut e = Env::new();
    write_exports(
        &mut e,
        401,
        serde_json::json!({"rc5": {"memory": 7, "at": STALE}, "future": [1, 2]}),
    );
    let bytes = std::fs::read(track_json(&e, 401)).unwrap();
    let mut m = parse(&bytes, &id(401)).unwrap();
    let edits = TrackEdits {
        band: m.band.clone(),
        album: m.album.clone(),
        title: "Renamed".into(),
        composers: m.composers.clone(),
        year: m.year,
        source_url: None,
        copyright: None,
    };
    apply_edits(&mut m, edits).unwrap();
    let back: serde_json::Value = serde_json::from_str(&to_json_pretty(&m)).unwrap();
    let orig: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back["exports"], orig["exports"]);
    assert_eq!(back["title"], "Renamed");
}

// ---- QA additions: the cleared holder, failures with a holder, start folders, MP3 info error ----

#[test]
fn ac_info_cleared_holder_keeps_revision_fields_and_survives_a_stale_save() {
    let mut e = Env::new();
    write_exports(
        &mut e,
        401,
        serde_json::json!({
            "rc5": {"memory": 7, "at": STALE},
            "mp3": {"path": "/music/a.mp3", "at": STALE}
        }),
    );
    let repo = Repository::new(&e.repo);
    let a = repo.load_record(&id(401)).unwrap();
    let base = e.rc5("rc5");
    e.export(403, rc5("7"), &base);
    let now = repo.load_record(&id(401)).unwrap();
    assert!(now.exports.rc5.is_none());
    assert_eq!(now.exports.mp3, a.exports.mp3);
    assert_eq!(
        now.revision, a.revision,
        "no conflict for an edit open on A"
    );
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(track_json(&e, 401)).unwrap()).unwrap();
    assert_eq!(j["schema_version"], 2);
    assert_eq!(j["modified"], "2026-10-23T10:00:00Z");
    // A save from the stale draft goes through and A stays cleared.
    let edits = calliope_gui::track_meta::TrackEdits {
        band: a.band.clone(),
        album: a.album.clone(),
        title: "Edited".into(),
        composers: a.composers.clone(),
        year: a.year,
        source_url: None,
        copyright: None,
    };
    let saved = repo
        .save_track(
            SaveTrackRequest {
                id: id(401),
                revision: a.revision.clone(),
                edits,
                tablatures: vec![],
                position: None,
                album_position: None,
                group: vec![],
            },
            &|_| None,
        )
        .unwrap();
    assert_eq!(saved.track.title, "Edited");
    assert!(saved.track.exports.rc5.is_none());
    assert_eq!(saved.track.exports.mp3, a.exports.mp3);
}

#[test]
fn ac_info_cancel_and_failure_leave_the_other_holder_alone() {
    let hold = serde_json::json!({"rc5": {"memory": 9, "at": STALE}});
    let mut e = Env::with(ExportState::new(Box::new(real_tools)).with_pace(5.0));
    write_exports(&mut e, 403, hold.clone());
    let base = e.rc5("rc5");
    e.answer(&rc5("9"), &base);
    let plan = e.prepare(401, rc5("9")).unwrap().unwrap();
    let snap = e.start(&plan.token, false).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    e.export.cancel(&snap.job);
    assert_eq!(e.wait().phase, ExportPhase::Cancelled);
    e.repo_unchanged();
    assert_eq!(exports_of(&e, 403).rc5.unwrap().memory, 9);
    // Declined overwrite.
    let mut e3 = Env::new();
    write_exports(&mut e3, 403, hold);
    let b3 = e3.rc5("rc5");
    std::fs::create_dir_all(wav_path(&b3, 9).parent().unwrap()).unwrap();
    std::fs::write(wav_path(&b3, 9), b"old").unwrap();
    e3.answer(&rc5("9"), &b3);
    assert!(e3.prepare(401, rc5("9")).unwrap().unwrap().exists);
    e3.repo_unchanged();
}

#[test]
fn ac_info_ogg_dialog_start_and_fallback_to_the_remembered_folder() {
    let e = Env::new();
    let d = e.folder("oggs");
    let other = e.folder("other");
    e.export(401, TargetReq::Ogg, &d);
    e.export(403, TargetReq::Mp3, &other); // remembered export folder: other
    let start_of = |n: u32, t: TargetReq, answer: &Path| {
        e.answer(&t, answer);
        e.prepare(n, t).unwrap().unwrap();
        e.picker.starts().last().unwrap().1.clone()
    };
    assert_eq!(start_of(401, TargetReq::Ogg, &other), Some(d.clone()));
    // The MP3 kind of 401 has no entry: not the OGG folder.
    assert_eq!(start_of(401, TargetReq::Mp3, &other), Some(other.clone()));
    // The recorded folder is gone but the remembered one exists.
    std::fs::remove_dir_all(&d).unwrap();
    assert_eq!(start_of(401, TargetReq::Ogg, &other), Some(other.clone()));
}

#[test]
fn ac_info_mp3_not_saved_is_reported_and_the_file_is_written() {
    if is_root() {
        eprintln!("skipped: running as root");
        return;
    }
    let e = Env::new();
    let d = e.folder("music");
    e.answer(&TargetReq::Mp3, &d);
    let plan = e.prepare(401, TargetReq::Mp3).unwrap().unwrap();
    let dir = e.repo.join("tracks").join(id(401));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    e.start(&plan.token, false).unwrap();
    let s = e.wait();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(s.phase, ExportPhase::Done, "{s:?}");
    assert!(d.join(format!("{BAND} - Stem Mix.mp3")).is_file());
    assert!(s
        .info_error
        .unwrap()
        .starts_with("The export info wasn't saved: "));
    assert!(exports_of(&e, 401).mp3.is_none());
    e.repo_unchanged_except_exports();
}
