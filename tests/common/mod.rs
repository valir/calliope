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
        let base = root().join("target/gui-e2e").join(test);
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
        let mut child = cmd
            .env("XDG_CONFIG_HOME", dirs.0.join("config"))
            .env("XDG_DATA_HOME", dirs.0.join("data"))
            .env("XDG_CACHE_HOME", dirs.0.join("cache"))
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
        App { child, lines, seen: 0 }
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
    let dir = root().join("target/gui-shots");
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
    assert!(dirs.0.starts_with(root().join("target")), "temp dir is not under target/");
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
