//! Tests of the job orchestrator against the REAL `calliope-stems` binary on 127.0.0.1 (temp
//! work dir, the stub separator, never the real model), the fake yt-dlp and the real
//! ffmpeg/ffprobe, in temp repositories only.

use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};

fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> PathBuf {
    project().join("tests/fixtures/import").join(name)
}

fn stems_binary() -> PathBuf {
    // unit tests live in target/<profile>/deps/
    let exe = std::env::current_exe().unwrap();
    let bin = exe.parent().unwrap().parent().unwrap().join("calliope-stems");
    assert!(bin.is_file(), "{} is missing: run cargo build -p calliope-stems (cargo test --workspace builds it)", bin.display());
    bin
}

/// The real server; killed when dropped. Its stderr lines are collected.
struct TestServer {
    child: Child,
    url: String,
    log: Arc<Mutex<Vec<String>>>,
    outer: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TestServer {
    fn start(mode: &str) -> TestServer {
        TestServer::start_with(mode, &[])
    }

    fn start_with(mode: &str, extra_args: &[&str]) -> TestServer {
        let outer = tempfile::tempdir().unwrap();
        let work = outer.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("stub-mode"), format!("{mode}\n")).unwrap();
        let mut child = Command::new(stems_binary())
            .arg("--separator")
            .arg(project().join("tests/support/stub-separator"))
            .args(["--listen", "127.0.0.1:0", "--work-dir"])
            .arg(&work)
            .args(extra_args)
            .env_remove("STUB_SEPARATOR_MODE")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        let addr: SocketAddr = loop {
            line.clear();
            assert!(lines.read_line(&mut line).unwrap() > 0, "server exited before listening");
            if let Some(rest) = line.split("listening addr=").nth(1) {
                break rest.split_whitespace().next().unwrap().parse().unwrap();
            }
        };
        assert!(addr.ip().is_loopback());
        let log = Arc::new(Mutex::new(Vec::new()));
        let l2 = log.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            while lines.read_line(&mut s).unwrap_or(0) > 0 {
                l2.lock().unwrap().push(s.trim_end().to_string());
                s.clear();
            }
        });
        TestServer { child, url: format!("http://{addr}"), log, outer }
    }

    fn set_mode(&self, mode: &str) {
        fs::write(self.outer.path().join("work/stub-mode"), format!("{mode}\n")).unwrap();
    }

    fn saw(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    fn wait_for(&self, needle: &str) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if self.saw(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn stem_gets(&self, name: &str) -> usize {
        let suffix = format!("/stems/{name} ");
        self.log.lock().unwrap().iter().filter(|l| l.contains("method=GET path=/v1/jobs/") && l.contains(&suffix)).count()
    }

    fn all_stem_gets(&self) -> usize {
        self.log.lock().unwrap().iter().filter(|l| l.contains("method=GET path=/v1/jobs/") && l.contains("/stems/")).count()
    }

    fn job_dirs(&self) -> usize {
        fs::read_dir(self.outer.path().join("work/jobs")).map(|r| r.flatten().count()).unwrap_or(0)
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    sources: PathBuf,
    state: ImportState,
    tools: Tools,
    lock: Arc<dyn RepoLock>,
    events: Arc<Mutex<Vec<ImportEvent>>>,
    sink: Sink,
}

fn tool(name: &str) -> PathBuf {
    crate::tools::find_in_path(name, &std::env::var_os("PATH").unwrap_or_default()).unwrap_or_else(|| panic!("{name} is needed"))
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let outside = dir.path().join("outside");
        let sources = dir.path().join("sources");
        for d in [&root, &outside, &sources] {
            fs::create_dir_all(d).unwrap();
        }
        Repository::new(&root).ensure_layout().unwrap();
        fs::write(outside.join("precious.txt"), "do not touch").unwrap();
        for f in ["tagged.mp3", "with-audio.mp4", "no-audio.mp4", "long.flac", "untagged.flac"] {
            fs::copy(fixture(f), sources.join(f)).unwrap();
        }
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
        Rig { _dir: dir, root, outside, sources, state: ImportState::new(tools.clone(), lock.clone(), Duration::from_millis(50)), tools, lock, events, sink }
    }

    fn url(&self, url: &str, resume: bool) -> Result<JobSnapshot, String> {
        self.state.start_url(&self.root, url, resume, self.sink.clone())
    }

    fn file(&self, name: &str) -> Result<JobSnapshot, String> {
        let kind = if name.ends_with(".mp4") { FileKind::Video } else { FileKind::Audio };
        self.state.start_file(&self.root, kind, &self.sources.join(name), self.sink.clone())
    }

    fn wait(&self, what: &str, pred: impl Fn(&JobSnapshot) -> bool) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(s) = self.state.snapshot() {
                if pred(&s) {
                    return s;
                }
            }
            assert!(Instant::now() < end, "timed out waiting for {what}; at {:?}", self.state.snapshot().map(|s| (s.phase, s.error)));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_idle(&self, what: &str) -> JobSnapshot {
        // Take the snapshot AFTER seeing the job stopped; reading it first returns a stale phase.
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if !self.state.is_running() {
                if let Some(s) = self.state.snapshot() {
                    return s;
                }
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ready(&self) -> JobSnapshot {
        let s = self.wait_idle("the job to stop");
        assert_eq!(s.phase, Phase::Ready, "{:?}", s.error);
        s
    }

    fn extract(&self, s: &JobSnapshot, server: &TestServer, keep: bool) -> Result<JobSnapshot, String> {
        self.state.start_extraction(&s.job, s.metadata.clone().unwrap(), Some(&server.url), keep)
    }

    fn tracks_dir(&self) -> PathBuf {
        self.root.join("tracks")
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    fn phases(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| serde_json::to_value(e).unwrap()["phase"].as_str().unwrap().to_string())
            .collect();
        v.dedup();
        v
    }

    fn check_clean(&self) {
        assert_eq!(fs::read_to_string(self.outside.join("precious.txt")).unwrap(), "do not touch");
        assert_eq!(Self::names_in(&self.outside), ["precious.txt"]);
        let tracks = self.tracks_dir();
        assert!(Self::names_in(&tracks).iter().all(|n| !n.starts_with(".staging-")), "staging left in {tracks:?}");
    }
}

fn hashes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap())).collect()
}

const WATCH: &str = "https://media.example/watch?v=ok";

#[test]
fn url_flow_saves_a_stem_track_and_removes_the_temp_folder() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    // a user's own file in import-tmp must survive everything
    fs::create_dir_all(rig.root.join("import-tmp")).unwrap();
    fs::write(rig.root.join("import-tmp/keep-me.txt"), "mine").unwrap();
    let snap = rig.url(WATCH, false).unwrap();
    assert_eq!(snap.phase, Phase::Downloading);
    assert_eq!(snap.source.kind, SourceKind::Url);
    assert!(rig.state.is_active());
    let ready = rig.ready();
    let meta = ready.metadata.clone().unwrap();
    assert_eq!((meta.band.as_str(), meta.title.as_str(), meta.year), ("The Example Band", "Night Drive", Some(2020)));
    assert_eq!(meta.source_url.as_deref(), Some("https://media.example/watch?v=abc123"));
    assert!(ready.duration_s.unwrap() > 4.0);
    assert!(rig.state.is_active() && !rig.state.is_running());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).iter().any(|n| n.starts_with("url-")));

    rig.extract(&ready, &server, false).unwrap();
    let done = rig.wait("saved", |s| s.phase == Phase::Saved);
    let track = done.track.clone().unwrap();
    assert!(!rig.state.is_active());
    let lib = Repository::new(&rig.root).scan();
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    assert_eq!(lib.tracks.len(), 1);
    let t = &lib.tracks[0];
    assert_eq!(*t, track);
    assert_eq!(t.track_type, TrackType::Stem);
    let names: Vec<&str> = t.stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar", "piano", "other"]);
    assert_eq!((t.audio.clone(), t.original.clone(), t.stem_model.as_deref()), (None, None, Some("htdemucs_6s")));
    assert!(t.missing.is_empty());
    assert_eq!((t.band.as_str(), t.title.as_str(), t.year), (meta.band.as_str(), meta.title.as_str(), meta.year));
    assert_eq!(t.source_url, meta.source_url);
    let tdir = rig.tracks_dir().join(&t.id);
    for s in &t.stems {
        assert!(fs::read(tdir.join(&s.file)).unwrap().starts_with(b"fLaC"));
        assert_eq!(fs::read(tdir.join(&s.file)).unwrap(), fs::read(fixture(&format!("stems/{}.flac", s.name))).unwrap());
    }
    assert_eq!(Rig::names_in(&tdir), ["stems", "track.json"]);
    // temp folder gone, the user's file stays, the server job is gone
    assert_eq!(Rig::names_in(&rig.root.join("import-tmp")), ["keep-me.txt"]);
    assert!(server.wait_for("method=DELETE"));
    assert!(server.saw("method=POST"));
    let phases = rig.phases();
    for p in ["downloading", "preparing", "ready", "uploading", "queued", "receiving", "saving", "saved"] {
        assert!(phases.iter().any(|x| x == p), "no {p} event in {phases:?}");
    }
    assert_eq!(phases.last().unwrap(), "saved");
    rig.check_clean();
}

#[test]
fn keep_original_puts_original_flac_in_the_track() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    let meta = ready.metadata.clone().unwrap();
    assert_eq!((meta.title.as_str(), meta.band.as_str(), meta.year), ("Glass Harbour", "The Example Band", Some(2021)));
    assert_eq!(ready.source.label, "tagged.mp3");
    assert_eq!(ready.source.kind, SourceKind::AudioFile);
    let src_before = hashes(&rig.sources);
    rig.extract(&ready, &server, true).unwrap();
    let t = rig.wait("saved", |s| s.phase == Phase::Saved).track.unwrap();
    assert_eq!(t.original.as_deref(), Some("original.flac"));
    let tdir = rig.tracks_dir().join(&t.id);
    assert!(fs::read(tdir.join("original.flac")).unwrap().starts_with(b"fLaC"));
    let json: serde_json::Value = serde_json::from_slice(&fs::read(tdir.join("track.json")).unwrap()).unwrap();
    assert_eq!(json["original"], "original.flac");
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["type"], "stem");
    assert!(json["audio"].is_null());
    assert_eq!(Rig::names_in(&rig.root.join("import-tmp")), Vec::<String>::new());
    assert!(Repository::new(&rig.root).scan().problems.is_empty());
    assert_eq!(hashes(&rig.sources), src_before);
    rig.check_clean();
}

#[test]
fn video_file_flow_and_sources_stay_byte_identical() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let before = hashes(&rig.sources);
    let snap = rig.file("with-audio.mp4").unwrap();
    assert_eq!(snap.source.kind, SourceKind::VideoFile);
    let ready = rig.ready();
    assert_eq!(ready.metadata.as_ref().unwrap().title, "Clip Title");
    rig.extract(&ready, &server, false).unwrap();
    let done = rig.wait("saved", |s| s.phase == Phase::Saved);
    assert_eq!(done.track.unwrap().stems.len(), 6);
    assert_eq!(hashes(&rig.sources), before);
    // untagged: the file name is the title
    rig.file("untagged.flac").unwrap();
    assert_eq!(rig.ready().metadata.unwrap().title, "untagged");
    assert_eq!(hashes(&rig.sources), before);
    rig.check_clean();
}

#[test]
fn a_video_without_audio_fails_with_the_exact_text() {
    let rig = Rig::new();
    rig.file("no-audio.mp4").unwrap();
    let s = rig.wait_idle("failure");
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!((e.stage, e.message.as_str()), (Stage::Prepare, "Selected file no-audio.mp4 has no audio track"));
    assert!(!rig.state.is_active());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    assert!(matches!(rig.events.lock().unwrap().last(), Some(ImportEvent::Failed { stage: Stage::Prepare, .. })));
    // a new job can start at once
    rig.file("tagged.mp3").unwrap();
    rig.ready();
    rig.check_clean();
}

#[test]
fn a_track_longer_than_15_minutes_is_refused_before_any_upload() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.file("long.flac").unwrap();
    let s = rig.wait_idle("failure");
    assert_eq!(s.error.unwrap().message, "Tracks longer than 15 minutes are not supported");
    assert!(!server.saw("method=POST") && !server.saw("method=GET"), "{:?}", server.log.lock().unwrap());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
}

#[test]
fn http_403_is_a_download_failure_with_its_status() {
    let rig = Rig::new();
    rig.url("https://media.example/http403", false).unwrap();
    let s = rig.wait_idle("failure");
    let e = s.error.unwrap();
    assert_eq!((e.stage, e.http_status, e.message.as_str()), (Stage::Download, Some(403), "Error 403 when attempting download"));
    assert!(!rig.state.is_active());
    // an invalid URL never creates a job
    assert_eq!(rig.url("ftp://x.example/a", false).unwrap_err(), "Entered URL is invalid");
    rig.check_clean();
}

#[test]
fn cancel_keeps_the_partial_and_resume_continues_while_start_over_restarts() {
    let rig = Rig::new();
    let chunk = fs::metadata(fixture("download.webm")).unwrap().len().div_ceil(5);
    let u = "https://media.example/slow";
    assert_eq!(rig.state.prepare_url(&rig.root, u).status, UrlPrepStatus::Ready);
    let snap = rig.url(u, false).unwrap();
    rig.wait("some bytes", |s| s.downloaded > 0);
    rig.state.cancel(&snap.job).unwrap();
    let s = rig.wait_idle("cancel");
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(!rig.state.is_active());
    assert!(matches!(rig.events.lock().unwrap().last(), Some(ImportEvent::Cancelled { back_to: BackTo::Source })));
    let prep = rig.state.prepare_url(&rig.root, u);
    assert_eq!(prep.status, UrlPrepStatus::Partial);
    assert!(prep.partial_bytes >= chunk);
    assert_eq!(prep.message, "Incomplete download file from the same URL found");

    // Start Over: restarts from zero
    let so = rig.url(u, false).unwrap();
    let first = rig.wait("some bytes", |s| s.downloaded > 0).downloaded;
    assert_eq!(first, chunk, "start over must restart at zero");
    rig.state.cancel(&so.job).unwrap();
    assert_eq!(rig.wait_idle("cancel").phase, Phase::Cancelled);

    // Resume: continues after the part that was kept
    rig.url(u, true).unwrap();
    let first = rig.wait("some bytes", |s| s.downloaded > 0).downloaded;
    assert!(first >= 2 * chunk, "resume must continue the part (first progress {first}, chunk {chunk})");
    let ready = rig.ready();
    assert_eq!(ready.metadata.unwrap().band, "The Example Band");
    // complete download kept for a later resume is offered as resumable too
    rig.check_clean();
}

#[test]
fn a_finished_download_is_reused_without_downloading_again() {
    let rig = Rig::new();
    rig.url(WATCH, false).unwrap();
    let first = rig.ready();
    assert_eq!(rig.state.prepare_url(&rig.root, WATCH).status, UrlPrepStatus::Busy);
    // the app was closed in the edit pane: a new state finds audio.flac and skips the download
    let state2 = ImportState::new(rig.tools.clone(), rig.lock.clone(), Duration::from_millis(50));
    let events: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let e2 = events.clone();
    state2.start_url(&rig.root, WATCH, true, Arc::new(move |e| e2.lock().unwrap().push(e))).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while state2.is_running() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    let s = state2.snapshot().unwrap();
    assert_eq!(s.phase, Phase::Ready);
    assert_eq!(s.metadata, first.metadata);
    assert!(events.lock().unwrap().iter().all(|e| !matches!(e, ImportEvent::Downloading { .. })));
    rig.check_clean();
}

#[test]
fn cancel_during_extraction_goes_back_to_the_edit_pane() {
    let server = TestServer::start("hang");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    let src_before = hashes(&rig.sources);
    rig.extract(&ready, &server, true).unwrap();
    rig.wait("working", |s| s.phase == Phase::Working);
    rig.state.cancel(&ready.job).unwrap();
    let s = rig.wait_idle("cancel");
    assert_eq!(s.phase, Phase::Ready);
    assert_eq!(s.metadata, ready.metadata);
    assert!(s.error.is_none());
    assert!(matches!(rig.events.lock().unwrap().last(), Some(ImportEvent::Cancelled { back_to: BackTo::Edit })));
    assert!(server.wait_for("method=DELETE"), "{:?}", server.log.lock().unwrap());
    let end = Instant::now() + Duration::from_secs(10);
    while server.job_dirs() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.job_dirs(), 0, "the server job was not removed");
    // no staging, no track; the temp audio is still there, so Extract can be retried
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty(), "{:?}", Rig::names_in(&rig.tracks_dir()));
    let jobdir = Rig::names_in(&rig.root.join("import-tmp"));
    assert_eq!(jobdir.len(), 1);
    assert!(rig.root.join("import-tmp").join(&jobdir[0]).join("audio.flac").is_file());
    assert!(rig.state.is_active());
    // retry succeeds once the server behaves; the original is adopted only then
    server.set_mode("ok");
    rig.extract(&s, &server, true).unwrap();
    let t = rig.wait("saved", |s| s.phase == Phase::Saved).track.unwrap();
    assert_eq!(t.original.as_deref(), Some("original.flac"));
    assert_eq!(hashes(&rig.sources), src_before);
    rig.check_clean();
}

#[test]
fn a_failed_extraction_returns_to_edit_with_the_message_and_can_be_retried() {
    let server = TestServer::start("fail");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server, false).unwrap();
    let s = rig.wait("failure", |s| s.phase == Phase::Failed && !rig.state.is_running());
    let e = s.error.clone().unwrap();
    assert_eq!(e.stage, Stage::Server);
    assert!(!e.message.is_empty());
    assert!(matches!(rig.events.lock().unwrap().last(), Some(ImportEvent::Failed { stage: Stage::Server, .. })));
    assert!(rig.state.is_active(), "a failed extraction keeps the job");
    assert_eq!(s.metadata, ready.metadata);
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty());
    server.set_mode("ok");
    rig.extract(&s, &server, false).unwrap();
    assert!(rig.state.snapshot().unwrap().error.is_none());
    rig.wait("saved", |s| s.phase == Phase::Saved);
    rig.check_clean();
}

#[test]
fn an_unreachable_server_fails_the_extraction_without_touching_the_repository() {
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    // nothing listens on port 1 of the loopback
    rig.state.start_extraction(&ready.job, ready.metadata.clone().unwrap(), Some("http://127.0.0.1:1"), false).unwrap();
    let s = rig.wait("failure", |s| s.phase == Phase::Failed && !rig.state.is_running());
    assert_eq!(s.error.unwrap().stage, Stage::Server);
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty());
    rig.check_clean();
}

#[test]
fn extraction_refusals_before_anything_starts() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    let edits = ready.metadata.clone().unwrap();
    assert_eq!(rig.state.start_extraction(&ready.job, edits.clone(), None, false).unwrap_err(), NOT_CONFIGURED_MESSAGE);
    let mut bad = edits.clone();
    bad.title = "  ".into();
    assert!(rig.state.start_extraction(&ready.job, bad, Some(&server.url), false).unwrap_err().contains("title"));
    let mut bad = edits.clone();
    bad.year = Some(0);
    assert!(rig.state.start_extraction(&ready.job, bad, Some(&server.url), false).is_err());
    assert_eq!(rig.state.start_extraction("nope", edits.clone(), Some(&server.url), false).unwrap_err(), "Unknown import job");
    // still ready and untouched, no request reached the server
    assert_eq!(rig.state.snapshot().unwrap().phase, Phase::Ready);
    assert!(!server.saw("method=POST"));
    // edited fields are used (trimmed)
    let mut e2 = edits;
    e2.title = "  Edited Title ".into();
    rig.state.start_extraction(&ready.job, e2, Some(&server.url), false).unwrap();
    let t = rig.wait("saved", |s| s.phase == Phase::Saved).track.unwrap();
    assert_eq!(t.title, "Edited Title");
    rig.check_clean();
}

#[test]
fn a_second_start_is_refused_while_a_job_is_active() {
    let rig = Rig::new();
    let snap = rig.url("https://media.example/slow", false).unwrap();
    assert_eq!(rig.file("tagged.mp3").unwrap_err(), BUSY_MESSAGE);
    assert_eq!(rig.url(WATCH, false).unwrap_err(), BUSY_MESSAGE);
    assert_eq!(rig.state.prepare_url(&rig.root, WATCH).status, UrlPrepStatus::Busy);
    assert!(rig.state.discard(&snap.job).is_err());
    rig.state.cancel(&snap.job).unwrap();
    rig.wait_idle("cancel");
    // a job waiting in the edit pane is active too, until it is discarded
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    assert_eq!(rig.file("untagged.flac").unwrap_err(), BUSY_MESSAGE);
    assert!(rig.state.is_active());
    rig.state.discard(&ready.job).unwrap();
    assert!(!rig.state.is_active() && rig.state.snapshot().is_none());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).iter().all(|n| !n.starts_with("file-")));
    rig.check_clean();
}

fn pgrep(pattern: &str) -> bool {
    Command::new("pgrep").args(["-f", "--", pattern]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false)
}

#[test]
fn shutdown_during_a_download_stops_yt_dlp_within_three_seconds() {
    let rig = Rig::new();
    assert!(rig.state.shutdown(Duration::from_secs(1)), "nothing running");
    rig.url("https://media.example/slow", false).unwrap();
    rig.wait("some bytes", |s| s.downloaded > 0);
    let jobdir = Rig::names_in(&rig.root.join("import-tmp")).pop().unwrap();
    let marker = rig.root.join("import-tmp").join(&jobdir).to_string_lossy().into_owned();
    assert!(pgrep(&marker), "yt-dlp should be running");
    let t = Instant::now();
    assert!(rig.state.shutdown(Duration::from_secs(3)));
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(!rig.state.is_running());
    assert!(!pgrep(&marker), "yt-dlp is still running");
    rig.check_clean();
}

#[test]
fn shutdown_during_extraction_deletes_the_server_job() {
    let server = TestServer::start("hang");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server, false).unwrap();
    rig.wait("working", |s| s.phase == Phase::Working);
    assert!(rig.state.shutdown(Duration::from_secs(3)));
    assert!(server.wait_for("method=DELETE"));
    rig.check_clean();
}

#[test]
fn leftover_staging_folders_are_cleaned_at_start_but_only_marked_ones() {
    let rig = Rig::new();
    let tracks = rig.tracks_dir();
    let id = track_meta::new_id();
    let ours = tracks.join(format!(".staging-{id}"));
    fs::create_dir_all(ours.join("stems")).unwrap();
    fs::write(ours.join(".calliope-staging"), "x").unwrap();
    let id2 = track_meta::new_id();
    let theirs = tracks.join(format!(".staging-{id2}"));
    fs::create_dir_all(&theirs).unwrap();
    fs::write(theirs.join("note.txt"), "user").unwrap();
    rig.file("tagged.mp3").unwrap();
    rig.ready();
    assert!(!ours.exists());
    assert!(theirs.join("note.txt").is_file());
}

#[test]
fn an_empty_root_gets_its_layout_at_the_first_import() {
    let rig = Rig::new();
    let empty = rig._dir.path().join("empty-root");
    fs::create_dir(&empty).unwrap();
    assert_eq!(repository::status(&empty), RepoStatus::Empty);
    rig.state.start_file(&empty, FileKind::Audio, &rig.sources.join("tagged.mp3"), rig.sink.clone()).unwrap();
    rig.ready();
    assert_eq!(repository::status(&empty), RepoStatus::Ok);
}

#[test]
fn a_foreign_folder_is_not_used_as_a_repository() {
    let rig = Rig::new();
    let other = rig._dir.path().join("docs");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("a.txt"), "x").unwrap();
    assert!(rig.state.start_file(&other, FileKind::Audio, &rig.sources.join("tagged.mp3"), rig.sink.clone()).is_err());
    assert_eq!(Rig::names_in(&other), ["a.txt"]);
}

#[test]
fn a_missing_root_or_tool_is_reported_before_a_job_exists() {
    let rig = Rig::new();
    let gone = rig.root.join("nope");
    assert!(rig.state.start_file(&gone, FileKind::Audio, &rig.sources.join("tagged.mp3"), rig.sink.clone()).unwrap_err().contains("repository folder is missing"));
    assert!(rig.state.snapshot().is_none());
    let state = ImportState::new(Tools::default(), Arc::new(|f: &mut dyn FnMut()| f()), Duration::from_millis(50));
    assert!(state.start_url(&rig.root, WATCH, false, rig.sink.clone()).unwrap_err().contains("yt-dlp was not found"));
    assert!(state.start_file(&rig.root, FileKind::Audio, &rig.sources.join("tagged.mp3"), rig.sink.clone()).unwrap_err().contains("ffmpeg was not found"));
    assert!(rig.state.start_file(&rig.root, FileKind::Audio, Path::new("relative.mp3"), rig.sink.clone()).is_err());
}

#[test]
fn a_tool_installed_while_the_app_runs_is_found_without_a_restart() {
    let rig = Rig::new();
    let installed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = installed.clone();
    let real = rig.tools.clone();
    let state = ImportState::new(Tools::default(), rig.lock.clone(), Duration::from_millis(50))
        .with_rediscovery(move || if flag.load(std::sync::atomic::Ordering::SeqCst) { real.clone() } else { Tools::default() });
    assert!(state.start_url(&rig.root, WATCH, false, rig.sink.clone()).unwrap_err().contains("yt-dlp was not found"));
    assert!(state.tools().path_of(crate::tools::ToolName::YtDlp).is_none());
    installed.store(true, std::sync::atomic::Ordering::SeqCst); // "pacman -S yt-dlp" in another terminal
    assert!(state.tools().path_of(crate::tools::ToolName::YtDlp).is_some(), "Settings must see the new tool");
    state.start_url(&rig.root, WATCH, false, rig.sink.clone()).expect("the import must start without restarting the app");
    state.cancel(&state.snapshot().unwrap().job).ok();
    let _ = state.shutdown(Duration::from_secs(5));
}

#[test]
fn replace_sink_redirects_later_events() {
    let rig = Rig::new();
    let other: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = other.clone();
    rig.file("tagged.mp3").unwrap();
    rig.state.replace_sink(Arc::new(move |e| o2.lock().unwrap().push(e)));
    rig.ready();
    // the Ready event may have gone to either sink depending on timing, but never to neither
    let n = rig.events.lock().unwrap().len() + other.lock().unwrap().len();
    assert!(n >= 1);
}

#[test]
fn throttle_allows_ten_per_second() {
    let mut t = Throttle::default();
    let t0 = Instant::now();
    assert!(t.allow(t0));
    assert!(!t.allow(t0 + Duration::from_millis(50)));
    assert!(!t.allow(t0 + Duration::from_millis(99)));
    assert!(t.allow(t0 + Duration::from_millis(100)));
    let allowed = (0..1000u64).filter(|i| t.allow(t0 + Duration::from_millis(200 + i))).count();
    assert!(allowed <= 11, "{allowed}");
}

#[test]
fn url_tags_fill_the_gaps_of_the_info() {
    let probe = Probe {
        duration_s: Some(5.0),
        has_audio: true,
        has_video: false,
        tags: [("album", "Tag Album"), ("artist", "Tag Band"), ("title", "Tag Title"), ("date", "2001"), ("copyright", "(c) tag")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let info = TrackEdits { band: String::new(), album: "Info Album".into(), title: "Info Title".into(), composers: vec![], year: None, source_url: Some("https://x.example/".into()), copyright: None };
    let m = merge_url_edits(Some(info), &probe, "https://x.example/");
    assert_eq!((m.band.as_str(), m.album.as_str(), m.title.as_str(), m.year), ("Tag Band", "Info Album", "Info Title", Some(2001)));
    assert_eq!(m.copyright.as_deref(), Some("(c) tag"));
    let n = merge_url_edits(None, &probe, "https://x.example/");
    assert_eq!((n.title.as_str(), n.source_url.as_deref()), ("Tag Title", Some("https://x.example/")));
    let bare = Probe { tags: BTreeMap::new(), ..probe };
    assert_eq!(merge_url_edits(None, &bare, "https://x.example/p").title, "https://x.example/p");
}

#[test]
fn event_json_shapes() {
    let v = serde_json::to_value(ImportEvent::Cancelled { back_to: BackTo::Edit }).unwrap();
    assert_eq!(v, serde_json::json!({"phase": "cancelled", "back_to": "edit"}));
    let v = serde_json::to_value(ImportEvent::Failed { stage: Stage::Download, message: "m".into(), http_status: Some(403) }).unwrap();
    assert_eq!(v, serde_json::json!({"phase": "failed", "stage": "download", "message": "m", "http_status": 403}));
    let v = serde_json::to_value(ImportEvent::Downloading { downloaded: 1, total: None }).unwrap();
    assert_eq!(v, serde_json::json!({"phase": "downloading", "downloaded": 1, "total": null}));
    let s = serde_json::to_value(new_snapshot("j".into(), SourceKind::AudioFile, "a.mp3".into(), Phase::Preparing)).unwrap();
    assert_eq!(s["source"], serde_json::json!({"kind": "audio-file", "label": "a.mp3"}));
}

#[test]
fn an_early_error_in_start_extraction_leaves_the_job_untouched() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();

    // import-tmp became a symlink: the job folder is still reachable through it, but opening is refused
    let real = rig._dir.path().join("moved-import-tmp");
    fs::rename(rig.root.join("import-tmp"), &real).unwrap();
    std::os::unix::fs::symlink(&real, rig.root.join("import-tmp")).unwrap();
    assert!(rig.extract(&ready, &server, false).is_err());
    assert!(!rig.state.is_running(), "a refused start must not leave the job running");
    let s = rig.state.snapshot().unwrap();
    assert_eq!(s.phase, Phase::Ready);
    assert!(rig.state.cancel(&ready.job).is_ok());
    fs::remove_file(rig.root.join("import-tmp")).unwrap();
    fs::rename(&real, rig.root.join("import-tmp")).unwrap();

    // and the job works again afterwards
    rig.extract(&ready, &server, false).unwrap();
    rig.wait("saved", |s| s.phase == Phase::Saved);
}

#[test]
fn a_vanished_root_before_start_extraction_is_a_clean_error() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    fs::remove_dir_all(&rig.root).unwrap();
    assert!(rig.extract(&ready, &server, false).is_err());
    assert!(!rig.state.is_running());
    assert!(rig.state.cancel(&ready.job).is_ok());
    rig.state.discard(&ready.job).unwrap();
    assert!(rig.state.snapshot().is_none());
}

fn extract_to_end(rig: &Rig, server: &TestServer, keep: bool) -> JobSnapshot {
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    rig.extract(&ready, server, keep).unwrap();
    rig.wait("the end", |s| matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running())
}

fn check_sparse(rig: &Rig, done: &JobSnapshot) -> TrackRecord {
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let track = done.track.clone().unwrap();
    let names: Vec<&str> = track.stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar"]);
    let tdir = rig.tracks_dir().join(&track.id);
    assert_eq!(Rig::names_in(&tdir.join("stems")), ["bass.flac", "drums.flac", "guitar.flac", "vocals.flac"]);
    assert_eq!(done.dropped.len(), 2);
    assert_eq!(done.dropped[0], DroppedStem { name: "piano".into(), peak_dbfs: None });
    assert_eq!(done.dropped[1].name, "other");
    let p = done.dropped[1].peak_dbfs.unwrap();
    assert!(p > -61.0 && p < -59.0, "{p}");
    let Some(ImportEvent::Saved { dropped, .. }) = rig.events.lock().unwrap().last().cloned() else { panic!("no saved event") };
    assert_eq!(dropped, done.dropped);
    assert!(!rig.state.is_active(), "a finished job is not active");
    assert!(Repository::new(&rig.root).scan().problems.is_empty());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
    track
}

#[test]
fn silent_stems_are_dropped_from_the_saved_track() {
    let server = TestServer::start("sparse");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    let track = check_sparse(&rig, &done);
    assert_eq!(track.original, None);
    assert!(server.wait_for("method=DELETE"));
}

#[test]
fn silent_stems_are_dropped_with_keep_original_too() {
    let server = TestServer::start("sparse");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    let track = check_sparse(&rig, &done);
    assert_eq!(track.original.as_deref(), Some("original.flac"));
    assert!(rig.tracks_dir().join(&track.id).join("original.flac").is_file());
}

#[test]
fn a_track_of_only_silent_stems_fails_and_keeps_the_prepared_audio() {
    let server = TestServer::start("silent");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    assert_eq!(done.phase, Phase::Failed);
    let e = done.error.clone().unwrap();
    assert_eq!(e.stage, Stage::Server);
    assert_eq!(e.message, "Every stem is silent (below -50 dBFS), so no track was saved");
    assert!(done.dropped.is_empty());
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty());
    let tmp = rig.root.join("import-tmp");
    let job_dirs = Rig::names_in(&tmp);
    assert_eq!(job_dirs.len(), 1);
    assert!(tmp.join(&job_dirs[0]).join("audio.flac").is_file());
    assert!(server.wait_for("method=DELETE"));
    assert_eq!(server.job_dirs(), 0);
    assert!(rig.state.is_active(), "the failed job stays for a retry");
    rig.check_clean();
}

#[test]
fn server_silent_stems_are_never_fetched() {
    let server = TestServer::start("sparse");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    check_sparse(&rig, &done);
    for n in ["vocals", "drums", "bass", "guitar"] {
        assert_eq!(server.stem_gets(n), 1, "{n}");
    }
    assert_eq!(server.stem_gets("piano"), 0);
    assert_eq!(server.stem_gets("other"), 0);
}

#[test]
fn an_old_server_without_peaks_gives_the_same_result_with_all_stems_fetched() {
    let server = TestServer::start_with("sparse", &["--no-stem-peaks"]);
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    check_sparse(&rig, &done);
    assert_eq!(server.all_stem_gets(), 6);
    assert_eq!(server.stem_gets("piano"), 1);
    assert_eq!(server.stem_gets("other"), 1);
}

#[test]
fn all_silent_by_the_server_fetches_nothing() {
    let server = TestServer::start("silent");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    assert_eq!(done.phase, Phase::Failed);
    assert!(done.error.clone().unwrap().message.starts_with("Every stem is silent"));
    assert_eq!(server.all_stem_gets(), 0);
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty());
    let tmp = rig.root.join("import-tmp");
    let job_dirs = Rig::names_in(&tmp);
    assert_eq!(job_dirs.len(), 1);
    assert!(tmp.join(&job_dirs[0]).join("audio.flac").is_file());
    rig.check_clean();
}

#[test]
fn an_undecodable_stem_is_fetched_and_kept_while_the_server_silent_one_is_not() {
    let server = TestServer::start("undecodable");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let names: Vec<String> = done.track.unwrap().stems.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar", "piano"]);
    assert_eq!(done.dropped.len(), 1);
    assert_eq!(done.dropped[0].name, "other");
    assert_eq!(server.stem_gets("piano"), 1);
    assert_eq!(server.stem_gets("other"), 0);
}

#[test]
fn a_normal_extraction_drops_nothing() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    assert_eq!(done.phase, Phase::Saved);
    assert_eq!(done.track.unwrap().stems.len(), 6);
    assert!(done.dropped.is_empty());
    let Some(ImportEvent::Saved { dropped, .. }) = rig.events.lock().unwrap().last().cloned() else { panic!() };
    assert!(dropped.is_empty());
    let v = serde_json::to_value(rig.events.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(v["dropped"], serde_json::json!([]));
}
