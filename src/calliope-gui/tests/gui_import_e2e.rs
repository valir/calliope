//! Import (stem extraction) GUI end-to-end tests. They need an X display and are run with:
//! `DISPLAY=:1 npm run test:gui` (or `cargo build -p calliope-stems && cargo test --features
//! e2e-hooks --test gui_import_e2e -- --ignored --test-threads=1`).
//!
//! Every test starts the e2e app and the real `calliope-stems` binary (with
//! `tests/support/stub-separator`) inside `unshare -rn` with only the loopback interface up, the
//! fake yt-dlp first on PATH, temp XDG dirs and HOME under `target/gui-e2e/<test>/` (see
//! `start_import_app`). Nothing reaches the internet, the LAN or the real model. The UI is driven
//! with the keyboard (Tab order from a click on the page heading) and the steps are awaited
//! through the app's `calliope-ui: import ...` log lines. After each test the files on disk
//! are checked: the library fixture and the media sources are byte-identical, no `.staging-*`
//! or `import-tmp` job folder is left, and the real user's config/data folders are untouched.

mod common;

use common::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const URL_OK: &str = "https://media.example/watch?v=ok";
const LONG: Duration = Duration::from_secs(40);

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg") && have("ip") && have("unshare") && have("ffmpeg")
}

/// What the test must not change: the real user's app folders.
fn outside_fingerprint() -> BTreeMap<String, (u64, u64)> {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut m = BTreeMap::new();
    fn walk(p: &Path, m: &mut BTreeMap<String, (u64, u64)>) {
        let Ok(md) = std::fs::symlink_metadata(p) else { return };
        let mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
        m.insert(p.display().to_string(), (md.len(), mtime));
        if md.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), m);
                }
            }
        }
    }
    for d in [".config/app.calliope.gui", ".local/share/app.calliope.gui", ".local/share/calliope", ".cache/app.calliope.gui"] {
        walk(&home.join(d), &mut m);
    }
    m
}

/// A running import test: the app, the library as it was and the guards.
struct Rig {
    dirs: Dirs,
    app: App,
    wid: String,
    repo_before: BTreeMap<String, Vec<u8>>,
    media_before: BTreeMap<String, Vec<u8>>,
    outside_before: BTreeMap<String, (u64, u64)>,
}

impl Rig {
    fn new(test: &str, stub: &str, keep_original: bool, answers: &[String]) -> Rig {
        let outside_before = outside_fingerprint();
        let dirs = import_dirs(test);
        let repo_before = snapshot(&dirs.repo());
        let media_before = snapshot(&dirs.base().join("media"));
        let answers: Vec<String> = answers.iter().map(|a| a.replace("{media}", &dirs.base().join("media").display().to_string())).collect();
        let (app, wid) = start_import_app(&dirs, stub, keep_original, &answers);
        Rig { dirs, app, wid, repo_before, media_before, outside_before }
    }

    fn wait(&mut self, what: &str) -> String {
        self.app.wait_line(|l| l.contains(what), LONG)
    }

    fn wait_ui(&mut self, what: &str) -> String {
        let full = format!("calliope-ui: import {what}");
        self.app.wait_line(|l| l.contains(&full), LONG)
    }

    /// Click on the page heading: the next Tab goes to the first control of the page.
    fn top(&self) {
        let s = Command::new("xdotool").args(["mousemove", "--window", &self.wid, "700", "55", "click", "1"]).status().unwrap();
        assert!(s.success());
        sleep_ms(150);
    }

    fn key(&self, keys: &str) {
        self.app.key(&self.wid, keys);
    }

    /// Alt+2, "Stem Extraction", then the source radio (0 = URL, 1 = audio, 2 = video).
    fn open_source(&mut self, radio: usize) {
        self.key("alt+2");
        self.app.wait_view("import");
        self.top();
        self.key("Tab Return"); // Stem Extraction
        self.wait_ui("mode=stem-extraction");
        self.key("Tab Tab"); // Back, the radio group
        match radio {
            0 => self.key("space"),
            n => self.key(&format!("{}space", "Down ".repeat(n))), // arrows only move the focus
        }
        self.wait_ui(["source=url", "source=audio", "source=video"][radio]);
    }

    /// From the source page with the URL radio chosen: type the URL and press Enter.
    fn enter_url(&self, url: &str) {
        self.key("Tab"); // the URL box
        self.app.typ(&self.wid, url);
        settle();
        self.key("Return");
    }

    /// Source page, audio/video radio chosen: Browse.
    fn browse(&self) {
        self.key("Tab Return");
    }

    /// On the edit pane: the Album box.
    fn type_album(&self, album: &str) {
        self.top();
        self.key("Tab Tab ctrl+a"); // Band, Album
        self.app.typ(&self.wid, album);
    }

    /// On the edit pane: the Extract button (Band..Copyright, "Change in Settings", Extract).
    fn extract(&self) {
        self.top();
        self.key(&"Tab ".repeat(9));
        self.key("Return");
    }

    /// On the progress page / source page while a job runs: Cancel is the first enabled control.
    fn cancel(&self) {
        self.top();
        self.key("Tab Return");
    }

    fn saved(&mut self) -> (String, serde_json::Value) {
        let l = self.wait_ui("saved id=");
        let id = field(&l, "id");
        assert_eq!(field(&l, "stems"), "6", "{l}");
        let json = self.dirs.json(&id);
        (id, json)
    }

    /// Disk checks after any test: sources, fixtures, leftovers, the real user's folders.
    fn check_disk(&self, new_track: Option<&str>) {
        self.check_disk_with(new_track, &[]);
    }

    /// `leftover`: prefixes of `import-tmp` folders that may stay: `url-` (a failed or cancelled
    /// URL download keeps its job file and partial download for the resume prompt) and `file-`
    /// (the prepared audio of a job interrupted by closing the app; the next import start
    /// removes it). Nothing else may stay.
    fn check_disk_with(&self, new_track: Option<&str>, leftover: &[&str]) {
        let repo = self.dirs.repo();
        let now = snapshot(&repo);
        let new_prefix = new_track.map(|id| format!("tracks/{id}/"));
        for (k, v) in &self.repo_before {
            assert_eq!(now.get(k), Some(v), "existing file changed or vanished: {k}");
        }
        for k in now.keys() {
            if self.repo_before.contains_key(k) {
                continue;
            }
            let ok = new_prefix.as_ref().is_some_and(|p| k.starts_with(p.as_str()))
                || leftover.iter().any(|p| k.starts_with(&format!("import-tmp/{p}")));
            assert!(ok, "unexpected new file in the repository: {k}");
        }
        assert_eq!(snapshot(&self.dirs.base().join("media")), self.media_before, "a source file changed");
        let tracks = repo.join("tracks");
        for e in std::fs::read_dir(&tracks).unwrap() {
            let n = e.unwrap().file_name().to_string_lossy().to_string();
            assert!(!n.starts_with(".staging"), "staging folder left: {n}");
        }
        let left: Vec<_> = std::fs::read_dir(self.dirs.import_tmp())
            .map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        assert!(left.iter().all(|n| leftover.iter().any(|p| n.starts_with(p))), "import-tmp not clean: {left:?}");
        assert!(self.dirs.trash().is_empty(), "something was trashed: {:?}", self.dirs.trash());
        assert_eq!(outside_fingerprint(), self.outside_before, "the real user's folders changed");
        self.app.no_csp_violation();
    }

    /// The new track folder has the expected shape.
    fn check_stem_track(&self, id: &str, json: &serde_json::Value, original: bool) {
        assert_eq!(json["schema_version"], 2);
        assert_eq!(json["type"], "stem");
        assert_eq!(json["id"], id);
        let stems = json["stems"].as_array().unwrap();
        let names: Vec<&str> = stems.iter().map(|s| s["name"].as_str().unwrap()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, STEM_NAMES);
        let dir = self.dirs.track_dir(id);
        for s in stems {
            let f = dir.join(s["file"].as_str().unwrap());
            assert!(std::fs::read(&f).unwrap().starts_with(b"fLaC"), "{} is not FLAC", f.display());
        }
        let mut in_stems: Vec<String> = std::fs::read_dir(dir.join("stems")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        in_stems.sort();
        assert_eq!(in_stems.len(), 6, "{in_stems:?}");
        assert!(json["stem_model"].is_string(), "{json}");
        if original {
            assert_eq!(json["original"], "original.flac");
            assert!(std::fs::read(dir.join("original.flac")).unwrap().starts_with(b"fLaC"));
        } else {
            assert!(json["original"].is_null(), "{json}");
            assert!(!dir.join("original.flac").exists());
        }
        let mut top: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        top.sort();
        let mut want = vec!["stems".to_string(), "track.json".to_string()];
        if original {
            want.insert(0, "original.flac".into());
        }
        assert_eq!(top, want);
    }

    /// No stray process of this test except the server (killed by the drop guard).
    fn assert_no_children(&self) {
        let bad: Vec<_> = self
            .app
            .leftovers()
            .into_iter()
            .filter(|(_, c)| is_tool_child(c))
            .collect();
        assert!(bad.is_empty(), "children left: {bad:?}");
    }
}

/// A yt-dlp, ffmpeg, ffprobe or stub-separator process (not the server that merely has the
/// stub's path in its arguments).
fn is_tool_child(cmd: &str) -> bool {
    if cmd.contains("calliope-stems") {
        return false;
    }
    cmd.split(' ').filter(|w| !w.is_empty()).take(3).any(|w| {
        let base = w.rsplit('/').next().unwrap_or(w);
        ["yt-dlp", "ffmpeg", "ffprobe", "stub-separator"].contains(&base)
    })
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Polls `f` for up to `secs` seconds.
fn until(secs: u64, what: &str, f: impl Fn() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(std::time::Instant::now() < end, "timeout: {what}");
        sleep_ms(100);
    }
}

#[test]
#[ignore]
fn import_url_happy_path() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_url_happy_path", "ok", false, &[]);
    r.open_source(0);
    shot("import-source-url");
    r.enter_url(URL_OK);
    r.wait_ui("phase=downloading");
    r.wait_ui("phase=ready");
    settle();
    shot("import-edit-url");
    r.type_album("Midnight Run");
    settle();
    r.extract();
    r.wait_ui("phase=uploading");
    r.wait_ui("phase=working");
    shot("import-working");
    let (id, json) = r.saved();
    settle();
    shot("import-saved");
    r.check_stem_track(&id, &json, false);
    assert_eq!(json["album"], "Midnight Run");
    assert_eq!(json["title"], "Night Drive");
    assert_eq!(json["band"], "The Example Band");
    assert_eq!(json["year"], 2020);
    assert_eq!(json["source_url"], "https://media.example/watch?v=abc123");
    // The Library shows the new track (with its "S" badge).
    r.top();
    r.key("Tab Return"); // Show in Library
    r.app.wait_view("library");
    r.app.wait_line(|l| l.contains("library root=") && field(l, "tracks") == "7", LONG);
    settle();
    shot("import-library-badge");
    let fake = r.dirs.ytdlp_log();
    assert_eq!(fake.lines().count(), 1, "{fake}");
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_url_invalid() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_url_invalid", "ok", false, &[]);
    r.open_source(0);
    r.enter_url("not a url");
    let l = r.wait_ui("error stage=ui");
    assert!(l.contains("message=Entered URL is invalid"), "{l}");
    settle();
    shot("import-url-invalid");
    assert!(r.dirs.ytdlp_log().is_empty(), "yt-dlp ran for an invalid URL");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("import phase=")), "a job started");
    r.check_disk(None);
}

#[test]
#[ignore]
fn import_url_http_403() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_url_http_403", "ok", false, &[]);
    r.open_source(0);
    r.enter_url("https://media.example/http403");
    let l = r.wait_ui("error stage=download");
    assert!(l.contains("message=Error 403 when attempting download"), "{l}");
    settle();
    shot("import-url-403");
    r.check_disk_with(None, &["url-"]);
    r.assert_no_children();
}

/// Cancel a slow download, enter the URL again: the prompt; `resume` picks Resume or Start Over.
fn resume_flow(test: &str, resume: bool) {
    let mut r = Rig::new(test, "ok", false, &[]);
    r.open_source(0);
    r.enter_url("https://media.example/slow?v=1");
    r.wait_ui("phase=downloading");
    // Wait until some bytes are on disk, then cancel.
    let part = || {
        std::fs::read_dir(r.dirs.import_tmp())
            .into_iter()
            .flatten()
            .flatten()
            .any(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten().any(|f| f.file_name().to_string_lossy().ends_with(".part")))
    };
    until(20, "a partial download", part);
    shot("import-downloading");
    r.cancel();
    r.wait_ui("phase=cancelled");
    sleep_ms(500);
    r.top();
    r.key("Tab Tab Tab Return"); // Back, the radio, the URL box: Enter
    let l = r.wait_ui("prompt=incomplete-download");
    assert!(field(&l, "bytes").parse::<u64>().unwrap() > 0, "{l}");
    settle();
    shot("import-resume-prompt");
    r.top();
    r.key(if resume { "Tab Tab Return" } else { "Tab Tab Tab Return" }); // Back, Resume [, Start Over]
    r.wait_ui(if resume { "prompt-answer=resume" } else { "prompt-answer=start-over" });
    r.wait_ui("phase=ready");
    let fake: Vec<serde_json::Value> = r.dirs.ytdlp_log().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(fake.len(), 2, "{fake:?}");
    let argv = |v: &serde_json::Value| v["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect::<Vec<_>>();
    if resume {
        assert!(argv(&fake[1]).iter().any(|a| a == "--continue"), "{:?}", fake[1]);
        assert!(fake[1]["resume_from"].as_u64().unwrap() > 0, "{:?}", fake[1]);
    } else {
        assert!(!argv(&fake[1]).iter().any(|a| a == "--continue"), "{:?}", fake[1]);
        assert_eq!(fake[1]["resume_from"].as_u64().unwrap_or(0), 0, "{:?}", fake[1]);
    }
    settle();
    shot(if resume { "import-resumed" } else { "import-started-over" });
    // Finish the import: the temp folder is cleaned.
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_url_resume_download() {
    if !ready() {
        return;
    }
    resume_flow("import_url_resume_download", true);
}

#[test]
#[ignore]
fn import_url_start_over() {
    if !ready() {
        return;
    }
    resume_flow("import_url_start_over", false);
}

#[test]
#[ignore]
fn import_local_audio() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_local_audio", "ok", false, &["import-audio {media}/tagged.ogg".into()]);
    r.open_source(1);
    shot("import-source-audio");
    r.browse();
    r.wait("dialog kind=import-audio result=picked");
    r.wait_ui("phase=ready");
    settle();
    shot("import-edit-audio");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    assert_eq!(json["title"], "Glass Harbour", "fields are prefilled from the tags");
    assert_eq!(json["band"], "The Example Band");
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_local_video() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_local_video", "ok", false, &["import-video {media}/with-audio.mp4".into()]);
    r.open_source(2);
    shot("import-source-video");
    r.browse();
    r.wait("dialog kind=import-video result=picked");
    r.wait_ui("phase=ready");
    settle();
    shot("import-edit-video");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    assert_eq!(json["title"], "Clip Title");
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_video_without_audio() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_video_without_audio", "ok", false, &["import-video {media}/no-audio.mp4".into()]);
    r.open_source(2);
    r.browse();
    let l = r.wait_ui("error stage=");
    assert!(l.ends_with("message=Selected file no-audio.mp4 has no audio track"), "{l}");
    settle();
    shot("import-video-no-audio");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("phase=Ready")), "a job became ready");
    r.check_disk(None);
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_sixteen_minute_file_refused() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_sixteen_minute_file_refused", "ok", false, &["import-audio {media}/long.flac".into()]);
    r.open_source(1);
    r.browse();
    let l = r.wait_ui("error stage=");
    assert!(l.ends_with("message=Tracks longer than 15 minutes are not supported"), "{l}");
    settle();
    shot("import-too-long");
    // Nothing was sent: the server saw only the start-up health check.
    let log = r.dirs.stems_log();
    assert!(!log.contains("method=POST") && !log.contains("method=PUT") && !log.contains("job id="), "{log}");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("phase=uploading")));
    r.check_disk(None);
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_cancel_while_working() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_cancel_while_working", "slow", false, &["import-audio {media}/untagged.flac".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    r.extract();
    r.wait_ui("phase=working");
    settle();
    shot("import-working-slow");
    r.cancel();
    let l = r.wait_ui("phase=cancelled");
    assert!(l.contains("phase=cancelled"));
    until(10, "the server job deleted", || r.dirs.stems_log().contains(" deleted") || r.dirs.stems_log().contains("method=DELETE"));
    settle();
    shot("import-cancelled-back-to-edit");
    // Back on the edit pane: nothing was saved; Extract works again and finishes.
    assert!(r.dirs.track_ids().len() == 6, "{:?}", r.dirs.track_ids());
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_keep_original() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_keep_original", "ok", false, &["import-audio {media}/tagged.mp3".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    settle();
    // "Change in Settings" (after Copyright), then the address box has the focus: Save, Test
    // connection, the switch.
    r.top();
    r.key(&"Tab ".repeat(8));
    r.key("Return");
    r.app.wait_view("settings");
    settle();
    r.key("Tab Tab Tab space");
    r.wait("calliope-ui: settings keep_original=true");
    let s: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dirs.app_config().join("settings.json")).unwrap()).unwrap();
    assert_eq!(s["keep_original"], true);
    shot("import-settings-keep-original");
    r.key("alt+2");
    settle();
    shot("import-edit-keep-original");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, true);
    assert_eq!(json["title"], "Glass Harbour");
    assert_eq!(json["album"], "Test Pressings");
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_close_window_during_extraction() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_close_window_during_extraction", "hang", false, &["import-audio {media}/untagged.flac".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    r.extract();
    r.wait_ui("phase=working");
    until(10, "the stub separator running", || r.app.leftovers().iter().any(|(_, c)| c.contains("stub-separator")));
    let wid = r.wid.clone();
    r.app.close_gracefully(&wid);
    until(10, "no child process left", || {
        !r.app.leftovers().iter().any(|(_, c)| is_tool_child(c) || runs_program(c, "calliope-gui"))
    });
    until(10, "the DELETE reaching the server", || r.dirs.stems_log().contains("method=DELETE"));
    r.assert_no_children();
    assert!(r.dirs.track_ids().len() == 6);
    // The interrupted job's prepared audio stays in import-tmp (removed at the next import start).
    r.check_disk_with(None, &["file-"]);
}

#[test]
#[ignore]
fn import_network_is_loopback_only() {
    if !ready() {
        return;
    }
    let r = Rig::new("import_network_is_loopback_only", "ok", false, &[]);
    assert_loopback_only(&r.app);
    // From the test's own namespace the app's server is not reachable, and from a fresh
    // namespace nothing outside loopback exists.
    let o = Command::new("unshare").args(["-rn", "sh", "-c", "ip -o link | cut -d: -f2 | tr -d ' '"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "lo");
}
