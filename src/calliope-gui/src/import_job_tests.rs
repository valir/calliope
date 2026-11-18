//! Tests of the job orchestrator against the REAL `calliope-stems` binary on 127.0.0.1 (temp
//! work dir, the stub separator, never the real model), the fake yt-dlp and the real
//! ffmpeg/ffprobe, in temp repositories only.

use super::*;
use crate::test_server::{pgrep, project, TestServer};
use std::collections::BTreeMap;
use std::fs;

fn fixture(name: &str) -> PathBuf {
    project().join("tests/fixtures/import").join(name)
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
    crate::tools::find_in_path(name, &std::env::var_os("PATH").unwrap_or_default())
        .unwrap_or_else(|| panic!("{name} is needed"))
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
        for f in [
            "tagged.mp3",
            "with-audio.mp4",
            "no-audio.mp4",
            "long.flac",
            "untagged.flac",
        ] {
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
        Rig {
            _dir: dir,
            root,
            outside,
            sources,
            state: ImportState::new(tools.clone(), lock.clone(), Duration::from_millis(50)),
            tools,
            lock,
            events,
            sink,
        }
    }

    fn url(&self, url: &str, resume: bool) -> Result<JobSnapshot, String> {
        self.state
            .start_url(&self.root, url, resume, self.sink.clone())
    }

    fn file(&self, name: &str) -> Result<JobSnapshot, String> {
        let kind = if name.ends_with(".mp4") {
            FileKind::Video
        } else {
            FileKind::Audio
        };
        self.state.start_file(
            &self.root,
            kind,
            &self.sources.join(name),
            self.sink.clone(),
        )
    }

    /// Copies fixtures into `sources/batch/` under the given names (`(name, fixture)`, in
    /// import order) and starts a directory import of them.
    fn batch(&self, files: &[(&str, &str)], server: &TestServer) -> Result<JobSnapshot, String> {
        self.batch_to(files, Some(&server.url))
    }

    fn batch_to(&self, files: &[(&str, &str)], url: Option<&str>) -> Result<JobSnapshot, String> {
        let dir = self.sources.join("batch");
        fs::create_dir_all(&dir).unwrap();
        let paths: Vec<PathBuf> = files
            .iter()
            .map(|(name, fix)| {
                fs::copy(fixture(fix), dir.join(name)).unwrap();
                dir.join(name)
            })
            .collect();
        self.state
            .start_batch(&self.root, &dir, paths, url, false, self.sink.clone())
    }

    fn batch_items(&self) -> Vec<BatchItem> {
        self.state
            .snapshot()
            .and_then(|s| s.batch)
            .map(|b| b.items)
            .unwrap_or_default()
    }

    fn wait(&self, what: &str, pred: impl Fn(&JobSnapshot) -> bool) -> JobSnapshot {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(s) = self.state.snapshot() {
                if pred(&s) {
                    return s;
                }
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for {what}; at {:?}",
                self.state.snapshot().map(|s| (s.phase, s.error))
            );
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

    /// The final snapshot is published before the terminal event is emitted, so wait (bounded) for the event.
    fn last_event_where(&self, what: &str, pred: impl Fn(&ImportEvent) -> bool) -> ImportEvent {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(e) = self.events.lock().unwrap().last().filter(|e| pred(e)) {
                return e.clone();
            }
            assert!(
                Instant::now() < end,
                "timed out waiting for the {what} event; got {:?}",
                self.events.lock().unwrap().last()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ready(&self) -> JobSnapshot {
        let s = self.wait_idle("the job to stop");
        assert_eq!(s.phase, Phase::Ready, "{:?}", s.error);
        s
    }

    fn extract(
        &self,
        s: &JobSnapshot,
        server: &TestServer,
        keep: bool,
    ) -> Result<JobSnapshot, String> {
        self.state
            .start_extraction(&s.job, s.metadata.clone().unwrap(), Some(&server.url), keep)
    }

    fn tracks_dir(&self) -> PathBuf {
        self.root.join("tracks")
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

    fn phases(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["phase"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        v.dedup();
        v
    }

    fn check_clean(&self) {
        assert_eq!(
            fs::read_to_string(self.outside.join("precious.txt")).unwrap(),
            "do not touch"
        );
        assert_eq!(Self::names_in(&self.outside), ["precious.txt"]);
        let tracks = self.tracks_dir();
        assert!(
            Self::names_in(&tracks)
                .iter()
                .all(|n| !n.starts_with(".staging-")),
            "staging left in {tracks:?}"
        );
    }
}

fn hashes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

/// A copy of `src` with the given tags set (real ffmpeg, audio copied). Returns `dest`.
fn tagged_copy(src: &Path, dest: &Path, tags: &[(&str, &str)]) -> PathBuf {
    let mut cmd = std::process::Command::new(tool("ffmpeg"));
    cmd.args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(format!("file:{}", src.display()))
        .args(["-map", "0:a", "-c", "copy"]);
    for (k, v) in tags {
        cmd.arg("-metadata").arg(format!("{k}={v}"));
    }
    let out = cmd
        .arg(format!("file:{}", dest.display()))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dest.to_path_buf()
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
    assert_eq!(
        (meta.band.as_str(), meta.title.as_str(), meta.year),
        ("The Example Band", "Night Drive", Some(2020))
    );
    assert_eq!(
        meta.source_url.as_deref(),
        Some("https://media.example/watch?v=abc123")
    );
    assert!(ready.duration_s.unwrap() > 4.0);
    assert!(rig.state.is_active() && !rig.state.is_running());
    assert!(Rig::names_in(&rig.root.join("import-tmp"))
        .iter()
        .any(|n| n.starts_with("url-")));

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
    assert_eq!(
        names,
        ["vocals", "drums", "bass", "guitar", "piano", "other"]
    );
    assert_eq!(
        (t.audio.clone(), t.original.clone(), t.stem_model.as_deref()),
        (None, None, Some("htdemucs_6s"))
    );
    assert!(t.missing.is_empty());
    assert_eq!(
        (t.band.as_str(), t.title.as_str(), t.year),
        (meta.band.as_str(), meta.title.as_str(), meta.year)
    );
    assert_eq!(t.source_url, meta.source_url);
    let tdir = rig.tracks_dir().join(&t.id);
    for s in &t.stems {
        assert!(fs::read(tdir.join(&s.file)).unwrap().starts_with(b"fLaC"));
        assert_eq!(
            fs::read(tdir.join(&s.file)).unwrap(),
            fs::read(fixture(&format!("stems/{}.flac", s.name))).unwrap()
        );
    }
    assert_eq!(Rig::names_in(&tdir), ["stems", "track.json"]);
    // temp folder gone, the user's file stays, the server job is gone
    assert_eq!(Rig::names_in(&rig.root.join("import-tmp")), ["keep-me.txt"]);
    assert!(server.wait_for("method=DELETE"));
    assert!(server.saw("method=POST"));
    let phases = rig.phases();
    for p in [
        "downloading",
        "preparing",
        "ready",
        "uploading",
        "queued",
        "receiving",
        "saving",
        "saved",
    ] {
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
    assert_eq!(
        (meta.title.as_str(), meta.band.as_str(), meta.year),
        ("Glass Harbour", "The Example Band", Some(2021))
    );
    assert_eq!(ready.source.label, "tagged.mp3");
    assert_eq!(ready.source.kind, SourceKind::AudioFile);
    let src_before = hashes(&rig.sources);
    rig.extract(&ready, &server, true).unwrap();
    let t = rig
        .wait("saved", |s| s.phase == Phase::Saved)
        .track
        .unwrap();
    assert_eq!(t.original.as_deref(), Some("original.flac"));
    let tdir = rig.tracks_dir().join(&t.id);
    assert!(fs::read(tdir.join("original.flac"))
        .unwrap()
        .starts_with(b"fLaC"));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(tdir.join("track.json")).unwrap()).unwrap();
    assert_eq!(json["original"], "original.flac");
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["type"], "stem");
    assert!(json["audio"].is_null());
    assert_eq!(
        Rig::names_in(&rig.root.join("import-tmp")),
        Vec::<String>::new()
    );
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
    // the track-number tag becomes the position, but only for a track with an album; an untagged source has none
    let finish = |ready: &JobSnapshot| {
        rig.extract(ready, &server, false).unwrap();
        rig.wait("saved", |s| s.phase == Phase::Saved)
            .track
            .unwrap()
    };
    let save = |name: &str, tags: &[(&str, &str)]| {
        let src = tagged_copy(&fixture("tagged.mp3"), &rig.sources.join(name), tags);
        rig.file(name).unwrap();
        let track = finish(&rig.ready());
        fs::remove_file(src).unwrap();
        track
    };
    assert_eq!(
        finish(&rig.wait("ready", |s| s.phase == Phase::Ready)).position,
        None
    );
    assert_eq!(
        save("numbered.mp3", &[("track", "2/9"), ("title", "Numbered")]).position,
        Some(2)
    );
    let no_album = save(
        "noalbum.mp3",
        &[("track", "3"), ("album", ""), ("title", "No album")],
    );
    assert_eq!((no_album.album.as_str(), no_album.position), ("", None));
    let tracks = Repository::new(&rig.root).scan().tracks;
    assert_eq!(tracks.iter().filter(|t| t.position.is_some()).count(), 1);
    assert!(tracks.iter().all(|t| t.album_position.is_none()));
    rig.check_clean();
}

#[test]
fn a_video_without_audio_fails_with_the_exact_text() {
    let rig = Rig::new();
    rig.file("no-audio.mp4").unwrap();
    let s = rig.wait_idle("failure");
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(
        (e.stage, e.message.as_str()),
        (
            Stage::Prepare,
            "Selected file no-audio.mp4 has no audio track"
        )
    );
    assert!(!rig.state.is_active());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.last_event_where("terminal", |e| {
        matches!(
            e,
            ImportEvent::Failed {
                stage: Stage::Prepare,
                ..
            }
        )
    });
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
    assert_eq!(
        s.error.unwrap().message,
        "Tracks longer than 15 minutes are not supported"
    );
    assert!(
        !server.saw("method=POST") && !server.saw("method=GET"),
        "{:?}",
        server.log.lock().unwrap()
    );
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
}

#[test]
fn http_403_is_a_download_failure_with_its_status() {
    let rig = Rig::new();
    rig.url("https://media.example/http403", false).unwrap();
    let s = rig.wait_idle("failure");
    let e = s.error.unwrap();
    assert_eq!(
        (e.stage, e.http_status, e.message.as_str()),
        (
            Stage::Download,
            Some(403),
            "Error 403 when attempting download"
        )
    );
    assert!(!rig.state.is_active());
    // an invalid URL never creates a job
    assert_eq!(
        rig.url("ftp://x.example/a", false).unwrap_err(),
        "Entered URL is invalid"
    );
    rig.check_clean();
}

#[test]
fn cancel_keeps_the_partial_and_resume_continues_while_start_over_restarts() {
    let rig = Rig::new();
    let chunk = fs::metadata(fixture("download.webm"))
        .unwrap()
        .len()
        .div_ceil(5);
    let u = "https://media.example/slow";
    assert_eq!(
        rig.state.prepare_url(&rig.root, u).status,
        UrlPrepStatus::Ready
    );
    let snap = rig.url(u, false).unwrap();
    rig.wait("some bytes", |s| s.downloaded > 0);
    rig.state.cancel(&snap.job).unwrap();
    let s = rig.wait_idle("cancel");
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(!rig.state.is_active());
    rig.last_event_where("terminal", |e| {
        matches!(
            e,
            ImportEvent::Cancelled {
                back_to: BackTo::Source
            }
        )
    });
    let prep = rig.state.prepare_url(&rig.root, u);
    assert_eq!(prep.status, UrlPrepStatus::Partial);
    assert!(prep.partial_bytes >= chunk);
    assert_eq!(
        prep.message,
        "Incomplete download file from the same URL found"
    );

    // Start Over: restarts from zero
    let so = rig.url(u, false).unwrap();
    let first = rig.wait("some bytes", |s| s.downloaded > 0).downloaded;
    assert_eq!(first, chunk, "start over must restart at zero");
    rig.state.cancel(&so.job).unwrap();
    assert_eq!(rig.wait_idle("cancel").phase, Phase::Cancelled);

    // Resume: continues after the part that was kept
    rig.url(u, true).unwrap();
    let first = rig.wait("some bytes", |s| s.downloaded > 0).downloaded;
    assert!(
        first >= 2 * chunk,
        "resume must continue the part (first progress {first}, chunk {chunk})"
    );
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
    assert_eq!(
        rig.state.prepare_url(&rig.root, WATCH).status,
        UrlPrepStatus::Busy
    );
    // the app was closed in the edit pane: a new state finds audio.flac and skips the download
    let state2 = ImportState::new(
        rig.tools.clone(),
        rig.lock.clone(),
        Duration::from_millis(50),
    );
    let events: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let e2 = events.clone();
    state2
        .start_url(
            &rig.root,
            WATCH,
            true,
            Arc::new(move |e| e2.lock().unwrap().push(e)),
        )
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while state2.is_running() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    let s = state2.snapshot().unwrap();
    assert_eq!(s.phase, Phase::Ready);
    assert_eq!(s.metadata, first.metadata);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .all(|e| !matches!(e, ImportEvent::Downloading { .. })));
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
    rig.last_event_where("terminal", |e| {
        matches!(
            e,
            ImportEvent::Cancelled {
                back_to: BackTo::Edit
            }
        )
    });
    assert!(
        server.wait_for("method=DELETE"),
        "{:?}",
        server.log.lock().unwrap()
    );
    let end = Instant::now() + Duration::from_secs(10);
    while server.job_dirs() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.job_dirs(), 0, "the server job was not removed");
    // no staging, no track; the temp audio is still there, so Extract can be retried
    assert!(
        Rig::names_in(&rig.tracks_dir()).is_empty(),
        "{:?}",
        Rig::names_in(&rig.tracks_dir())
    );
    let jobdir = Rig::names_in(&rig.root.join("import-tmp"));
    assert_eq!(jobdir.len(), 1);
    assert!(rig
        .root
        .join("import-tmp")
        .join(&jobdir[0])
        .join("audio.flac")
        .is_file());
    assert!(rig.state.is_active());
    // retry succeeds once the server behaves; the original is adopted only then
    server.set_mode("ok");
    rig.extract(&s, &server, true).unwrap();
    let t = rig
        .wait("saved", |s| s.phase == Phase::Saved)
        .track
        .unwrap();
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
    let s = rig.wait("failure", |s| {
        s.phase == Phase::Failed && !rig.state.is_running()
    });
    let e = s.error.clone().unwrap();
    assert_eq!(e.stage, Stage::Server);
    assert!(!e.message.is_empty());
    rig.last_event_where("terminal", |e| {
        matches!(
            e,
            ImportEvent::Failed {
                stage: Stage::Server,
                ..
            }
        )
    });
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
    rig.state
        .start_extraction(
            &ready.job,
            ready.metadata.clone().unwrap(),
            Some("http://127.0.0.1:1"),
            false,
        )
        .unwrap();
    let s = rig.wait("failure", |s| {
        s.phase == Phase::Failed && !rig.state.is_running()
    });
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
    assert_eq!(
        rig.state
            .start_extraction(&ready.job, edits.clone(), None, false)
            .unwrap_err(),
        NOT_CONFIGURED_MESSAGE
    );
    let mut bad = edits.clone();
    bad.title = "  ".into();
    assert!(rig
        .state
        .start_extraction(&ready.job, bad, Some(&server.url), false)
        .unwrap_err()
        .contains("title"));
    let mut bad = edits.clone();
    bad.year = Some(0);
    assert!(rig
        .state
        .start_extraction(&ready.job, bad, Some(&server.url), false)
        .is_err());
    assert_eq!(
        rig.state
            .start_extraction("nope", edits.clone(), Some(&server.url), false)
            .unwrap_err(),
        "Unknown import job"
    );
    // still ready and untouched, no request reached the server
    assert_eq!(rig.state.snapshot().unwrap().phase, Phase::Ready);
    assert!(!server.saw("method=POST"));
    // edited fields are used (trimmed)
    let mut e2 = edits;
    e2.title = "  Edited Title ".into();
    rig.state
        .start_extraction(&ready.job, e2, Some(&server.url), false)
        .unwrap();
    let t = rig
        .wait("saved", |s| s.phase == Phase::Saved)
        .track
        .unwrap();
    assert_eq!(t.title, "Edited Title");
    rig.check_clean();
}

#[test]
fn a_second_start_is_refused_while_a_job_is_active() {
    let rig = Rig::new();
    let snap = rig.url("https://media.example/slow", false).unwrap();
    assert_eq!(rig.file("tagged.mp3").unwrap_err(), BUSY_MESSAGE);
    assert_eq!(rig.url(WATCH, false).unwrap_err(), BUSY_MESSAGE);
    assert_eq!(
        rig.state.prepare_url(&rig.root, WATCH).status,
        UrlPrepStatus::Busy
    );
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
    assert!(Rig::names_in(&rig.root.join("import-tmp"))
        .iter()
        .all(|n| !n.starts_with("file-")));
    rig.check_clean();
}

#[test]
fn shutdown_during_a_download_stops_yt_dlp_within_three_seconds() {
    let rig = Rig::new();
    assert!(
        rig.state.shutdown(Duration::from_secs(1)),
        "nothing running"
    );
    rig.url("https://media.example/slow", false).unwrap();
    rig.wait("some bytes", |s| s.downloaded > 0);
    let jobdir = Rig::names_in(&rig.root.join("import-tmp")).pop().unwrap();
    let marker = rig
        .root
        .join("import-tmp")
        .join(&jobdir)
        .to_string_lossy()
        .into_owned();
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

const AB: [(&str, &str); 2] = [("a.flac", "untagged.flac"), ("b.mp3", "tagged.mp3")];

#[test]
fn batch_imports_each_file_in_order() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let snap = rig.batch(&AB, &server).unwrap();
    assert_eq!(
        (snap.source.kind, snap.phase),
        (SourceKind::Directory, Phase::Preparing)
    );
    assert_eq!(snap.source.label, "batch");
    let b = snap.batch.unwrap();
    assert_eq!(
        (b.files.clone(), b.current, b.items.len()),
        (vec!["a.flac".to_string(), "b.mp3".to_string()], Some(0), 0)
    );
    assert!(rig.state.is_active());
    let done = rig.wait_idle("the batch to end");
    assert_eq!(done.phase, Phase::Finished);
    let batch = done.batch.clone().unwrap();
    assert_eq!(batch.current, None);
    assert_eq!(batch.items.len(), 2);
    assert!(!rig.state.is_active());
    let ev = rig.last_event_where("finished", |e| matches!(e, ImportEvent::Finished { .. }));
    assert_eq!(
        ev,
        ImportEvent::Finished {
            batch: batch.clone()
        }
    );
    // queued/working depend on timing
    let phases: Vec<String> = rig
        .phases()
        .into_iter()
        .filter(|p| p != "queued" && p != "working")
        .collect();
    let file = [
        "batch-file",
        "preparing",
        "normalizing",
        "uploading",
        "receiving",
        "saving",
        "batch-item",
    ];
    let expected: Vec<&str> = file
        .iter()
        .chain(file.iter())
        .copied()
        .chain(["finished"])
        .collect();
    assert_eq!(phases, expected);
    let tracks = Repository::new(&rig.root).scan();
    assert!(tracks.problems.is_empty(), "{:?}", tracks.problems);
    let mut titles: Vec<&str> = tracks.tracks.iter().map(|t| t.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, ["Glass Harbour", "a"]);
    for (i, item) in batch.items.iter().enumerate() {
        assert_eq!(
            (item.index, item.name.as_str()),
            (i, ["a.flac", "b.mp3"][i])
        );
        assert!(
            matches!(&item.outcome, BatchOutcome::Saved { track, dropped, .. } if dropped.is_empty() && tracks.tracks.contains(track))
        );
    }
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    assert_eq!(
        hashes(&rig.sources.join("batch")),
        AB.iter()
            .map(|(n, f)| (n.to_string(), fs::read(fixture(f)).unwrap()))
            .collect()
    );
    rig.check_clean();
}

#[test]
fn batch_saves_the_tag_positions_and_leaves_existing_tracks_alone() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let album = [("album", "Order"), ("artist", "Band")];
    let tags = |n: &str, title: &'static str| -> Vec<(&'static str, String)> {
        let mut v: Vec<(&str, String)> = album.iter().map(|(k, v)| (*k, v.to_string())).collect();
        v.push(("title", title.to_string()));
        if !n.is_empty() {
            v.push(("track", n.to_string()));
        }
        v
    };
    let make = |dir: &Path, name: &str, base: &str, t: Vec<(&str, String)>| {
        let t: Vec<(&str, &str)> = t.iter().map(|(k, v)| (*k, v.as_str())).collect();
        tagged_copy(&fixture(base), &dir.join(name), &t);
    };
    // an existing track of the album
    let first = rig.sources.join("first");
    fs::create_dir_all(&first).unwrap();
    make(&first, "x.mp3", "tagged.mp3", tags("7", "Existing"));
    rig.state
        .start_file(
            &rig.root,
            FileKind::Audio,
            &first.join("x.mp3"),
            rig.sink.clone(),
        )
        .unwrap();
    let ready = rig.ready();
    rig.extract(&ready, &server, false).unwrap();
    let existing = rig
        .wait("saved", |s| s.phase == Phase::Saved)
        .track
        .unwrap();
    assert_eq!(existing.position, Some(7));
    let existing_bytes = fs::read(rig.tracks_dir().join(&existing.id).join("track.json")).unwrap();
    // the batch: tags 2, 1, 3 and an untagged file
    let dir = rig.sources.join("album");
    fs::create_dir_all(&dir).unwrap();
    make(&dir, "a.mp3", "tagged.mp3", tags("2", "Second"));
    make(&dir, "b.mp3", "tagged.mp3", tags("1", "First"));
    make(&dir, "c.mp3", "tagged.mp3", tags("3/10", "Third"));
    make(&dir, "d.mp3", "tagged.mp3", tags("", "Unnumbered"));
    let paths: Vec<PathBuf> = ["a.mp3", "b.mp3", "c.mp3", "d.mp3"]
        .iter()
        .map(|n| dir.join(n))
        .collect();
    rig.state
        .start_batch(
            &rig.root,
            &dir,
            paths,
            Some(&server.url),
            false,
            rig.sink.clone(),
        )
        .unwrap();
    let done = rig.wait_idle("the batch to end");
    assert_eq!(done.batch.unwrap().items.len(), 4);
    let mut got: Vec<(String, Option<u32>)> = Repository::new(&rig.root)
        .scan()
        .tracks
        .into_iter()
        .map(|t| (t.title, t.position))
        .collect();
    got.sort();
    let want = [
        ("Existing", Some(7)),
        ("First", Some(1)),
        ("Second", Some(2)),
        ("Third", Some(3)),
        ("Unnumbered", None),
    ];
    assert_eq!(
        got,
        want.iter()
            .map(|(t, p)| (t.to_string(), *p))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fs::read(rig.tracks_dir().join(&existing.id).join("track.json")).unwrap(),
        existing_bytes
    );
    rig.check_clean();
}

#[test]
fn batch_failures_go_on() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    rig.batch(
        &[
            ("1.flac", "untagged.flac"),
            ("2.mp3", "not-audio.mp3"),
            ("3.ogg", "tagged.ogg"),
        ],
        &server,
    )
    .unwrap();
    rig.wait_idle("the batch to end");
    rig.last_event_where("finished", |e| matches!(e, ImportEvent::Finished { .. }));
    let items = rig.batch_items();
    assert!(
        matches!(items[0].outcome, BatchOutcome::Saved { .. }),
        "{items:?}"
    );
    assert!(
        matches!(&items[1].outcome, BatchOutcome::Failed { stage: Stage::Prepare, message } if message.contains("2.mp3")),
        "{items:?}"
    );
    assert!(
        matches!(items[2].outcome, BatchOutcome::Saved { .. }),
        "{items:?}"
    );
    assert_eq!(Repository::new(&rig.root).scan().tracks.len(), 2);
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());

    // a server failure on the second file only
    let server = TestServer::start("seq:ok,fail,ok");
    rig.events.lock().unwrap().clear();
    rig.batch(
        &[
            ("4.flac", "untagged.flac"),
            ("5.flac", "untagged.flac"),
            ("6.flac", "untagged.flac"),
        ],
        &server,
    )
    .unwrap();
    rig.wait_idle("the batch to end");
    rig.last_event_where("finished", |e| matches!(e, ImportEvent::Finished { .. }));
    let items = rig.batch_items();
    assert!(
        matches!(items[0].outcome, BatchOutcome::Saved { .. }),
        "{items:?}"
    );
    assert!(
        matches!(
            items[1].outcome,
            BatchOutcome::Failed {
                stage: Stage::Server,
                ..
            }
        ),
        "{items:?}"
    );
    assert!(
        matches!(items[2].outcome, BatchOutcome::Saved { .. }),
        "{items:?}"
    );
    assert!(
        rig.events.lock().unwrap().iter().all(|e| !matches!(
            e,
            ImportEvent::Failed { .. } | ImportEvent::Ready { .. } | ImportEvent::Saved { .. }
        )),
        "single-import events in a batch: {:?}",
        rig.phases()
    );
    assert_eq!(Repository::new(&rig.root).scan().tracks.len(), 4);
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
}

#[test]
fn batch_cancel_on_the_second_file() {
    let server = TestServer::start("seq:ok,hang");
    let rig = Rig::new();
    let snap = rig.batch(&AB, &server).unwrap();
    rig.wait("the second file working", |s| {
        s.phase == Phase::Working && s.batch.as_ref().is_some_and(|b| b.current == Some(1))
    });
    rig.state.cancel(&snap.job).unwrap();
    let s = rig.wait_idle("cancel");
    assert_eq!(s.phase, Phase::Cancelled);
    assert!(!rig.state.is_active());
    let b = s.batch.unwrap();
    assert_eq!((b.current, b.items.len(), b.files.len()), (None, 1, 2));
    assert!(matches!(b.items[0].outcome, BatchOutcome::Saved { .. }));
    rig.last_event_where("cancelled", |e| {
        matches!(
            e,
            ImportEvent::Cancelled {
                back_to: BackTo::Source
            }
        )
    });
    assert_eq!(Repository::new(&rig.root).scan().tracks.len(), 1);
    let end = Instant::now() + Duration::from_secs(10);
    while server.job_dirs() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.job_dirs(), 0, "the server job was not removed");
    assert!(
        server
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("method=DELETE"))
            .count()
            >= 2,
        "one delete per file at least"
    );
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
}

#[test]
fn batch_refusals_start_nothing() {
    let server = TestServer::start("hang");
    let rig = Rig::new();
    assert!(rig
        .state
        .start_batch(
            &rig.root,
            &rig.sources,
            Vec::new(),
            Some(&server.url),
            false,
            rig.sink.clone()
        )
        .is_err());
    assert_eq!(rig.batch_to(&AB, None).unwrap_err(), NOT_CONFIGURED_MESSAGE);
    assert_eq!(
        rig.batch_to(&AB, Some("  ")).unwrap_err(),
        NOT_CONFIGURED_MESSAGE
    );
    assert!(rig.state.snapshot().is_none());
    assert!(
        !rig.root.join("import-tmp").exists()
            || Rig::names_in(&rig.root.join("import-tmp")).is_empty()
    );

    let snap = rig.batch(&AB, &server).unwrap();
    assert_eq!(rig.batch(&AB, &server).unwrap_err(), BUSY_MESSAGE);
    assert_eq!(rig.file("tagged.mp3").unwrap_err(), BUSY_MESSAGE);
    assert_eq!(rig.state.snapshot().unwrap().job, snap.job);
    rig.state.cancel(&snap.job).unwrap();
    assert_eq!(rig.wait_idle("cancel").phase, Phase::Cancelled);
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
}

#[test]
fn shutdown_during_a_batch() {
    let server = TestServer::start("seq:hang");
    let rig = Rig::new();
    rig.batch(&AB, &server).unwrap();
    rig.wait("working", |s| s.phase == Phase::Working);
    assert!(rig.state.shutdown(Duration::from_secs(3)));
    assert!(server.wait_for("method=DELETE"));
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
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
    rig.state
        .start_file(
            &empty,
            FileKind::Audio,
            &rig.sources.join("tagged.mp3"),
            rig.sink.clone(),
        )
        .unwrap();
    rig.ready();
    assert_eq!(repository::status(&empty), RepoStatus::Ok);
}

#[test]
fn a_foreign_folder_is_not_used_as_a_repository() {
    let rig = Rig::new();
    let other = rig._dir.path().join("docs");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("a.txt"), "x").unwrap();
    assert!(rig
        .state
        .start_file(
            &other,
            FileKind::Audio,
            &rig.sources.join("tagged.mp3"),
            rig.sink.clone()
        )
        .is_err());
    assert_eq!(Rig::names_in(&other), ["a.txt"]);
}

#[test]
fn a_missing_root_or_tool_is_reported_before_a_job_exists() {
    let rig = Rig::new();
    let gone = rig.root.join("nope");
    assert!(rig
        .state
        .start_file(
            &gone,
            FileKind::Audio,
            &rig.sources.join("tagged.mp3"),
            rig.sink.clone()
        )
        .unwrap_err()
        .contains("repository folder is missing"));
    assert!(rig.state.snapshot().is_none());
    let state = ImportState::new(
        Tools::default(),
        Arc::new(|f: &mut dyn FnMut()| f()),
        Duration::from_millis(50),
    );
    assert!(state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .unwrap_err()
        .contains("yt-dlp was not found"));
    assert!(state
        .start_file(
            &rig.root,
            FileKind::Audio,
            &rig.sources.join("tagged.mp3"),
            rig.sink.clone()
        )
        .unwrap_err()
        .contains("ffmpeg was not found"));
    assert!(rig
        .state
        .start_file(
            &rig.root,
            FileKind::Audio,
            Path::new("relative.mp3"),
            rig.sink.clone()
        )
        .is_err());
}

#[test]
fn a_tool_installed_while_the_app_runs_is_found_without_a_restart() {
    let rig = Rig::new();
    let installed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = installed.clone();
    let real = rig.tools.clone();
    let state = ImportState::new(
        Tools::default(),
        rig.lock.clone(),
        Duration::from_millis(50),
    )
    .with_rediscovery(move || {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            real.clone()
        } else {
            Tools::default()
        }
    });
    assert!(state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .unwrap_err()
        .contains("yt-dlp was not found"));
    assert!(state
        .tools()
        .path_of(crate::tools::ToolName::YtDlp)
        .is_none());
    installed.store(true, std::sync::atomic::Ordering::SeqCst); // "pacman -S yt-dlp" in another terminal
    assert!(
        state
            .tools()
            .path_of(crate::tools::ToolName::YtDlp)
            .is_some(),
        "Settings must see the new tool"
    );
    state
        .start_url(&rig.root, WATCH, false, rig.sink.clone())
        .expect("the import must start without restarting the app");
    state.cancel(&state.snapshot().unwrap().job).ok();
    let _ = state.shutdown(Duration::from_secs(5));
}

#[test]
fn replace_sink_redirects_later_events() {
    let rig = Rig::new();
    let other: Arc<Mutex<Vec<ImportEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let o2 = other.clone();
    rig.file("tagged.mp3").unwrap();
    rig.state
        .replace_sink(Arc::new(move |e| o2.lock().unwrap().push(e)));
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
    let allowed = (0..1000u64)
        .filter(|i| t.allow(t0 + Duration::from_millis(200 + i)))
        .count();
    assert!(allowed <= 11, "{allowed}");
}

#[test]
fn url_tags_fill_the_gaps_of_the_info() {
    let probe = Probe {
        duration_s: Some(5.0),
        has_audio: true,
        has_video: false,
        tags: [
            ("album", "Tag Album"),
            ("artist", "Tag Band"),
            ("title", "Tag Title"),
            ("date", "2001"),
            ("copyright", "(c) tag"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    };
    let info = TrackEdits {
        band: String::new(),
        album: "Info Album".into(),
        title: "Info Title".into(),
        composers: vec![],
        year: None,
        source_url: Some("https://x.example/".into()),
        copyright: None,
    };
    let m = merge_url_edits(Some(info), &probe, "https://x.example/");
    assert_eq!(
        (m.band.as_str(), m.album.as_str(), m.title.as_str(), m.year),
        ("Tag Band", "Info Album", "Info Title", Some(2001))
    );
    assert_eq!(m.copyright.as_deref(), Some("(c) tag"));
    let n = merge_url_edits(None, &probe, "https://x.example/");
    assert_eq!(
        (n.title.as_str(), n.source_url.as_deref()),
        ("Tag Title", Some("https://x.example/"))
    );
    let bare = Probe {
        tags: BTreeMap::new(),
        ..probe
    };
    assert_eq!(
        merge_url_edits(None, &bare, "https://x.example/p").title,
        "https://x.example/p"
    );
}

#[test]
fn event_json_shapes() {
    let v = serde_json::to_value(ImportEvent::Cancelled {
        back_to: BackTo::Edit,
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "cancelled", "back_to": "edit"})
    );
    let v = serde_json::to_value(ImportEvent::Failed {
        stage: Stage::Download,
        message: "m".into(),
        http_status: Some(403),
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "failed", "stage": "download", "message": "m", "http_status": 403})
    );
    let v = serde_json::to_value(ImportEvent::Downloading {
        downloaded: 1,
        total: None,
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "downloading", "downloaded": 1, "total": null})
    );
    let s = serde_json::to_value(new_snapshot(
        "j".into(),
        SourceKind::AudioFile,
        "a.mp3".into(),
        Phase::Preparing,
    ))
    .unwrap();
    assert_eq!(
        s["source"],
        serde_json::json!({"kind": "audio-file", "label": "a.mp3"})
    );
    assert!(
        s.get("batch").is_none(),
        "a single job has no batch key: {s}"
    );
    let d = serde_json::to_value(new_snapshot(
        "j".into(),
        SourceKind::Directory,
        "songs".into(),
        Phase::Finished,
    ))
    .unwrap();
    assert_eq!(
        (d["source"]["kind"].as_str(), d["phase"].as_str()),
        (Some("directory"), Some("finished"))
    );

    let v = serde_json::to_value(ImportEvent::BatchFile {
        index: 0,
        total: 2,
        name: "a.flac".into(),
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "batch-file", "index": 0, "total": 2, "name": "a.flac"})
    );
    let failed = BatchItem {
        index: 1,
        name: "b.mp3".into(),
        outcome: BatchOutcome::Failed {
            stage: Stage::Prepare,
            message: "bad".into(),
        },
    };
    let v = serde_json::to_value(ImportEvent::BatchItem {
        item: failed.clone(),
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "batch-item", "item": {"index": 1, "name": "b.mp3", "result": "failed", "stage": "prepare", "message": "bad"}})
    );
    let batch = BatchSnapshot {
        dir: "/m".into(),
        files: vec!["a.flac".into(), "b.mp3".into()],
        current: None,
        items: vec![failed],
    };
    let v = serde_json::to_value(ImportEvent::Finished {
        batch: batch.clone(),
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"phase": "finished", "batch": {"dir": "/m", "files": ["a.flac", "b.mp3"], "current": null,
            "items": [{"index": 1, "name": "b.mp3", "result": "failed", "stage": "prepare", "message": "bad"}]}})
    );
    let mut snap = new_snapshot(
        "j".into(),
        SourceKind::Directory,
        "m".into(),
        Phase::Preparing,
    );
    snap.batch = Some(batch);
    assert_eq!(serde_json::to_value(snap).unwrap()["batch"]["dir"], "/m");
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
    assert!(
        !rig.state.is_running(),
        "a refused start must not leave the job running"
    );
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
    rig.wait("the end", |s| {
        matches!(s.phase, Phase::Saved | Phase::Failed) && !rig.state.is_running()
    })
}

fn check_sparse(rig: &Rig, done: &JobSnapshot) -> TrackRecord {
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let track = done.track.clone().unwrap();
    let names: Vec<&str> = track.stems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar"]);
    let tdir = rig.tracks_dir().join(&track.id);
    assert_eq!(
        Rig::names_in(&tdir.join("stems")),
        ["bass.flac", "drums.flac", "guitar.flac", "vocals.flac"]
    );
    assert_eq!(done.dropped.len(), 2);
    assert_eq!(
        done.dropped[0],
        DroppedStem {
            name: "piano".into(),
            audible_ms: 0
        }
    );
    // `other` has short loud bursts (peak -8 dBFS) but only 9.5 s audible
    assert_eq!(
        done.dropped[1],
        DroppedStem {
            name: "other".into(),
            audible_ms: 9500
        }
    );
    let ImportEvent::Saved { dropped, .. } =
        rig.last_event_where("saved", |e| matches!(e, ImportEvent::Saved { .. }))
    else {
        unreachable!()
    };
    assert_eq!(dropped, done.dropped);
    assert!(!rig.state.is_active(), "a finished job is not active");
    assert!(Repository::new(&rig.root).scan().problems.is_empty());
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    rig.check_clean();
    track
}

#[test]
fn empty_stems_are_dropped_from_the_saved_track() {
    let server = TestServer::start("sparse");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    let track = check_sparse(&rig, &done);
    assert_eq!(track.original, None);
    assert!(server.wait_for("method=DELETE"));
}

#[test]
fn empty_stems_are_dropped_with_keep_original_too() {
    let server = TestServer::start("sparse");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    let track = check_sparse(&rig, &done);
    assert_eq!(track.original.as_deref(), Some("original.flac"));
    assert!(rig
        .tracks_dir()
        .join(&track.id)
        .join("original.flac")
        .is_file());
}

#[test]
fn a_track_of_only_empty_stems_fails_and_keeps_the_prepared_audio() {
    let server = TestServer::start("silent");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    assert_eq!(done.phase, Phase::Failed);
    let e = done.error.clone().unwrap();
    assert_eq!(e.stage, Stage::Server);
    assert_eq!(
        e.message,
        "Every stem is empty (less than 15 s above -40 dBFS), so no track was saved"
    );
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
fn server_empty_stems_are_never_fetched() {
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
fn an_old_server_without_levels_gives_the_same_result_with_all_stems_fetched() {
    let server = TestServer::start_with("sparse", &["--no-stem-levels"]);
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    check_sparse(&rig, &done);
    assert_eq!(server.all_stem_gets(), 6);
    assert_eq!(server.stem_gets("piano"), 1);
    assert_eq!(server.stem_gets("other"), 1);
}

#[test]
fn all_empty_by_the_server_fetches_nothing() {
    let server = TestServer::start("silent");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, true);
    assert_eq!(done.phase, Phase::Failed);
    assert!(done
        .error
        .clone()
        .unwrap()
        .message
        .starts_with("Every stem is empty"));
    assert_eq!(server.all_stem_gets(), 0);
    assert!(Rig::names_in(&rig.tracks_dir()).is_empty());
    let tmp = rig.root.join("import-tmp");
    let job_dirs = Rig::names_in(&tmp);
    assert_eq!(job_dirs.len(), 1);
    assert!(tmp.join(&job_dirs[0]).join("audio.flac").is_file());
    rig.check_clean();
}

#[test]
fn an_undecodable_stem_is_fetched_and_kept_while_the_server_empty_one_is_not() {
    let server = TestServer::start("undecodable");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let names: Vec<String> = done
        .track
        .unwrap()
        .stems
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar", "piano"]);
    assert_eq!(done.dropped.len(), 1);
    assert_eq!(done.dropped[0].name, "other");
    assert_eq!(server.stem_gets("piano"), 1);
    assert_eq!(server.stem_gets("other"), 0);
}

fn check_boundary(done: &JobSnapshot) {
    assert_eq!(done.phase, Phase::Saved, "{:?}", done.error);
    let names: Vec<&str> = done
        .track
        .as_ref()
        .unwrap()
        .stems
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["vocals", "drums", "bass", "guitar"]);
    // dropped in server order (vocals, drums, bass, guitar, piano, other)
    assert_eq!(
        done.dropped,
        [
            DroppedStem {
                name: "piano".into(),
                audible_ms: 9500
            },
            DroppedStem {
                name: "other".into(),
                audible_ms: 14_900
            }
        ]
    );
}

#[test]
fn the_15_second_boundary_is_decided_by_the_server_report() {
    let server = TestServer::start("boundary");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    check_boundary(&done);
    assert_eq!(server.stem_gets("guitar"), 1);
    assert_eq!(server.stem_gets("other"), 0);
    assert_eq!(server.stem_gets("piano"), 0);
    assert_eq!(server.all_stem_gets(), 4);
}

#[test]
fn the_15_second_boundary_is_the_same_when_the_app_measures() {
    let server = TestServer::start_with("boundary", &["--no-stem-levels"]);
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    check_boundary(&done);
    assert_eq!(server.all_stem_gets(), 6);
}

#[test]
fn a_normal_extraction_drops_nothing() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let done = extract_to_end(&rig, &server, false);
    assert_eq!(done.phase, Phase::Saved);
    assert_eq!(done.track.unwrap().stems.len(), 6);
    assert!(done.dropped.is_empty());
    let ImportEvent::Saved { dropped, .. } =
        rig.last_event_where("saved", |e| matches!(e, ImportEvent::Saved { .. }))
    else {
        unreachable!()
    };
    assert!(dropped.is_empty());
    let v = serde_json::to_value(rig.events.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(v["dropped"], serde_json::json!([]));
}

// ---- the normalizing phase (gui-import-normalize) ----

impl Rig {
    /// The same rig, with the normalizing step slowed down by `ms` per progress call.
    fn paced(ms: u64) -> Rig {
        let mut rig = Rig::new();
        rig.state = ImportState::new(
            rig.tools.clone(),
            rig.lock.clone(),
            Duration::from_millis(50),
        )
        .with_normalize_pace(Duration::from_millis(ms));
        rig
    }

    /// The audio the active job goes on with.
    fn job_audio(&self) -> PathBuf {
        guard(&self.state.inner.job)
            .as_ref()
            .and_then(|j| j.audio.clone())
            .expect("the job has no audio")
    }

    /// The only job folder in `import-tmp`.
    fn job_dir(&self, prefix: &str) -> PathBuf {
        let dirs: Vec<String> = Self::names_in(&self.root.join("import-tmp"))
            .into_iter()
            .filter(|n| n.starts_with(prefix))
            .collect();
        assert_eq!(dirs.len(), 1, "{dirs:?}");
        self.root.join("import-tmp").join(&dirs[0])
    }

    fn normalizing_events(&self) -> Vec<f64> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ImportEvent::Normalizing { progress } => Some(*progress),
                _ => None,
            })
            .collect()
    }

    /// The result of the last saved batch item.
    fn batch_loudness_of_last(&self) -> Option<LoudnessResult> {
        match self.batch_items().last().map(|i| i.outcome.clone()) {
            Some(BatchOutcome::Saved { loudness, .. }) => Some(loudness),
            _ => None,
        }
    }

    fn ready_loudness(&self) -> Option<LoudnessResult> {
        self.events.lock().unwrap().iter().find_map(|e| match e {
            ImportEvent::Ready { loudness, .. } => Some(loudness.clone()),
            _ => None,
        })
    }
}

/// A 6 s stereo 1 kHz sine of about `lufs` LUFS (both channels equal: LUFS = 20 log10 amplitude),
/// or digital silence for `None`, written as 16-bit FLAC with the real ffmpeg.
fn generated(path: &Path, lufs: Option<f64>) -> PathBuf {
    let (src, filter) = match lufs {
        Some(l) => (
            "sine=frequency=1000:duration=6:sample_rate=44100".to_string(),
            format!("volume={}dB,pan=stereo|c0=c0|c1=c0", l + 18.0618),
        ),
        None => (
            "anullsrc=r=44100:cl=stereo:d=6".to_string(),
            "anull".to_string(),
        ),
    };
    let out = std::process::Command::new(tool("ffmpeg"))
        .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(src)
        .args(["-af", &filter, "-c:a", "flac", "-sample_fmt", "s16"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    path.to_path_buf()
}

#[test]
fn a_local_file_is_normalized_before_ready_and_that_mix_is_what_gets_extracted() {
    let server = TestServer::start("ok");
    let inputs = server.keep_inputs();
    let rig = Rig::new();
    rig.file("tagged.mp3").unwrap();
    let ready = rig.ready();
    let phases = rig.phases();
    let pos = |p: &str| phases.iter().position(|x| x == p).unwrap();
    assert!(pos("normalizing") < pos("ready"));
    assert_eq!(rig.normalizing_events()[0], 0.0);
    let loudness = ready.loudness.clone().unwrap();
    assert_eq!(rig.ready_loudness(), Some(loudness.clone()));
    assert_eq!(loudness.outcome, crate::loudness::Outcome::Normalized);
    let audio = rig.job_audio();
    assert_eq!(audio.file_name().unwrap(), "normalized.flac");
    let mix = fs::read(&audio).unwrap();
    rig.extract(&ready, &server, true).unwrap();
    let t = rig
        .wait("saved", |s| s.phase == Phase::Saved)
        .track
        .unwrap();
    assert_eq!(fs::read(inputs.join("001.flac")).unwrap(), mix);
    let original = rig.tracks_dir().join(&t.id).join("original.flac");
    assert_eq!(fs::read(original).unwrap(), mix);
    // the snapshot keeps the result through the extraction
    assert_eq!(rig.state.snapshot().unwrap().loudness, Some(loudness));
    rig.check_clean();
}

#[test]
fn a_video_file_and_a_url_are_normalized_and_the_input_sent_is_that_mix() {
    let server = TestServer::start("ok");
    let inputs = server.keep_inputs();
    let rig = Rig::new();
    rig.file("with-audio.mp4").unwrap();
    let ready = rig.ready();
    assert!(rig.phases().contains(&"normalizing".to_string()));
    assert!(ready.loudness.is_some());
    let mix = fs::read(rig.job_audio()).unwrap();
    rig.extract(&ready, &server, false).unwrap();
    rig.wait("saved", |s| s.phase == Phase::Saved);
    assert_eq!(fs::read(inputs.join("001.flac")).unwrap(), mix);

    let rig = Rig::new();
    rig.url(WATCH, false).unwrap();
    let ready = rig.ready();
    let phases = rig.phases();
    let pos = |p: &str| phases.iter().position(|x| x == p).unwrap();
    assert!(pos("preparing") < pos("normalizing") && pos("normalizing") < pos("ready"));
    assert!(ready.loudness.is_some());
    let audio = rig.job_audio();
    assert_eq!(audio, rig.job_dir("url-").join("normalized.flac"));
    let mix = fs::read(&audio).unwrap();
    rig.extract(&ready, &server, false).unwrap();
    rig.wait("saved", |s| s.phase == Phase::Saved);
    assert_eq!(fs::read(inputs.join("002.flac")).unwrap(), mix);
}

#[test]
fn a_url_resume_does_not_normalize_again_but_an_old_folder_is_normalized_then() {
    let rig = Rig::new();
    rig.url(WATCH, false).unwrap();
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
    let resume = || {
        // the app was closed in the edit pane and started again
        let state = ImportState::new(
            rig.tools.clone(),
            rig.lock.clone(),
            Duration::from_millis(50),
        );
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
        let end = Instant::now() + Duration::from_secs(30);
        while state.is_running() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        let snap = state.snapshot().unwrap();
        assert_eq!(snap.phase, Phase::Ready, "{:?}", snap.error);
        let n = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, ImportEvent::Normalizing { .. }))
            .count();
        (snap, n)
    };
    let (snap, n) = resume();
    assert_eq!(n, 0);
    assert_eq!(snap.loudness, first.loudness);
    assert_eq!(before, (stamp("normalized.flac"), stamp("normalize.json")));

    // a folder left by an older build: audio.flac only
    fs::remove_file(dir.join("normalized.flac")).unwrap();
    fs::remove_file(dir.join("normalize.json")).unwrap();
    let (snap, n) = resume();
    assert!(n > 0);
    assert_eq!(snap.loudness, first.loudness);
    assert_eq!(stamp("normalized.flac").0, before.0 .0);
    assert!(dir.join("normalize.json").is_file());
    rig.check_clean();
}

#[test]
fn cancel_while_normalizing_leaves_no_track_and_a_url_folder_keeps_only_its_inputs() {
    let rig = Rig::paced(50);
    let snap = rig.file("tagged.mp3").unwrap();
    rig.wait("normalizing", |s| s.phase == Phase::Normalizing);
    rig.state.cancel(&snap.job).unwrap();
    assert_eq!(rig.wait_idle("cancel").phase, Phase::Cancelled);
    rig.last_event_where("cancelled", |e| {
        matches!(
            e,
            ImportEvent::Cancelled {
                back_to: BackTo::Source
            }
        )
    });
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    assert!(Repository::new(&rig.root).scan().tracks.is_empty());
    rig.check_clean();

    let rig = Rig::paced(50);
    let snap = rig.url(WATCH, false).unwrap();
    rig.wait("normalizing", |s| s.phase == Phase::Normalizing);
    rig.state.cancel(&snap.job).unwrap();
    assert_eq!(rig.wait_idle("cancel").phase, Phase::Cancelled);
    let names = Rig::names_in(&rig.job_dir("url-"));
    for gone in [".normalized.flac.part", "normalized.flac", "normalize.json"] {
        assert!(!names.contains(&gone.to_string()), "{names:?}");
    }
    assert!(names.contains(&"audio.flac".to_string()), "{names:?}");
    assert!(
        names.iter().any(|n| n.starts_with("download.")),
        "{names:?}"
    );
    assert!(Repository::new(&rig.root).scan().tracks.is_empty());
    rig.check_clean();
}

#[test]
fn a_normalizing_failure_is_a_preparation_failure_and_removes_the_job_folder() {
    let rig = Rig::paced(50);
    rig.file("tagged.mp3").unwrap();
    rig.wait("normalizing", |s| s.phase == Phase::Normalizing);
    fs::create_dir(rig.job_dir("file-").join(".normalized.flac.part")).unwrap();
    let s = rig.wait_idle("the failure");
    assert_eq!(s.phase, Phase::Failed);
    let e = s.error.unwrap();
    assert_eq!(e.stage, Stage::Prepare);
    assert!(
        e.message.starts_with("Cannot normalize the audio:"),
        "{}",
        e.message
    );
    assert!(Rig::names_in(&rig.root.join("import-tmp")).is_empty());
    assert!(Repository::new(&rig.root).scan().tracks.is_empty());
    rig.check_clean();
}

#[test]
fn every_batch_file_is_normalized_and_the_result_is_reset_between_files() {
    let server = TestServer::start("ok");
    let rig = Rig::new();
    let dir = rig.sources.join("levels");
    fs::create_dir_all(&dir).unwrap();
    let files = vec![
        generated(&dir.join("a-quiet.flac"), Some(-24.0)),
        generated(&dir.join("b-target.flac"), Some(-14.0)),
        generated(&dir.join("c-silence.flac"), None),
    ];
    // the snapshot's result when each file starts
    let at_start: Arc<Mutex<Vec<Option<LoudnessResult>>>> = Arc::new(Mutex::new(Vec::new()));
    let (events, starts, state) = (
        rig.events.clone(),
        at_start.clone(),
        ImportState {
            inner: rig.state.inner.clone(),
        },
    );
    let sink: Sink = Arc::new(move |ev| {
        if matches!(ev, ImportEvent::BatchFile { .. }) {
            starts
                .lock()
                .unwrap()
                .push(state.snapshot().unwrap().loudness);
        }
        events.lock().unwrap().push(ev);
    });
    rig.state
        .start_batch(&rig.root, &dir, files, Some(&server.url), false, sink)
        .unwrap();
    let done = rig.wait_idle("the batch to end");
    assert_eq!(done.phase, Phase::Finished);
    let phases: Vec<String> = rig
        .phases()
        .into_iter()
        .filter(|p| p != "queued" && p != "working")
        .collect();
    let file = [
        "batch-file",
        "preparing",
        "normalizing",
        "uploading",
        "receiving",
        "saving",
        "batch-item",
    ];
    let expected: Vec<&str> = file
        .iter()
        .cycle()
        .take(21)
        .copied()
        .chain(["finished"])
        .collect();
    assert_eq!(phases, expected);
    use crate::loudness::Outcome::*;
    let outcomes: Vec<_> = rig
        .batch_items()
        .iter()
        .map(|i| match &i.outcome {
            BatchOutcome::Saved { loudness, .. } => loudness.outcome,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(outcomes, [Normalized, AtTarget, TooQuiet]);
    assert_eq!(*at_start.lock().unwrap(), [None, None, None]);
    assert_eq!(done.loudness, rig.batch_loudness_of_last());
    rig.check_clean();
}
