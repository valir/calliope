//! Import (stem extraction) GUI end-to-end tests. They need an X display and are run with:
//! `npm run test:gui` (or `cargo build -p calliope-stems && cargo test --features
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
        Rig::build(test, stub, keep_original, answers, serde_json::json!({}))
    }

    /// `extra`: settings.json fields written before the start (`{media}` in a string is replaced
    /// by the media folder).
    fn new_with_settings(test: &str, stub: &str, answers: &[String], extra: serde_json::Value) -> Rig {
        Rig::build(test, stub, false, answers, extra)
    }

    fn build(test: &str, stub: &str, keep_original: bool, answers: &[String], extra: serde_json::Value) -> Rig {
        let outside_before = outside_fingerprint();
        let dirs = import_dirs(test);
        let media_dir = dirs.base().join("media").display().to_string();
        let extra: serde_json::Value = serde_json::from_str(&extra.to_string().replace("{media}", &media_dir)).unwrap();
        std::fs::create_dir_all(dirs.app_config()).unwrap();
        std::fs::write(dirs.app_config().join("settings.json"), serde_json::to_vec(&extra).unwrap()).unwrap();
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

    /// Alt+2, "Stem Extraction", then the source radio (0 = URL, 1 = audio, 2 = video, 3 = directory).
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
        self.wait_ui(["source=url", "source=audio", "source=video", "source=directory"][radio]);
    }

    /// From the source page with the URL radio chosen: type the URL and press Enter.
    fn enter_url(&self, url: &str) {
        self.key("Tab"); // the URL box
        self.app.typ(&self.wid, url);
        settle();
        self.key("Return");
    }

    /// Source page, audio/video radio chosen: Browse fills the box (box, Browse); Extract then has
    /// the focus and Return imports.
    fn browse(&mut self) {
        self.key("Tab Tab Return");
        self.wait("result=picked");
        self.wait_ui("path-picked");
        settle();
        self.key("Return");
    }

    /// Source page, audio/video radio chosen: type the path into the box, then Enter.
    fn type_path(&self, p: &str) {
        self.key("Tab ctrl+a");
        self.app.typ(&self.wid, p);
        settle();
        self.key("Return");
    }

    /// The path box already has the focus (after an error): replace its text and press Enter.
    fn retype_path(&self, p: &str) {
        self.key("ctrl+a");
        self.app.typ(&self.wid, p);
        settle();
        self.key("Return");
    }

    fn media(&self) -> String {
        self.dirs.base().join("media").display().to_string()
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
        let (id, json, _) = self.saved_line("6");
        (id, json)
    }

    /// The "saved" log line with the expected stem count; returns the id, track.json and the line.
    fn saved_line(&mut self, stems: &str) -> (String, serde_json::Value, String) {
        let l = self.wait_ui("saved id=");
        let id = field(&l, "id");
        assert_eq!(field(&l, "stems"), stems, "{l}");
        let json = self.dirs.json(&id);
        (id, json, l)
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
        let ids: Vec<String> = new_track.map(String::from).into_iter().collect();
        self.check_disk_tracks(&ids, leftover);
    }

    /// Like `check_disk_with`, for any number of new tracks.
    fn check_disk_tracks(&self, new_tracks: &[String], leftover: &[&str]) {
        let repo = self.dirs.repo();
        let now = snapshot(&repo);
        let new_prefixes: Vec<String> = new_tracks.iter().map(|id| format!("tracks/{id}/")).collect();
        for (k, v) in &self.repo_before {
            assert_eq!(now.get(k), Some(v), "existing file changed or vanished: {k}");
        }
        for k in now.keys() {
            if self.repo_before.contains_key(k) {
                continue;
            }
            let ok = new_prefixes.iter().any(|p| k.starts_with(p.as_str()))
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
fn import_drops_empty_stems() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_drops_empty_stems", "sparse", false, &["import-audio {media}/tagged.ogg".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    r.extract();
    let (id, json, line) = r.saved_line("4");
    assert!(line.contains("dropped=piano,other"), "{line}");
    settle();
    shot("import-done-dropped");
    let want = "stem=other dropped audible_s=9.5 min_audible_s=15.0 level_dbfs=-40 source=server";
    assert!(r.app.all_lines().iter().any(|l| l.contains(want)), "no `{want}` in the app's stderr");
    assert_eq!(json["type"], "stem");
    let mut names: Vec<&str> = json["stems"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["bass", "drums", "guitar", "vocals"]);
    let dir = r.dirs.track_dir(&id);
    for s in json["stems"].as_array().unwrap() {
        let f = dir.join(s["file"].as_str().unwrap());
        assert!(std::fs::read(&f).unwrap().starts_with(b"fLaC"), "{} is not FLAC", f.display());
    }
    let mut on_disk: Vec<String> = std::fs::read_dir(dir.join("stems")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    on_disk.sort();
    assert_eq!(on_disk.len(), 4, "{on_disk:?}");
    for n in ["piano", "other"] {
        assert!(!on_disk.iter().any(|f| f.contains(n)), "{on_disk:?}");
    }
    // The server's request log proves the empty stems were never downloaded.
    let log = r.dirs.stems_log();
    let fetches = |name: &str| {
        log.lines()
            .filter(|l| l.contains("request method=GET") && l.split_whitespace().any(|w| w.strip_prefix("path=").is_some_and(|p| p.ends_with(&format!("/stems/{name}")))))
            .count()
    };
    for n in ["piano", "other"] {
        assert_eq!(fetches(n), 0, "{n} was fetched:\n{log}");
    }
    for n in ["vocals", "drums", "bass", "guitar"] {
        assert_eq!(fetches(n), 1, "{n} fetch count:\n{log}");
    }
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_boundary_stems() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_boundary_stems", "boundary", false, &["import-audio {media}/tagged.ogg".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    r.extract();
    let (id, json, line) = r.saved_line("4");
    assert!(line.contains("dropped=piano,other"), "{line}");
    settle();
    shot("import-done-boundary");
    let mut names: Vec<&str> = json["stems"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
    names.sort();
    assert_eq!(names, ["bass", "drums", "guitar", "vocals"]);
    let log = r.dirs.stems_log();
    for n in ["other", "piano"] {
        let fetched = log.lines().any(|l| l.contains("request method=GET") && l.split_whitespace().any(|w| w.strip_prefix("path=").is_some_and(|p| p.ends_with(&format!("/stems/{n}")))));
        assert!(!fetched, "{n} was fetched:\n{log}");
    }
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
fn import_typed_audio_path() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_typed_audio_path", "ok", false, &[]);
    r.open_source(1);
    let p = format!("{}/tagged.mp3", r.media());
    r.key("Tab ctrl+a");
    r.app.typ(&r.wid, &p);
    settle();
    shot("import-path-audio");
    r.key("Return");
    r.wait_ui("phase=ready");
    settle();
    shot("import-path-audio-edit");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("dialog kind=import")), "no dialog may open");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_typed_video_path() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_typed_video_path", "ok", false, &[]);
    r.open_source(2);
    let p = format!("{}/with-audio.mp4", r.media());
    r.key("Tab ctrl+a");
    r.app.typ(&r.wid, &p);
    settle();
    shot("import-path-video");
    r.key("Return");
    r.wait_ui("phase=ready");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
    assert_eq!(json["title"], "Clip Title");
    r.check_disk(Some(&id));
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_typed_path_errors() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_typed_path_errors", "ok", false, &[]);
    let media = r.media();
    r.open_source(1);
    r.type_path(&format!("{media}/nothing-here.mp3"));
    let l = r.wait_ui("error stage=ui");
    assert!(l.ends_with(&format!("message=No file found at {media}/nothing-here.mp3")), "{l}");
    r.retype_path(&media);
    let l = r.wait_ui("error stage=ui");
    assert!(l.ends_with(&format!("message={media} is a folder, not a file")), "{l}");
    // switch to Local Video File (the radio group is before the box) and type an audio file
    r.key("shift+Tab Down space");
    r.wait_ui("source=video");
    r.type_path(&format!("{media}/tagged.mp3"));
    let l = r.wait_ui("error stage=ui");
    assert!(l.ends_with("message=\"tagged.mp3\" is not a video file"), "{l}");
    settle();
    shot("import-path-error");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("import job=")), "a job was started");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("phase=ready")), "a job became ready");
    r.check_disk(None);
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_browse_fills_path() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_browse_fills_path", "ok", false, &["import-audio {media}/tagged.ogg".into()]);
    r.open_source(1);
    r.key("Tab Tab Return");
    r.wait("dialog kind=import-audio result=picked");
    r.wait_ui("path-picked");
    settle();
    shot("import-path-browsed");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("phase=preparing")), "Browse must not start the import");
    r.key("Return");
    r.wait_ui("phase=ready");
    r.extract();
    let (id, json) = r.saved();
    r.check_stem_track(&id, &json, false);
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

/// The expected `dialog start` line, as the app logs it.
fn dialog_start(kind: &str, from: &str, dir: &str) -> String {
    format!("calliope: dialog start kind={kind} from={from} dir={dir}")
}

impl Rig {
    fn wait_dialog_start(&mut self, kind: &str, from: &str, dir: &str) {
        let want = dialog_start(kind, from, dir);
        self.app.wait_line(|l| l.trim_end().ends_with(&want), LONG);
    }

    /// Focus on the path box (e.g. after a Browse): replace the text with `p` and leave the focus
    /// on Browse.
    fn type_in_box_then_browse_focus(&self, p: &str) {
        self.key("shift+Tab ctrl+a");
        self.app.typ(&self.wid, p);
        settle();
        self.key("Tab");
    }
}

#[test]
#[ignore]
fn import_browse_start_folders() {
    if !ready() {
        return;
    }
    let answers: Vec<String> = ["import-audio CANCEL"; 4].iter().map(|s| s.to_string()).chain(["import-directory {media}/album".to_string()]).collect();
    let mut r = Rig::new_with_settings("import_browse_start_folders", "ok", &answers, serde_json::json!({"last_import_dir": "{media}"}));
    let media = r.media();
    r.open_source(1);
    // empty box: the remembered folder
    r.key("Tab Tab Return");
    r.wait_dialog_start("import-audio", "last", &media);
    // a typed file: its folder
    r.type_in_box_then_browse_focus(&format!("{media}/tagged.mp3"));
    r.key("Return");
    r.wait_dialog_start("import-audio", "typed", &media);
    // a typed folder: the folder itself
    r.type_in_box_then_browse_focus(&format!("{media}/album"));
    r.key("Return");
    r.wait_dialog_start("import-audio", "typed", &format!("{media}/album"));
    // a missing file: silently the remembered folder
    r.type_in_box_then_browse_focus(&format!("{media}/nope.mp3"));
    r.key("Return");
    r.wait_dialog_start("import-audio", "last", &media);
    assert!(!r.app.all_lines().iter().any(|l| l.contains("import job=")), "a job started");

    // Local Directory: the folder dialog; the choice fills the box
    r.key("shift+Tab shift+Tab Down Down space");
    r.wait_ui("source=directory");
    r.key("Tab Tab Return");
    r.wait("dialog kind=import-directory result=picked");
    r.wait_ui("path-picked kind=directory");
    settle();
    shot("import-source-directory");
    // the box has the folder; Extract has the focus. Empty folder, then a file.
    r.key("shift+Tab shift+Tab");
    r.retype_path(&format!("{media}/no-songs"));
    let l = r.wait_ui("error stage=ui");
    assert!(l.ends_with(&format!("message=Directory {media}/no-songs has no audio files")), "{l}");
    settle();
    shot("import-directory-no-audio");
    r.retype_path(&format!("{media}/tagged.mp3"));
    let l = r.wait_ui("error stage=ui");
    assert!(l.ends_with(&format!("message={media}/tagged.mp3 is not a folder")), "{l}");
    assert!(!r.app.all_lines().iter().any(|l| l.contains("import job=") || l.contains("import batch")), "a job started");
    r.check_disk(None);
    r.assert_no_children();
}

/// The `calliope: track` ids that are not in the starting library.
fn new_tracks(r: &Rig) -> Vec<(String, serde_json::Value)> {
    let before: Vec<String> = r.repo_before.keys().filter_map(|k| k.strip_prefix("tracks/").and_then(|k| k.split('/').next())).map(String::from).collect();
    r.dirs.track_ids().into_iter().filter(|id| !before.contains(id)).map(|id| (id.clone(), r.dirs.json(&id))).collect()
}

#[test]
#[ignore]
fn import_directory_batch() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_directory_batch", "seq:slow,ok", false, &["import-directory {media}/album".into(), "import-audio CANCEL".into()]);
    let media = r.media();
    let home = r.dirs.base().join("home").display().to_string();
    r.open_source(3);
    r.key("Tab Tab Return");
    r.wait_dialog_start("import-directory", "home", &home);
    r.wait_ui("path-picked kind=directory");
    settle();
    r.key("Return"); // Extract
    r.wait_ui("batch file=1/2 name=a.flac");
    r.wait_ui("phase=working"); // the first file is slow: still working at the screenshot
    settle();
    shot("import-batch-progress");
    r.wait_ui("batch item=1 result=saved");
    r.wait_ui("batch file=2/2 name=b.mp3");
    r.wait_ui("batch finished saved=2 failed=0");
    settle();
    shot("import-batch-summary");
    let tracks = new_tracks(&r);
    assert_eq!(tracks.len(), 2);
    let mut titles: Vec<String> = tracks.iter().map(|(_, j)| j["title"].as_str().unwrap().to_string()).collect();
    titles.sort();
    assert_eq!(titles, ["Glass Harbour", "a"]);
    for (id, j) in &tracks {
        r.check_stem_track(id, j, false);
    }
    let s: serde_json::Value = serde_json::from_slice(&std::fs::read(r.dirs.app_config().join("settings.json")).unwrap()).unwrap();
    assert_eq!(s["last_import_dir"], format!("{media}/album"), "the directory itself is remembered");
    for l in ["batch file=1/2 name=a.flac", "batch file=2/2 name=b.mp3"] {
        assert!(r.app.all_lines().iter().any(|x| x.contains(&format!("calliope: import {l}")) || x.contains(&format!("calliope-ui: import {l}"))), "{l}");
    }
    assert!(r.app.all_lines().iter().any(|l| l.contains("calliope: import batch job=") && l.contains("finished saved=2 failed=0")));
    // Import another track, Local Audio File, Browse with an empty box: the folder of the batch.
    r.top();
    r.key("Tab Tab Return"); // Show in Library, Import another track
    settle();
    r.top();
    r.key("Tab Tab Down space"); // Back, the radio group (nothing chosen: the first radio), Local Audio File
    r.wait_ui("source=audio");
    r.key("Tab Tab Return");
    r.wait_dialog_start("import-audio", "last", &format!("{media}/album"));
    let ids: Vec<String> = tracks.iter().map(|(id, _)| id.clone()).collect();
    r.check_disk_tracks(&ids, &[]);
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_directory_one_fails() {
    if !ready() {
        return;
    }
    let mut r = Rig::new_with_settings("import_directory_one_fails", "seq:ok,fail,ok", &[], serde_json::json!({"theme": "light"}));
    r.open_source(3);
    let three = format!("{}/three", r.media());
    r.type_path(&three);
    r.wait_ui("batch file=1/3 name=1.flac");
    r.wait_ui("phase=working");
    settle();
    shot("import-batch-progress-light");
    let l = r.wait_ui("batch item=2 result=failed stage=server");
    assert!(field(&l, "stage") == "server", "{l}");
    r.wait_ui("batch finished saved=2 failed=1");
    settle();
    shot("import-batch-failed-summary");
    size(&r.wid, 1024, 640);
    settle();
    shot("import-batch-failed-summary-small");
    let tracks = new_tracks(&r);
    assert_eq!(tracks.len(), 2);
    for (id, j) in &tracks {
        r.check_stem_track(id, j, false);
    }
    let ids: Vec<String> = tracks.iter().map(|(id, _)| id.clone()).collect();
    r.check_disk_tracks(&ids, &[]);
    r.assert_no_children();
}

#[test]
#[ignore]
fn import_directory_cancel() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("import_directory_cancel", "seq:ok,hang", false, &[]);
    r.open_source(3);
    let three = format!("{}/three", r.media());
    r.type_path(&three);
    r.wait_ui("batch item=1 result=saved");
    r.wait_ui("batch file=2/3 name=2.ogg");
    r.wait_ui("phase=working");
    size(&r.wid, 1024, 640);
    settle();
    shot("import-batch-progress-small");
    r.cancel();
    r.wait_ui("batch cancelled saved=1 of=3");
    settle();
    shot("import-batch-cancelled");
    until(10, "the server job deleted", || r.dirs.stems_log().contains("method=DELETE"));
    let tracks = new_tracks(&r);
    assert_eq!(tracks.len(), 1);
    r.check_stem_track(&tracks[0].0, &tracks[0].1, false);
    r.check_disk_tracks(&[tracks[0].0.clone()], &[]);
    r.assert_no_children();
}
