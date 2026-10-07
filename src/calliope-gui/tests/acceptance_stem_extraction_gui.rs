//! QA end-to-end checks of gui-stem-extraction on display :1 (independent of gui_import_e2e):
//! a screenshot walk-through of the acceptance criteria, SIGKILL of the real app mid-import,
//! and a malicious edge-AI server seen through the UI.
//!
//! Run with: `DISPLAY=:1 cargo test --features e2e-hooks --test acceptance_stem_extraction_gui
//! -- --ignored --test-threads=1` (after `cargo build -p calliope-stems`).
//!
//! The app and the server run inside `unshare -rn` (only loopback), temp XDG dirs and HOME under
//! `target/gui-e2e/<test>/`, the fake yt-dlp first on PATH, COPIES of the fixtures. The real
//! user's folders are fingerprinted before and after.

#![allow(dead_code)]

mod common;

use common::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const LONG: Duration = Duration::from_secs(40);

fn ready() -> bool {
    have_display() && have("xdotool") && have("i3-msg") && have("ip") && have("unshare") && have("ffmpeg") && have("python3")
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn until(secs: u64, what: &str, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < end, "timeout: {what}");
        sleep_ms(100);
    }
}

fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

fn real_user_fingerprint() -> BTreeMap<String, (u64, u64)> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut m = BTreeMap::new();
    fn walk(p: &Path, m: &mut BTreeMap<String, (u64, u64)>) {
        let Ok(md) = std::fs::symlink_metadata(p) else { return };
        let mt = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
        m.insert(p.display().to_string(), (md.len(), mt));
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

/// Starts the e2e app and a server inside `unshare -rn`. `server_cmd` is the shell command that
/// starts the edge-AI stand-in on 127.0.0.1:8765 (stderr to `<base>/stems.log`).
fn start_app(dirs: &Dirs, server_cmd: &str, keep_original: bool, answers: &[String]) -> (App, String) {
    assert!(dirs.base().starts_with(repo().join("target")), "temp dir is not under target/");
    let cfg = dirs.app_config();
    std::fs::create_dir_all(&cfg).unwrap();
    let settings = serde_json::json!({
        "repository_root": dirs.repo(),
        "edge_ai_url": format!("http://127.0.0.1:{STEMS_PORT}"),
        "keep_original": keep_original,
    });
    std::fs::write(cfg.join("settings.json"), serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
    let ans = dirs.base().join("answers");
    std::fs::write(&ans, answers.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    let tag = dirs.base().file_name().unwrap().to_string_lossy().to_string();
    kill_tagged(&tag);
    let _ = std::fs::remove_file(dirs.base().join("stems.log"));
    let script = format!(
        "ip link set lo up || exit 1; {server} 2> {log} & exec {bin}",
        server = server_cmd,
        log = sh_quote(&dirs.base().join("stems.log")),
        bin = sh_quote(Path::new(BIN)),
    );
    let xauth = std::env::var_os("XAUTHORITY").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".Xauthority"));
    let mut cmd = Command::new("unshare");
    cmd.args(["-rn", "sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dirs.base().join("config"))
        .env("XDG_DATA_HOME", dirs.base().join("data"))
        .env("XDG_CACHE_HOME", dirs.base().join("cache"))
        .env("XDG_STATE_HOME", dirs.base().join("state"))
        .env("XAUTHORITY", xauth)
        .env("HOME", dirs.base().join("home"))
        .env("PATH", format!("{}:{}", root().join("tests/support/bin").display(), std::env::var("PATH").unwrap_or_default()))
        .env("FAKE_YTDLP_LOG", dirs.base().join("ytdlp.log"))
        .env("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)
        .env("CALLIOPE_E2E_TAG", &tag);
    let mut app = App::spawn(cmd, Some(tag));
    app.wait_ready();
    if server_cmd.contains("python3") {
        sleep_ms(1000);
    } else {
        until(15, "the server starting", || dirs.stems_log().contains("listening"));
    }
    sleep_ms(300);
    app.wait_line(|l| l.contains("library root="), Duration::from_secs(20));
    let wid = app.wid();
    if !server_cmd.contains("python3") {
        assert_loopback_only(&app);
    }
    float(&wid);
    size(&wid, 1280, 800);
    (app, wid)
}

fn real_server(dirs: &Dirs, mode: &str) -> String {
    std::fs::create_dir_all(dirs.base().join("stems-work")).unwrap();
    std::fs::write(dirs.base().join("stems-work/stub-mode"), format!("{mode}\n")).unwrap();
    format!(
        "{} --listen 127.0.0.1:{STEMS_PORT} --work-dir {} --separator {}",
        sh_quote(&repo().join("target/debug/calliope-stems")),
        sh_quote(&dirs.base().join("stems-work")),
        sh_quote(&root().join("tests/support/stub-separator"))
    )
}

fn hostile_server(mode: &str) -> String {
    format!("python3 {} {STEMS_PORT} {mode}", sh_quote(&root().join("tests/support/hostile-edge-ai.py")))
}

struct Rig {
    dirs: Dirs,
    app_slot: Option<App>,
    wid: String,
    repo_before: BTreeMap<String, Vec<u8>>,
    media_before: BTreeMap<String, Vec<u8>>,
    user_before: BTreeMap<String, (u64, u64)>,
}

impl Rig {
    fn app(&self) -> &App {
        self.app_slot.as_ref().unwrap()
    }
    fn app_mut(&mut self) -> &mut App {
        self.app_slot.as_mut().unwrap()
    }
    fn new(test: &str, server: impl Fn(&Dirs) -> String, answers: &[String]) -> Rig {
        let user_before = real_user_fingerprint();
        let dirs = import_dirs(test);
        let repo_before = snapshot(&dirs.repo());
        let media = dirs.base().join("media");
        let media_before = snapshot(&media);
        let answers: Vec<String> = answers.iter().map(|a| a.replace("{media}", &media.display().to_string())).collect();
        let cmd = server(&dirs);
        let (app, wid) = start_app(&dirs, &cmd, false, &answers);
        Rig { dirs, app_slot: Some(app), wid, repo_before, media_before, user_before }
    }

    fn wait_ui(&mut self, what: &str) -> String {
        let full = format!("calliope-ui: import {what}");
        self.app_mut().wait_line(|l| l.contains(&full), LONG)
    }
    fn top(&self) {
        let s = Command::new("xdotool").args(["mousemove", "--window", &self.wid, "700", "55", "click", "1"]).status().unwrap();
        assert!(s.success());
        sleep_ms(150);
    }
    fn key(&self, keys: &str) {
        self.app().key(&self.wid, keys);
    }
    fn open_source(&mut self, radio: usize) {
        self.key("alt+2");
        self.app_mut().wait_view("import");
        self.top();
        self.key("Tab Return");
        self.wait_ui("mode=stem-extraction");
        self.key("Tab Tab");
        match radio {
            0 => self.key("space"),
            n => self.key(&format!("{}space", "Down ".repeat(n))),
        }
        self.wait_ui(["source=url", "source=audio", "source=video"][radio]);
    }
    fn enter_url(&self, url: &str) {
        self.key("Tab");
        self.app().typ(&self.wid, url);
        settle();
        self.key("Return");
    }
    fn browse(&self) {
        self.key("Tab Return");
    }
    fn extract(&self) {
        self.top();
        self.key(&"Tab ".repeat(9));
        self.key("Return");
    }
    fn kill_app(&mut self) {
        let _ = Command::new("kill").args(["-9", &self.app().pid().to_string()]).status();
        self.app_mut().wait_exit(Duration::from_secs(10));
    }
    /// A second run of the app on the same folders (the first one was killed).
    fn restart(&mut self, server_cmd: &str, answers: &[&str]) {
        // drop the old handle first: its Drop kills everything carrying the test's tag
        drop(self.app_slot.take());
        let media = self.dirs.base().join("media").display().to_string();
        let answers: Vec<String> = answers.iter().map(|a| a.replace("{media}", &media)).collect();
        let (app, wid) = start_app(&self.dirs, server_cmd, false, &answers);
        self.app_slot = Some(app);
        self.wid = wid;
    }
    /// After a SIGKILL of the app: no yt-dlp/ffmpeg/ffprobe of the dead app may survive. (A
    /// stub separator belongs to the server, which cannot know the app died.)
    fn assert_no_client_children(&self) {
        until(10, "no client tool processes left", || {
            self.app().leftovers().iter().all(|(_, c)| {
                let first: Vec<&str> = c.split(' ').filter(|w| !w.is_empty()).take(3).collect();
                !first.iter().any(|w| {
                    let b = w.rsplit('/').next().unwrap_or(w);
                    ["yt-dlp", "ffmpeg", "ffprobe"].contains(&b)
                })
            })
        });
    }
    fn assert_no_children(&self) {
        until(10, "no tool processes left", || {
            self.app().leftovers().iter().all(|(_, c)| {
                let first: Vec<&str> = c.split(' ').filter(|w| !w.is_empty()).take(3).collect();
                !first.iter().any(|w| {
                    let b = w.rsplit('/').next().unwrap_or(w);
                    ["yt-dlp", "ffmpeg", "ffprobe", "stub-separator"].contains(&b)
                }) || c.contains("calliope-stems")
            })
        });
    }
    /// Every file that existed before is still there, byte for byte; sources identical; the
    /// real user's folders untouched.
    fn assert_originals_intact(&self) {
        let now = snapshot(&self.dirs.repo());
        for (k, v) in &self.repo_before {
            assert_eq!(now.get(k), Some(v), "existing file changed or vanished: {k}");
        }
        assert_eq!(snapshot(&self.dirs.base().join("media")), self.media_before, "a source file changed");
        assert_eq!(real_user_fingerprint(), self.user_before, "the real user's folders changed");
        assert!(self.dirs.trash().is_empty());
    }
}

#[test]
#[ignore]
fn qa_walkthrough_with_screenshots() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("qa_walkthrough", |d| real_server(d, "slow"), &["import-audio {media}/tagged.mp3".into()]);
    // AC1: the Import tab shows a "Stem Extraction" button
    r.key("alt+2");
    r.app_mut().wait_view("import");
    settle();
    shot("qa-1-import-tab");
    // AC2/AC3: the select-source page, URL option shows the box
    r.top();
    r.key("Tab Return");
    r.wait_ui("mode=stem-extraction");
    settle();
    shot("qa-2-select-source");
    r.key("Tab Tab space");
    r.wait_ui("source=url");
    settle();
    shot("qa-3-url-box");
    // AC5: malformed URL -> accent text
    r.key("Tab");
    r.app().typ(&r.wid, "not a url");
    r.key("Return");
    let l = r.wait_ui("error stage=ui");
    assert!(l.contains("Entered URL is invalid"), "{l}");
    settle();
    shot("qa-4-invalid-url");
    // AC6/AC7: valid URL -> progress bar (slow fake download)
    r.key("ctrl+a");
    r.app().typ(&r.wid, "https://media.example/slow?v=qa");
    settle();
    r.key("Return");
    r.wait_ui("phase=downloading");
    until(20, "some bytes", || {
        std::fs::read_dir(r.dirs.import_tmp()).into_iter().flatten().flatten().any(|d| {
            std::fs::read_dir(d.path()).into_iter().flatten().flatten().any(|f| f.file_name().to_string_lossy().ends_with(".part") && f.metadata().map(|m| m.len() > 0).unwrap_or(false))
        })
    });
    shot("qa-5-download-progress");
    // AC: download complete -> temp file inside the repository, edit pane with metadata
    r.wait_ui("phase=ready");
    let tmp: Vec<String> = std::fs::read_dir(r.dirs.import_tmp()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(tmp.len(), 1, "{tmp:?}");
    assert!(std::fs::read_dir(r.dirs.import_tmp().join(&tmp[0])).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("download.")));
    settle();
    shot("qa-6-edit-pane");
    // AC: Extract -> "Working..." wheel when the server confirms start
    r.extract();
    r.wait_ui("phase=uploading");
    r.wait_ui("phase=working");
    settle();
    shot("qa-7-working");
    let l = r.wait_ui("saved id=");
    let id = field(&l, "id");
    assert_eq!(field(&l, "stems"), "6");
    // AC: temp file removed after processing
    assert!(std::fs::read_dir(r.dirs.import_tmp()).unwrap().next().is_none(), "import-tmp not empty");
    let json = r.dirs.json(&id);
    assert_eq!(json["type"], "stem");
    assert_eq!(json["schema_version"], 2);
    settle();
    shot("qa-8-saved");
    // AC: Library shows the S icon in front of the name
    r.top();
    r.key("Tab Return");
    r.app_mut().wait_view("library");
    r.app_mut().wait_line(|l| l.contains("library root=") && field(l, "tracks") == "7", LONG);
    settle();
    shot("qa-9-library-badge");
    // v1 fixtures are untouched
    r.assert_originals_intact_except_new(&id);
    r.assert_no_children();
}

impl Rig {
    fn assert_originals_intact_except_new(&self, new_id: &str) {
        let now = snapshot(&self.dirs.repo());
        for (k, v) in &self.repo_before {
            assert_eq!(now.get(k), Some(v), "existing file changed or vanished: {k}");
        }
        for k in now.keys() {
            assert!(self.repo_before.contains_key(k) || k.starts_with(&format!("tracks/{new_id}/")), "unexpected new file {k}");
        }
        assert_eq!(snapshot(&self.dirs.base().join("media")), self.media_before);
        assert_eq!(real_user_fingerprint(), self.user_before);
    }
}

#[test]
#[ignore]
fn qa_sigkill_during_download_then_resume_prompt() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("qa_kill_download", |d| real_server(d, "ok"), &[]);
    r.open_source(0);
    r.enter_url("https://media.example/slow?v=killme");
    r.wait_ui("phase=downloading");
    until(20, "some bytes", || {
        std::fs::read_dir(r.dirs.import_tmp()).into_iter().flatten().flatten().any(|d| {
            std::fs::read_dir(d.path()).into_iter().flatten().flatten().any(|f| f.file_name().to_string_lossy().ends_with(".part") && f.metadata().map(|m| m.len() > 0).unwrap_or(false))
        })
    });
    r.kill_app();
    r.assert_no_children();
    // the library as an old build would see it: untouched
    for (k, v) in &r.repo_before {
        assert_eq!(snapshot(&r.dirs.repo()).get(k), Some(v), "{k}");
    }
    // restart: the library loads with its 6 tracks and no problems; the same URL offers the prompt
    let cmd = real_server(&r.dirs, "ok");
    r.restart(&cmd, &[]);
    let lib = r.app().all_lines().into_iter().rev().find(|l| l.contains("library root=")).unwrap();
    assert_eq!(field(&lib, "tracks"), "6", "{lib}");
    assert_eq!(field(&lib, "problems"), "0", "{lib}");
    r.open_source(0);
    r.enter_url("https://media.example/slow?v=killme");
    let l = r.wait_ui("prompt=incomplete-download");
    assert!(field(&l, "bytes").parse::<u64>().unwrap() > 0, "{l}");
    settle();
    shot("qa-kill-resume-prompt");
    // Resume Download
    r.top();
    r.key("Tab Tab Return");
    r.wait_ui("prompt-answer=resume");
    r.wait_ui("phase=ready");
    r.extract();
    let l = r.wait_ui("saved id=");
    assert_eq!(field(&l, "stems"), "6");
    r.assert_originals_intact_except_new(&field(&l, "id"));
    assert!(std::fs::read_dir(r.dirs.import_tmp()).unwrap().next().is_none());
    r.assert_no_children();
}

#[test]
#[ignore]
fn qa_sigkill_while_working_leaves_the_library_intact_and_the_next_run_cleans_up() {
    if !ready() {
        return;
    }
    let mut r = Rig::new("qa_kill_working", |d| real_server(d, "hang"), &["import-audio {media}/untagged.flac".into()]);
    r.open_source(1);
    r.browse();
    r.wait_ui("phase=ready");
    r.extract();
    r.wait_ui("phase=working");
    until(10, "the stub separator", || r.app().leftovers().iter().any(|(_, c)| c.contains("stub-separator") && c.contains("input.flac")));
    r.kill_app();
    r.assert_no_client_children();
    // no partial track, no visible staging in tracks/
    assert_eq!(r.dirs.track_ids().len(), 6, "{:?}", r.dirs.track_ids());
    for (k, v) in &r.repo_before {
        assert_eq!(snapshot(&r.dirs.repo()).get(k), Some(v), "{k}");
    }
    let leftovers: Vec<String> = std::fs::read_dir(r.dirs.import_tmp()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert!(leftovers.iter().all(|n| n.starts_with("file-")), "{leftovers:?}");
    // restart (server restarted too: its old job folder is cleaned, its state forgotten)
    let cmd = real_server(&r.dirs, "ok");
    r.restart(&cmd, &["import-audio {media}/untagged.flac"]);
    let lib = r.app().all_lines().into_iter().rev().find(|l| l.contains("library root=")).unwrap();
    assert_eq!((field(&lib, "tracks").as_str(), field(&lib, "problems").as_str()), ("6", "0"), "{lib}");
    // a new import works and removes the stale file-* folder
    r.open_source(1);
    r.browse();
    r.wait("dialog kind=import-audio");
    r.wait_ui("phase=ready");
    assert!(std::fs::read_dir(r.dirs.import_tmp()).unwrap().flatten().count() == 1, "the stale folder was not cleaned");
    r.extract();
    let l = r.wait_ui("saved id=");
    r.assert_originals_intact_except_new(&field(&l, "id"));
    r.assert_no_children();
}

impl Rig {
    fn wait(&mut self, what: &str) -> String {
        self.app_mut().wait_line(|l| l.contains(what), LONG)
    }
}

#[test]
#[ignore]
fn qa_malicious_server_stem_names_are_refused_in_the_ui() {
    if !ready() {
        return;
    }
    for mode in ["traversal", "duplicate", "notflac"] {
        let mut r = Rig::new(&format!("qa_hostile_{mode}"), |_| hostile_server(mode), &["import-audio {media}/untagged.flac".into()]);
        r.open_source(1);
        r.browse();
        r.wait("dialog kind=import-audio");
        r.wait_ui("phase=ready");
        r.extract();
        let l = r.wait_ui("error stage=");
        settle();
        shot(&format!("qa-hostile-{mode}"));
        eprintln!("[qa] {mode}: {l}");
        assert!(l.contains("stage=server") || l.contains("stage=save"), "{l}");
        // nothing was saved, nothing escaped the repository
        assert_eq!(r.dirs.track_ids().len(), 6, "{mode}: {:?}", r.dirs.track_ids());
        assert!(!r.dirs.repo().join("evil.flac").exists());
        assert!(!r.dirs.base().join("data/evil.flac").exists());
        assert!(!r.dirs.base().join("evil.flac").exists());
        for e in std::fs::read_dir(r.dirs.repo().join("tracks")).unwrap().flatten() {
            assert!(!e.file_name().to_string_lossy().starts_with(".staging"), "staging left");
        }
        // the Extract button is usable again (the prepared audio is kept)
        let tmp: Vec<_> = std::fs::read_dir(r.dirs.import_tmp()).unwrap().flatten().collect();
        assert_eq!(tmp.len(), 1, "the prepared audio should stay for a retry");
        for (k, v) in &r.repo_before {
            assert_eq!(snapshot(&r.dirs.repo()).get(k), Some(v), "{k}");
        }
        r.assert_no_children();
    }
}
