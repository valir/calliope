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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
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
        let mut child = cmd.stderr(Stdio::piped()).spawn().expect("spawn calliope-gui");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let l2 = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                eprintln!("[app] {line}");
                l2.lock().unwrap().push(line);
            }
        });
        App { child, tag, lines, seen: 0 }
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
            assert!(Instant::now() < end, "timeout waiting for a stderr line; got: {:?}", self.all_lines());
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
        let o = out("xdotool", &["search", "--sync", "--onlyvisible", "--name", "^calliope$"]);
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
        let _ = Command::new("i3-msg").arg(format!("[id={wid}] kill")).output();
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
        let bad: Vec<_> = self.all_lines().into_iter().filter(|l| l.contains("csp-violation") || l.contains("not allowed")).collect();
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
    let o = Command::new("i3-msg").arg(format!("[id={wid}] floating enable")).output().unwrap();
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
            let mut it = s.split([',', 'x']).map(|n| n.trim().parse::<i32>().unwrap_or(0));
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
    line.split_whitespace().find_map(|w| w.strip_prefix(&pre)).unwrap_or("").to_string()
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
    assert!(dirs.0.starts_with(repo().join("target")), "temp dir is not under target/");
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
            .map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
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
            .args(["windowactivate", "--sync", wid, "type", "--delay", "40", text])
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
        self.wait_line(|l| l.trim_end().ends_with(&format!("select id={id}")), Duration::from_secs(10));
        settle();
    }

    pub fn wait_contains(&mut self, what: &str) -> String {
        self.wait_line(|l| l.contains(what), Duration::from_secs(15))
    }
}

pub fn size(wid: &str, w: u32, h: u32) {
    let _ = Command::new("xdotool").args(["windowsize", wid, &w.to_string(), &h.to_string()]).status();
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
    let Ok(rd) = std::fs::read_dir("/proc") else { return v };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == me {
            continue;
        }
        let Ok(env) = std::fs::read(e.path().join("environ")) else { continue };
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
            .map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// Every file below `dir` (relative path -> bytes), for byte-identity checks.
pub fn snapshot(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(p.strip_prefix(base).unwrap().to_string_lossy().to_string(), std::fs::read(&p).unwrap());
            }
        }
    }
    let mut m = std::collections::BTreeMap::new();
    walk(dir, dir, &mut m);
    m
}

/// A temp area for one import test: the library fixture is the starting repository, the
/// fixture sources are copied to `<base>/media/`.
pub fn import_dirs(test: &str) -> Dirs {
    let dirs = lib_dirs(test);
    let media = dirs.base().join("media");
    std::fs::create_dir_all(&media).unwrap();
    for f in ["tagged.mp3", "tagged.ogg", "untagged.flac", "with-audio.mp4", "no-audio.mp4", "long.flac"] {
        std::fs::copy(root().join("tests/fixtures/import").join(f), media.join(f)).unwrap();
    }
    std::fs::create_dir_all(dirs.base().join("stems-work")).unwrap();
    std::fs::create_dir_all(dirs.base().join("home")).unwrap();
    dirs
}

impl Dirs {
    pub fn media(&self, name: &str) -> PathBuf {
        self.0.join("media").join(name)
    }
}

/// Starts the e2e app and the real `calliope-stems` binary (with `tests/support/stub-separator`)
/// inside `unshare -rn` with only the loopback interface up, so neither can reach the internet
/// or the LAN. Temp XDG dirs and HOME, the fake yt-dlp first on PATH, a pre-written
/// `settings.json` (repository root, server on 127.0.0.1:8765, `keep_original`). Waits for the
/// Library to load, floats the window at 1280x800.
pub fn start_import_app(dirs: &Dirs, stub_mode: &str, keep_original: bool, answers: &[String]) -> (App, String) {
    assert!(dirs.base().starts_with(repo().join("target")), "temp dir is not under target/");
    let stems = repo().join("target/debug/calliope-stems");
    assert!(stems.exists(), "build calliope-stems first (cargo build -p calliope-stems)");
    std::fs::write(dirs.base().join("stems-work/stub-mode"), format!("{stub_mode}\n")).unwrap();
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
    // A leftover from an earlier crashed run would hold the port.
    kill_tagged(&tag);
    let script = format!(
        "ip link set lo up || exit 1; {stems} --listen 127.0.0.1:{STEMS_PORT} --work-dir {work} \
         --separator {sep} 2> {log} & exec {bin}",
        stems = sh_quote(&stems),
        work = sh_quote(&dirs.base().join("stems-work")),
        sep = sh_quote(&root().join("tests/support/stub-separator")),
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
        .env("PATH", format!("{}:{}", root().join("tests/support/bin").display(), system_path()))
        .env("FAKE_YTDLP_LOG", dirs.base().join("ytdlp.log"))
        .env("CALLIOPE_E2E_DIALOG_ANSWERS", &ans)
        .env("CALLIOPE_E2E_TAG", &tag);
    let mut app = App::spawn(cmd, Some(tag));
    app.wait_ready();
    let end = Instant::now() + Duration::from_secs(15);
    while !dirs.stems_log().contains("listening") {
        assert!(Instant::now() < end, "calliope-stems did not start: {}", dirs.stems_log());
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
        let Ok(ns) = std::fs::read_link(format!("/proc/{pid}/ns/net")) else { continue };
        assert_ne!(ns, me, "{cmd} shares the test's network namespace");
        let dev = std::fs::read_to_string(format!("/proc/{pid}/net/dev")).unwrap();
        let ifaces: Vec<&str> = dev.lines().skip(2).filter_map(|l| l.split(':').next()).map(str::trim).collect();
        assert_eq!(ifaces, ["lo"], "{cmd}: interfaces {ifaces:?}");
        checked += 1;
    }
    assert!(checked >= 2, "expected the app and the server, checked {checked}");
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
