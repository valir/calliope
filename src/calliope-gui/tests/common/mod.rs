//! Shared harness for the GUI e2e tests (copied from gui_e2e.rs, plus library helpers).
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_calliope-gui");

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The repository root (workspace): `target/`, `docs/`, `specs/` and the workspace Cargo.toml.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub fn have_display() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        println!("skipping: no DISPLAY");
        return false;
    }
    true
}

pub fn have(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool} >/dev/null")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn out(cmd: &str, args: &[&str]) -> String {
    let o = Command::new(cmd).args(args).output().expect(cmd);
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Fresh XDG dirs under target/gui-e2e/<test>/.
pub struct Dirs(PathBuf);

impl Dirs {
    pub fn new(test: &str) -> Dirs {
        let base = repo().join("target/gui-e2e").join(test);
        let _ = std::fs::remove_dir_all(&base);
        for d in ["config", "data", "cache"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        Dirs(base)
    }
    pub fn app_config(&self) -> PathBuf {
        self.0.join("config/app.calliope.gui")
    }
}

/// A running app; killed on drop.
pub struct App {
    child: Child,
    /// `CALLIOPE_E2E_TAG` value: every process carrying it in its environment is killed on drop.
    tag: Option<String>,
    lines: Arc<Mutex<Vec<String>>>,
    seen: usize,
}

impl App {
    pub fn start(dirs: &Dirs, offline: bool) -> App {
        App::start_with(dirs, offline, &[])
    }

    pub fn start_with(dirs: &Dirs, offline: bool, env: &[(&str, &Path)]) -> App {
        if !cfg!(feature = "e2e-hooks") && env.iter().any(|(k, _)| *k == "CALLIOPE_E2E_AUDIO") {
            panic!("the audio fake needs --features e2e-hooks; without it the app would open the real sound device");
        }
        let mut cmd = if offline {
            let mut c = Command::new("unshare");
            c.args(["-rn", BIN]);
            c
        } else {
            Command::new(BIN)
        };
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("XDG_CONFIG_HOME", dirs.0.join("config"))
            .env("XDG_DATA_HOME", dirs.0.join("data"))
            .env("XDG_CACHE_HOME", dirs.0.join("cache"));
        App::spawn(cmd, None)
    }

    pub fn spawn(mut cmd: Command, tag: Option<String>) -> App {
        let mut child = cmd
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn calliope-gui");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let l2 = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                eprintln!("[app] {line}");
                l2.lock().unwrap().push(line);
            }
        });
        App {
            child,
            tag,
            lines,
            seen: 0,
        }
    }

    pub fn all_lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    /// Waits for a new line (after those already consumed) matching `pred`.
    pub fn wait_line(&mut self, pred: impl Fn(&str) -> bool, timeout: Duration) -> String {
        let end = Instant::now() + timeout;
        loop {
            {
                let l = self.lines.lock().unwrap();
                while self.seen < l.len() {
                    let line = l[self.seen].clone();
                    self.seen += 1;
                    if pred(&line) {
                        return line;
                    }
                }
            }
            assert!(
                Instant::now() < end,
                "timeout waiting for a stderr line; got: {:?}",
                self.all_lines()
            );
            sleep(Duration::from_millis(50));
        }
    }

    pub fn wait_ready(&mut self) -> String {
        self.wait_line(|l| l.contains("ready "), Duration::from_secs(30))
    }

    pub fn wait_view(&mut self, id: &str) {
        let want = format!("view={id}");
        self.wait_line(|l| l.trim_end().ends_with(&want), Duration::from_secs(10));
    }

    pub fn wid(&self) -> String {
        let o = out(
            "xdotool",
            &["search", "--sync", "--onlyvisible", "--name", "^calliope$"],
        );
        o.lines().next().expect("window id").to_string()
    }

    pub fn key(&self, wid: &str, keys: &str) {
        let s = Command::new("xdotool")
            .args(["windowactivate", "--sync", wid, "key", "--clearmodifiers"])
            .args(keys.split_whitespace())
            .status()
            .unwrap();
        assert!(s.success(), "xdotool key {keys}");
    }

    pub fn close_gracefully(&mut self, wid: &str) {
        let _ = Command::new("i3-msg")
            .arg(format!("[id={wid}] kill"))
            .output();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            sleep(Duration::from_millis(100));
        }
        panic!("app did not exit within 5 s after i3 kill");
    }

    pub fn no_csp_violation(&self) {
        let bad: Vec<_> = self
            .all_lines()
            .into_iter()
            .filter(|l| l.contains("csp-violation") || l.contains("not allowed"))
            .collect();
        assert!(bad.is_empty(), "CSP violations: {bad:?}");
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(tag) = &self.tag {
            kill_tagged(tag);
        }
    }
}

pub fn shot(name: &str) {
    shot_of(name, "^calliope$");
}

pub fn shot_of(name: &str, title: &str) {
    if !have("gui-shot") {
        println!("skipping screenshot {name}: gui-shot missing");
        return;
    }
    let dir = repo().join("target/gui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    let s = Command::new("gui-shot")
        .arg(&path)
        .args([title, "10"])
        .status()
        .unwrap();
    assert!(s.success(), "gui-shot {name}");
    assert!(path.exists());
}

pub fn float(wid: &str) {
    let o = Command::new("i3-msg")
        .arg(format!("[id={wid}] floating enable"))
        .output()
        .unwrap();
    assert!(o.status.success(), "i3-msg floating enable");
    sleep(Duration::from_millis(300));
}

pub fn geometry(wid: &str) -> ((i32, i32), (i32, i32)) {
    let g = out("xdotool", &["getwindowgeometry", wid]);
    let mut pos = (0, 0);
    let mut size = (0, 0);
    for line in g.lines() {
        let line = line.trim();
        let parse = |s: &str| {
            let mut it = s
                .split([',', 'x'])
                .map(|n| n.trim().parse::<i32>().unwrap_or(0));
            (it.next().unwrap_or(0), it.next().unwrap_or(0))
        };
        if let Some(r) = line.strip_prefix("Position:") {
            pos = parse(r.split_whitespace().next().unwrap_or(""));
        } else if let Some(r) = line.strip_prefix("Geometry:") {
            size = parse(r.trim());
        }
    }
    (pos, size)
}

pub fn field(line: &str, name: &str) -> String {
    let pre = format!("{name}=");
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&pre))
        .unwrap_or("")
        .to_string()
}

pub fn settle() {
    sleep(Duration::from_millis(500));
}

// ---- library helpers ----

pub fn copy_dir(from: &Path, to: &Path) {
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

pub const ID1: &str = "0199b0a0-0000-7000-8000-000000000001";
pub const ID2: &str = "0199b0a0-0000-7000-8000-000000000002";
pub const ID4: &str = "0199b0a0-0000-7000-8000-000000000004";
pub const ID5: &str = "0199b0a0-0000-7000-8000-000000000005";
pub const ID6: &str = "0199b0a0-0000-7000-8000-000000000006";

/// A temp dir with the library fixture copied to the default root under XDG_DATA_HOME.
pub fn lib_dirs(test: &str) -> Dirs {
    let dirs = Dirs::new(test);
    assert!(
        dirs.0.starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    copy_dir(&root().join("tests/fixtures/library-sample"), &dirs.repo());
    dirs
}

impl Dirs {
    pub fn base(&self) -> &Path {
        &self.0
    }
    /// The default repository root.
    pub fn repo(&self) -> PathBuf {
        self.0.join("data/calliope")
    }
    pub fn track_json(&self, id: &str) -> PathBuf {
        self.repo().join("tracks").join(id).join("track.json")
    }
    pub fn track_dir(&self, id: &str) -> PathBuf {
        self.repo().join("tracks").join(id)
    }
    pub fn json(&self, id: &str) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.track_json(id)).unwrap()).unwrap()
    }
    /// Entries of `<repo>/trash` (names), sorted.
    pub fn trash(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.repo().join("trash"))
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

impl App {
    /// Starts the app with the Library loaded, a floating 1280x800 window; returns it and its id.
    pub fn start_lib(dirs: &Dirs, env: &[(&str, &Path)]) -> (App, String, String) {
        let mut app = App::start_with(dirs, false, env);
        app.wait_ready();
        let lib = app.wait_line(|l| l.contains("library root="), Duration::from_secs(20));
        let wid = app.wid();
        float(&wid);
        size(&wid, 1280, 800);
        (app, wid, lib)
    }

    pub fn typ(&self, wid: &str, text: &str) {
        let s = Command::new("xdotool")
            .args([
                "windowactivate",
                "--sync",
                wid,
                "type",
                "--delay",
                "40",
                "--",
                text,
            ])
            .status()
            .unwrap();
        assert!(s.success(), "xdotool type");
    }

    pub fn tabs(&self, wid: &str, n: usize) {
        for _ in 0..n {
            self.key(wid, "Tab");
        }
    }

    /// Selects a track through the search box: Ctrl+F, type, three Downs (band, album, track), Enter.
    pub fn select_by_search(&mut self, wid: &str, query: &str, id: &str) {
        self.key(wid, "ctrl+f");
        self.typ(wid, query);
        settle();
        self.key(wid, "Down Down Down Return");
        self.wait_line(
            |l| l.trim_end().ends_with(&format!("select id={id}")),
            Duration::from_secs(10),
        );
        settle();
    }

    pub fn wait_contains(&mut self, what: &str) -> String {
        self.wait_line(|l| l.contains(what), Duration::from_secs(15))
    }
}

pub fn size(wid: &str, w: u32, h: u32) {
    let _ = Command::new("xdotool")
        .args(["windowsize", wid, &w.to_string(), &h.to_string()])
        .status();
    sleep(Duration::from_millis(600));
}

/// Number of `tracks=` in a `library ...` line.
pub fn tracks_in(line: &str) -> usize {
    field(line, "tracks").parse().unwrap()
}

// ---- import helpers ----

/// SIGKILLs every process whose environment has `CALLIOPE_E2E_TAG=<tag>` (the app, the
/// calliope-stems server, yt-dlp, ffmpeg and the stub separator of one test).
pub fn kill_tagged(tag: &str) {
    let want = format!("CALLIOPE_E2E_TAG={tag}");
    for _ in 0..3 {
        for (pid, _) in tagged_pids(&want) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        if tagged_pids(&want).is_empty() {
            return;
        }
        sleep(Duration::from_millis(100));
    }
}

/// (pid, command line) of the processes carrying `want` (`NAME=value`) in their environment.
pub fn tagged_pids(want: &str) -> Vec<(u32, String)> {
    let me = std::process::id();
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return v;
    };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Ok(env) = std::fs::read(e.path().join("environ")) else {
            continue;
        };
        if env.split(|b| *b == 0).any(|kv| kv == want.as_bytes()) {
            let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
            v.push((pid, String::from_utf8_lossy(&cmd).replace('\0', " ")));
        }
    }
    v
}

pub const STEMS_PORT: u16 = 8765;
pub const STEM_NAMES: [&str; 6] = ["bass", "drums", "guitar", "other", "piano", "vocals"];

/// The X authority file of the real session (HOME is replaced by a temp dir for the app).
fn xauthority() -> PathBuf {
    match std::env::var_os("XAUTHORITY") {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".Xauthority"),
    }
}

fn system_path() -> String {
    std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into())
}

impl Dirs {
    pub fn import_tmp(&self) -> PathBuf {
        self.repo().join("import-tmp")
    }
    pub fn stems_log(&self) -> String {
        std::fs::read_to_string(self.0.join("stems.log")).unwrap_or_default()
    }
    pub fn ytdlp_log(&self) -> String {
        std::fs::read_to_string(self.0.join("ytdlp.log")).unwrap_or_default()
    }
    /// Names of the entries of `<repo>/tracks`, sorted.
    pub fn track_ids(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.repo().join("tracks"))
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// Every file below `dir` (relative path -> bytes), for byte-identity checks.
pub fn snapshot(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_string_lossy().to_string(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut m = std::collections::BTreeMap::new();
    walk(dir, dir, &mut m);
    m
}

/// Like [`snapshot`], but each `track.json` is compared without its `exports` member (an export
/// records what was written there), re-serialized.
pub fn snapshot_except_exports(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut m = snapshot(dir);
    for (k, v) in m.iter_mut() {
        if k.ends_with("track.json") {
            let mut j: serde_json::Value = serde_json::from_slice(v).unwrap();
            if let Some(o) = j.as_object_mut() {
                o.remove("exports");
            }
            *v = serde_json::to_vec_pretty(&j).unwrap();
        }
    }
    m
}

/// A temp area for one import test: the library fixture is the starting repository, the
/// fixture sources are copied to `<base>/media/`.
pub fn import_dirs(test: &str) -> Dirs {
    let dirs = lib_dirs(test);
    let media = dirs.base().join("media");
    std::fs::create_dir_all(&media).unwrap();
    for f in [
        "tagged.mp3",
        "tagged.ogg",
        "untagged.flac",
        "with-audio.mp4",
        "no-audio.mp4",
        "long.flac",
    ] {
        std::fs::copy(root().join("tests/fixtures/import").join(f), media.join(f)).unwrap();
    }
    // Folders for the directory imports (copied before the tests snapshot `media`).
    let fx = root().join("tests/fixtures/import");
    let put = |dir: &str, name: &str, from: &str| {
        let d = media.join(dir);
        std::fs::create_dir_all(d.join(Path::new(name).parent().unwrap())).unwrap();
        std::fs::copy(fx.join(from), d.join(name)).unwrap();
    };
    put("album", "a.flac", "untagged.flac");
    put("album", "b.mp3", "tagged.mp3");
    put("album", "sub/c.mp3", "tagged.mp3");
    std::fs::write(media.join("album/cover.jpg"), b"not an image").unwrap();
    put("three", "1.flac", "untagged.flac");
    put("three", "2.ogg", "tagged.ogg");
    put("three", "3.mp3", "tagged.mp3");
    std::fs::create_dir_all(media.join("no-songs")).unwrap();
    std::fs::write(media.join("no-songs/cover.jpg"), b"not an image").unwrap();
    std::fs::write(media.join("no-songs/notes.txt"), b"notes").unwrap();
    std::fs::create_dir_all(dirs.base().join("stems-work")).unwrap();
    std::fs::create_dir_all(dirs.base().join("home")).unwrap();
    dirs
}

impl Dirs {
    pub fn media(&self, name: &str) -> PathBuf {
        self.0.join("media").join(name)
    }
}

/// Starts the e2e app and the real `calliope-stems` binary (with `tests/support/stub-separator`
/// and `tests/support/stub-beat-tracker`, tracker mode `ok`) inside `unshare -rn` with only the
/// loopback interface up, so neither can reach the internet or the LAN. Temp XDG dirs and HOME,
/// the fake yt-dlp first on PATH, a pre-written `settings.json` (repository root, server on
/// 127.0.0.1:8765, `keep_original`). Waits for the Library to load, floats the window at 1280x800.
pub fn start_import_app(
    dirs: &Dirs,
    stub_mode: &str,
    keep_original: bool,
    answers: &[String],
) -> (App, String) {
    start_stems_app(dirs, stub_mode, "ok", keep_original, answers, None)
}

/// Sets the stub beat tracker's mode for the runs that start from now on.
pub fn set_tracker_mode(dirs: &Dirs, mode: &str) {
    std::fs::write(
        dirs.base().join("stems-work/stub-tracker-mode"),
        format!("{mode}\n"),
    )
    .unwrap();
}

/// The Editor on the `library-editor` copy (`editor_dirs`) with the capturing fake audio backend
/// and a calliope-stems server (stub beat tracker in `tracker_mode`, timeout 3 s) in the same
/// private network namespace as the app; opens the Editor (Alt+3).
pub fn start_detect_app(dirs: &Dirs, tracker_mode: &str) -> (App, String) {
    if !cfg!(feature = "e2e-hooks") {
        panic!("Editor GUI tests need --features e2e-hooks; without it the app would open the real sound device");
    }
    std::fs::create_dir_all(dirs.base().join("stems-work")).unwrap();
    std::fs::create_dir_all(dirs.base().join("home")).unwrap();
    let (mut app, wid) =
        start_stems_app(dirs, "ok", tracker_mode, false, &[], Some(dirs.capture()));
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    (app, wid)
}

fn start_stems_app(
    dirs: &Dirs,
    stub_mode: &str,
    tracker_mode: &str,
    keep_original: bool,
    answers: &[String],
    capture: Option<PathBuf>,
) -> (App, String) {
    assert!(
        dirs.base().starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    let stems = repo().join("target/debug/calliope-stems");
    assert!(
        stems.exists(),
        "build calliope-stems first (cargo build -p calliope-stems)"
    );
    std::fs::create_dir_all(dirs.base().join("stems-work")).unwrap();
    std::fs::write(
        dirs.base().join("stems-work/stub-mode"),
        format!("{stub_mode}\n"),
    )
    .unwrap();
    set_tracker_mode(dirs, tracker_mode);
    let cfg = dirs.app_config();
    std::fs::create_dir_all(&cfg).unwrap();
    // Keeps what the test wrote before (e.g. the theme).
    let file = cfg.join("settings.json");
    let mut settings: serde_json::Value = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    settings["repository_root"] = serde_json::json!(dirs.repo());
    settings["edge_ai_url"] = serde_json::json!(format!("http://127.0.0.1:{STEMS_PORT}"));
    settings["keep_original"] = serde_json::json!(keep_original);
    std::fs::write(file, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
    let ans = dirs.base().join("answers");
    std::fs::write(
        &ans,
        answers.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();

    let tag = dirs
        .base()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // A leftover from an earlier crashed run would hold the port.
    kill_tagged(&tag);
    let script = format!(
        "ip link set lo up || exit 1; {stems} --listen 127.0.0.1:{STEMS_PORT} --work-dir {work} \
         --separator {sep} --beat-tracker {beats} --beat-timeout-ms 3000 2> {log} & exec {bin}",
        stems = sh_quote(&stems),
        work = sh_quote(&dirs.base().join("stems-work")),
        sep = sh_quote(&root().join("tests/support/stub-separator")),
        beats = sh_quote(&root().join("tests/support/stub-beat-tracker")),
        log = sh_quote(&dirs.base().join("stems.log")),
        bin = sh_quote(Path::new(BIN)),
    );
    let mut cmd = Command::new("unshare");
    cmd.args(["-rn", "sh", "-c", &script])
        .env("XDG_CONFIG_HOME", dirs.base().join("config"))
        .env("XDG_DATA_HOME", dirs.base().join("data"))
        .env("XDG_CACHE_HOME", dirs.base().join("cache"))
        .env("XDG_STATE_HOME", dirs.base().join("state"))
        .env("XAUTHORITY", xauthority())
        .env("HOME", dirs.base().join("home"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                root().join("tests/support/bin").display(),
                system_path()
            ),
        )
        .env("FAKE_YTDLP_LOG", dirs.base().join("ytdlp.log"))
        .env("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)
        .env("CALLIOPE_E2E_TAG", &tag);
    if let Some(cap) = capture {
        cmd.env("CALLIOPE_E2E_AUDIO", format!("capture:{}", cap.display()));
    }
    let mut app = App::spawn(cmd, Some(tag));
    app.wait_ready();
    let end = Instant::now() + Duration::from_secs(15);
    while !dirs.stems_log().contains("listening") {
        assert!(
            Instant::now() < end,
            "calliope-stems did not start: {}",
            dirs.stems_log()
        );
        sleep(Duration::from_millis(50));
    }
    app.wait_line(|l| l.contains("library root="), Duration::from_secs(20));
    let wid = app.wid();
    assert_loopback_only(&app);
    float(&wid);
    size(&wid, 1280, 800);
    (app, wid)
}

/// True when one of the words of `cmd` is the program `name` (by file name, so a path that
/// merely contains the name, like `src/calliope-gui/tests/...`, doesn't count).
pub fn runs_program(cmd: &str, name: &str) -> bool {
    cmd.split_whitespace()
        .any(|w| w.trim_matches(|c| c == '\'' || c == '"').rsplit('/').next() == Some(name))
}

fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

/// The app process and the server see only `lo` (their own network namespace).
pub fn assert_loopback_only(app: &App) {
    let me = std::fs::read_link("/proc/self/ns/net").unwrap();
    let tag = app.tag.clone().unwrap();
    let procs = tagged_pids(&format!("CALLIOPE_E2E_TAG={tag}"));
    assert!(!procs.is_empty());
    let mut checked = 0;
    for (pid, cmd) in procs {
        if !(runs_program(&cmd, "calliope-gui") || runs_program(&cmd, "calliope-stems")) {
            continue;
        }
        let Ok(ns) = std::fs::read_link(format!("/proc/{pid}/ns/net")) else {
            continue;
        };
        assert_ne!(ns, me, "{cmd} shares the test's network namespace");
        let dev = std::fs::read_to_string(format!("/proc/{pid}/net/dev")).unwrap();
        let ifaces: Vec<&str> = dev
            .lines()
            .skip(2)
            .filter_map(|l| l.split(':').next())
            .map(str::trim)
            .collect();
        assert_eq!(ifaces, ["lo"], "{cmd}: interfaces {ifaces:?}");
        checked += 1;
    }
    assert!(
        checked >= 2,
        "expected the app and the server, checked {checked}"
    );
}

impl App {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Waits until the app process has exited.
    pub fn wait_exit(&mut self, timeout: Duration) {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            sleep(Duration::from_millis(100));
        }
        panic!("app still running after {timeout:?}");
    }

    /// Processes of this test still alive (empty when everything is gone).
    pub fn leftovers(&self) -> Vec<(u32, String)> {
        tagged_pids(&format!("CALLIOPE_E2E_TAG={}", self.tag.clone().unwrap()))
    }
}

// ---- editor helpers ----

pub const ED_FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
pub const ED_MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
pub const ED_PLAIN: &str = "0199c0a0-0000-7000-8000-000000000203";
pub const ED_SIX: &str = "0199c0a0-0000-7000-8000-000000000204";
pub const ED_MISSING: &str = "0199c0a0-0000-7000-8000-000000000205";

impl Dirs {
    /// The WAV the capture audio backend writes.
    pub fn capture(&self) -> PathBuf {
        self.0.join("capture.wav")
    }
}

/// A temp dir with the `library-editor` fixture as the default repository.
pub fn editor_dirs(test: &str) -> Dirs {
    let dirs = Dirs::new(test);
    assert!(
        dirs.0.starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    copy_dir(&root().join("tests/fixtures/library-editor"), &dirs.repo());
    dirs
}

/// Starts the e2e app on the editor fixture with the capturing fake audio backend (never a real
/// device), floats the window at 1280x800 and opens the Editor (Alt+3).
pub fn start_editor(dirs: &Dirs) -> (App, String) {
    if !cfg!(feature = "e2e-hooks") {
        panic!("Editor GUI tests need --features e2e-hooks; without it the app would open the real sound device");
    }
    let cap = dirs.capture();
    let audio = format!("capture:{}", cap.display());
    // Offline too: nothing in the editor needs a network.
    let mut app = App::start_with(dirs, true, &[("CALLIOPE_E2E_AUDIO", Path::new(&audio))]);
    app.wait_ready();
    app.wait_line(|l| l.contains("library root="), Duration::from_secs(20));
    let wid = app.wid();
    float(&wid);
    size(&wid, 1280, 800);
    app.key(&wid, "alt+3");
    app.wait_view("editor");
    (app, wid)
}

impl App {
    /// Left-clicks at (x, y) relative to the window.
    pub fn click(&self, wid: &str, x: i32, y: i32) {
        let s = Command::new("xdotool")
            .args([
                "windowactivate",
                "--sync",
                wid,
                "mousemove",
                "--window",
                wid,
                &x.to_string(),
                &y.to_string(),
                "click",
                "1",
            ])
            .status()
            .unwrap();
        assert!(s.success(), "xdotool click");
    }

    /// Drags with the left button from (x0, y) to (x1, y), in steps, relative to the window.
    pub fn drag(&self, wid: &str, x0: i32, x1: i32, y: i32, before_release: impl FnOnce()) {
        let run = |args: &[&str]| {
            let s = Command::new("xdotool").args(args).status().unwrap();
            assert!(s.success(), "xdotool {args:?}");
        };
        run(&[
            "windowactivate",
            "--sync",
            wid,
            "mousemove",
            "--window",
            wid,
            &x0.to_string(),
            &y.to_string(),
            "mousedown",
            "1",
        ]);
        let n = 8;
        for i in 1..=n {
            let x = x0 + (x1 - x0) * i / n;
            run(&["mousemove", "--window", wid, &x.to_string(), &y.to_string()]);
            sleep(Duration::from_millis(40));
        }
        before_release();
        run(&["mouseup", "1"]);
    }

    /// Mouse wheel (`up` = button 4) `n` times at (x, y) relative to the window.
    pub fn wheel(&self, wid: &str, x: i32, y: i32, up: bool, n: u32) {
        let b = if up { "4" } else { "5" };
        let s = Command::new("xdotool")
            .args([
                "windowactivate",
                "--sync",
                wid,
                "mousemove",
                "--window",
                wid,
                &x.to_string(),
                &y.to_string(),
            ])
            .status()
            .unwrap();
        assert!(s.success(), "xdotool mousemove");
        for _ in 0..n {
            let s = Command::new("xdotool").args(["click", b]).status().unwrap();
            assert!(s.success(), "xdotool wheel");
            sleep(Duration::from_millis(60));
        }
    }

    /// Lines logged so far that contain `what`.
    pub fn count(&self, what: &str) -> usize {
        self.all_lines().iter().filter(|l| l.contains(what)).count()
    }

    /// The test never opened the real device.
    pub fn assert_no_real_audio(&self) {
        let lines = self.all_lines();
        assert!(
            !lines.iter().any(|l| l.contains("audio backend=cpal")),
            "the real audio device was opened"
        );
        assert!(!lines.iter().any(|l| l.contains("audio backend=")
            && !(l.contains("backend=capture") || l.contains("backend=null"))));
    }
}

// ---- audio analysis (Editor and Player tests) ----

/// Interleaved stereo f32 samples of the capture WAV from byte `from` of the file (the header
/// is 44 bytes; its sizes are only patched when the stream closes, so they are ignored).
pub fn capture_from(path: &Path, from: usize) -> Vec<f32> {
    let b = std::fs::read(path).unwrap();
    let from = from.max(44) & !7;
    b[from.min(b.len())..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn capture_len(path: &Path) -> usize {
    std::fs::metadata(path)
        .map(|m| m.len() as usize)
        .unwrap_or(0)
}

/// Mean power of `freq` in the left channel (Goertzel).
pub fn tone_power(stereo: &[f32], rate: f64, freq: f64) -> f64 {
    let left: Vec<f64> = stereo.chunks_exact(2).map(|c| f64::from(c[0])).collect();
    let n = left.len();
    assert!(n > 1000, "too little audio: {n} frames");
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for x in &left {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let p = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    p / (n as f64 * n as f64)
}

pub fn db(a: f64, b: f64) -> f64 {
    10.0 * ((a + 1e-30) / (b + 1e-30)).log10()
}

/// Decodes any audio file to interleaved stereo f32 with ffmpeg.
pub fn decode(path: &Path) -> Vec<f32> {
    let o = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-ac", "2", "-"])
        .output()
        .unwrap();
    assert!(o.status.success(), "ffmpeg decode of {}", path.display());
    o.stdout
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn rate_of(app: &App) -> f64 {
    let l = app
        .all_lines()
        .into_iter()
        .rev()
        .find(|l| l.contains("audio backend="))
        .expect("audio backend line");
    field(&l, "rate").parse().unwrap()
}

/// Onsets (ms from the start of `stereo`) of clicks in the 1.5 and 2 kHz band: windows of 4 ms
/// every 1 ms; an onset is the first window whose band power passes 25% of the maximum after
/// at least 100 ms below it.
pub fn click_onsets_ms(stereo: &[f32], rate: f64) -> Vec<f64> {
    let left: Vec<f64> = stereo.chunks_exact(2).map(|c| f64::from(c[0])).collect();
    let win = (rate * 0.004) as usize;
    let hop = (rate * 0.001) as usize;
    let power = |start: usize, hz: f64| {
        let w = 2.0 * std::f64::consts::PI * hz / rate;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for x in &left[start..start + win] {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - coeff * s1 * s2
    };
    let mut p = Vec::new();
    let mut i = 0;
    while i + win <= left.len() {
        p.push(power(i, 1500.0) + power(i, 2000.0));
        i += hop;
    }
    let max = p.iter().cloned().fold(0.0, f64::max);
    let mut out = Vec::new();
    let mut quiet = usize::MAX / 2;
    for (k, v) in p.iter().enumerate() {
        if *v > max * 0.25 {
            if quiet >= 100 {
                out.push(k as f64);
            }
            quiet = 0;
        } else {
            quiet = quiet.saturating_add(1);
        }
    }
    out
}

/// Time (ms from the start of `stereo`) at which a tone of `freq` starts: the first 10 ms
/// window holding a quarter of the maximum power is half filled, so its middle is the onset.
pub fn tone_start_ms(stereo: &[f32], rate: f64, freq: f64) -> Option<f64> {
    let left: Vec<f64> = stereo.chunks_exact(2).map(|c| f64::from(c[0])).collect();
    let win = (rate * 0.01) as usize;
    let hop = (rate * 0.001) as usize;
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let coeff = 2.0 * w.cos();
    let mut p = Vec::new();
    let mut i = 0;
    while i + win <= left.len() {
        let (mut s1, mut s2) = (0.0, 0.0);
        for x in &left[i..i + win] {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        p.push(s1 * s1 + s2 * s2 - coeff * s1 * s2);
        i += hop;
    }
    let max = p.iter().cloned().fold(0.0, f64::max);
    p.iter()
        .position(|v| *v > max * 0.25)
        .map(|k| (k * hop + win / 2) as f64 * 1000.0 / rate)
}

impl App {
    /// Marks every line logged so far as consumed, so `wait_line` only sees new ones.
    pub fn skip_seen(&mut self) {
        self.seen = self.lines.lock().unwrap().len();
    }
}

// ---- player helpers ----

pub const PL_PRACTICE: &str = "0199c0a0-0000-7000-8000-000000000301";
pub const PL_STEMS: &str = "0199c0a0-0000-7000-8000-000000000302";
pub const PL_MP3: &str = "0199c0a0-0000-7000-8000-000000000303";
pub const PL_TEMPO: &str = "0199c0a0-0000-7000-8000-000000000304";
pub const PL_MISSING_BACKING: &str = "0199c0a0-0000-7000-8000-000000000305";
pub const PL_MISSING_AUDIO: &str = "0199c0a0-0000-7000-8000-000000000306";

impl Dirs {
    pub fn player_json(&self, id: &str) -> PathBuf {
        self.track_dir(id).join("player.json")
    }
}

/// A temp dir with the `library-player` fixture as the default repository.
pub fn player_dirs(test: &str) -> Dirs {
    let dirs = Dirs::new(test);
    assert!(
        dirs.0.starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    copy_dir(&root().join("tests/fixtures/library-player"), &dirs.repo());
    dirs
}

/// Starts the e2e app on the Player fixture with the capturing fake audio backend (never a real
/// device), floats the window at 1280x800 and opens the Player (Alt+5). Works on the same dirs
/// again after a restart (the capture file is rewritten).
pub fn start_player(dirs: &Dirs) -> (App, String) {
    if !cfg!(feature = "e2e-hooks") {
        panic!("Player GUI tests need --features e2e-hooks; without it the app would open the real sound device");
    }
    let audio = format!("capture:{}", dirs.capture().display());
    let mut app = App::start_with(dirs, true, &[("CALLIOPE_E2E_AUDIO", Path::new(&audio))]);
    app.wait_ready();
    app.wait_line(|l| l.contains("library root="), Duration::from_secs(20));
    let wid = app.wid();
    float(&wid);
    size(&wid, 1280, 800);
    app.key(&wid, "alt+5");
    app.wait_view("player");
    (app, wid)
}

// ---- seeded repositories (gui-library-album-order) ----

/// One backing track of a seeded repository.
pub struct Seed<'a> {
    pub id: &'a str,
    pub band: &'a str,
    pub album: &'a str,
    pub title: &'a str,
    pub position: Option<u32>,
    pub album_position: Option<u32>,
}

/// Shorthand for a seed without positions.
pub fn seed<'a>(id: &'a str, band: &'a str, album: &'a str, title: &'a str) -> Seed<'a> {
    Seed {
        id,
        band,
        album,
        title,
        position: None,
        album_position: None,
    }
}

/// A temp dir (under target/) whose default root holds only these schema-2 backing tracks (each
/// with a copy of the sample's backing.mp3) and the repository marker.
pub fn seed_dirs(test: &str, tracks: &[Seed]) -> Dirs {
    let dirs = Dirs::new(test);
    assert!(
        dirs.0.starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    let repo_dir = dirs.repo();
    std::fs::create_dir_all(repo_dir.join("tracks")).unwrap();
    std::fs::write(
        repo_dir.join("calliope-repository.json"),
        "{\n  \"schema_version\": 1\n}\n",
    )
    .unwrap();
    let audio = root()
        .join("tests/fixtures/library-sample/tracks")
        .join(ID1)
        .join("backing.mp3");
    for t in tracks {
        let dir = dirs.track_dir(t.id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&audio, dir.join("backing.mp3")).unwrap();
        let mut v = serde_json::json!({
            "schema_version": 2,
            "type": "backing",
            "id": t.id,
            "band": t.band,
            "album": t.album,
            "title": t.title,
            "composers": [],
            "year": null,
            "source_url": null,
            "copyright": null,
            "audio": "backing.mp3",
            "original": null,
            "stems": [],
            "stem_model": null,
            "tablatures": [],
            "imported": "2026-10-01T12:00:00Z",
            "modified": "2026-10-01T12:00:00Z",
        });
        if let Some(p) = t.position {
            v["position"] = p.into();
        }
        if let Some(p) = t.album_position {
            v["album_position"] = p.into();
        }
        std::fs::write(
            dir.join("track.json"),
            serde_json::to_string_pretty(&v).unwrap() + "\n",
        )
        .unwrap();
    }
    dirs
}

impl Dirs {
    /// `(position, album_position)` of a track on disk.
    pub fn positions(&self, id: &str) -> (Option<u64>, Option<u64>) {
        let v = self.json(id);
        (v["position"].as_u64(), v["album_position"].as_u64())
    }
}

impl App {
    /// Left-button drag from `from` to `to` in 10 steps of 40 ms, relative to the window.
    /// `before_release` runs while the button is still down.
    pub fn drag_xy(
        &self,
        wid: &str,
        from: (i32, i32),
        to: (i32, i32),
        before_release: impl FnOnce(),
    ) {
        let run = |args: &[&str]| {
            let s = Command::new("xdotool").args(args).status().unwrap();
            assert!(s.success(), "xdotool {args:?}");
        };
        run(&[
            "windowactivate",
            "--sync",
            wid,
            "mousemove",
            "--window",
            wid,
            &from.0.to_string(),
            &from.1.to_string(),
            "mousedown",
            "1",
        ]);
        let n = 10;
        for i in 1..=n {
            let x = from.0 + (to.0 - from.0) * i / n;
            let y = from.1 + (to.1 - from.1) * i / n;
            run(&["mousemove", "--window", wid, &x.to_string(), &y.to_string()]);
            sleep(Duration::from_millis(40));
        }
        before_release();
        run(&["mouseup", "1"]);
    }
}

// ---- export helpers ----

pub const EX_STEM: &str = "0199c0a0-0000-7000-8000-000000000401";
pub const EX_NOMIX: &str = "0199c0a0-0000-7000-8000-000000000402";
pub const EX_PLAIN: &str = "0199c0a0-0000-7000-8000-000000000403";
pub const EX_MONO: &str = "0199c0a0-0000-7000-8000-000000000404";
pub const EX_LONG: &str = "0199c0a0-0000-7000-8000-000000000407";
pub const EX_NOTEMPO: &str = "0199c0a0-0000-7000-8000-000000000410";

/// A temp dir with the `library-export` fixture as the default repository.
pub fn export_dirs(test: &str) -> Dirs {
    let dirs = Dirs::new(test);
    assert!(
        dirs.0.starts_with(repo().join("target")),
        "temp dir is not under target/"
    );
    copy_dir(&root().join("tests/fixtures/library-export"), &dirs.repo());
    dirs
}
