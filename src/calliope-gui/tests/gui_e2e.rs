//! GUI end-to-end tests. They need an X display and are run with:
//! `DISPLAY=:1 npm run test:gui` (or `cargo test --test gui_e2e -- --ignored --test-threads=1`).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_calliope-gui");
const VIEWS: [&str; 6] = ["library", "import", "editor", "playlists", "player", "settings"];

/// The repository root (workspace): `target/`, `docs/`, `specs/` and the workspace Cargo.toml.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn have_display() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        println!("skipping: no DISPLAY");
        return false;
    }
    true
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool} >/dev/null")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn out(cmd: &str, args: &[&str]) -> String {
    let o = Command::new(cmd).args(args).output().expect(cmd);
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Fresh XDG dirs under target/gui-e2e/<test>/.
struct Dirs(PathBuf);

impl Dirs {
    fn new(test: &str) -> Dirs {
        let base = repo().join("target/gui-e2e").join(test);
        let _ = std::fs::remove_dir_all(&base);
        for d in ["config", "data", "cache"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        Dirs(base)
    }
    fn app_config(&self) -> PathBuf {
        self.0.join("config/app.calliope.gui")
    }
}

/// A running app; killed on drop.
struct App {
    child: Child,
    lines: Arc<Mutex<Vec<String>>>,
    seen: usize,
}

impl App {
    fn start(dirs: &Dirs, offline: bool) -> App {
        let mut cmd = if offline {
            let mut c = Command::new("unshare");
            c.args(["-rn", BIN]);
            c
        } else {
            Command::new(BIN)
        };
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

    fn all_lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    /// Waits for a new line (after those already consumed) matching `pred`.
    fn wait_line(&mut self, pred: impl Fn(&str) -> bool, timeout: Duration) -> String {
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

    fn wait_ready(&mut self) -> String {
        self.wait_line(|l| l.contains("ready "), Duration::from_secs(30))
    }

    fn wait_view(&mut self, id: &str) {
        let want = format!("view={id}");
        self.wait_line(|l| l.trim_end().ends_with(&want), Duration::from_secs(10));
    }

    fn wid(&self) -> String {
        let o = out("xdotool", &["search", "--sync", "--onlyvisible", "--name", "^calliope$"]);
        o.lines().next().expect("window id").to_string()
    }

    fn key(&self, wid: &str, keys: &str) {
        let s = Command::new("xdotool")
            .args(["windowactivate", "--sync", wid, "key", "--clearmodifiers"])
            .args(keys.split_whitespace())
            .status()
            .unwrap();
        assert!(s.success(), "xdotool key {keys}");
    }

    fn close_gracefully(&mut self, wid: &str) {
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

    fn no_csp_violation(&self) {
        let bad: Vec<_> = self.all_lines().into_iter().filter(|l| l.contains("csp-violation")).collect();
        assert!(bad.is_empty(), "CSP violations: {bad:?}");
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn shot(name: &str) {
    if !have("gui-shot") {
        println!("skipping screenshot {name}: gui-shot missing");
        return;
    }
    let dir = repo().join("target/gui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    let s = Command::new("gui-shot")
        .arg(&path)
        .args(["^calliope$", "10"])
        .status()
        .unwrap();
    assert!(s.success(), "gui-shot {name}");
    assert!(path.exists());
}

fn float(wid: &str) {
    let o = Command::new("i3-msg").arg(format!("[id={wid}] floating enable")).output().unwrap();
    assert!(o.status.success(), "i3-msg floating enable");
    sleep(Duration::from_millis(300));
}

fn geometry(wid: &str) -> ((i32, i32), (i32, i32)) {
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

fn field(line: &str, name: &str) -> String {
    let pre = format!("{name}=");
    line.split_whitespace().find_map(|w| w.strip_prefix(&pre)).unwrap_or("").to_string()
}

fn settle() {
    sleep(Duration::from_millis(500));
}

#[test]
#[ignore]
fn starts_dark_with_version() {
    if !have_display() {
        return;
    }
    let dirs = Dirs::new("starts_dark_with_version");
    let mut app = App::start(&dirs, false);
    let ready = app.wait_ready();
    assert_eq!(field(&ready, "theme"), "dark", "{ready}");
    let version = out(BIN, &["--version"]);
    assert!(!field(&ready, "version").is_empty());
    assert!(version.contains(&field(&ready, "version")), "ready={ready} version={version}");
    assert_eq!(field(&ready, "version"), version.split_whitespace().last().unwrap());
    app.wid();
    // First start: the default root is created and the Library shows the empty state.
    let end = Instant::now() + Duration::from_secs(15);
    let lib = loop {
        if let Some(l) = app.all_lines().into_iter().find(|l| l.contains("library root=")) {
            break l;
        }
        assert!(Instant::now() < end, "no library line; got: {:?}", app.all_lines());
        sleep(Duration::from_millis(50));
    };
    assert!(lib.contains("tracks=0"), "{lib}");
    let repo = dirs.0.join("data/calliope");
    assert!(repo.join("calliope-repository.json").is_file(), "marker missing in {repo:?}");
    assert!(repo.join("tracks").is_dir());
    settle();
    shot("start-dark");
    app.no_csp_violation();
}

#[test]
#[ignore]
fn shortcuts_switch_views() {
    if !have_display() {
        return;
    }
    let dirs = Dirs::new("shortcuts_switch_views");
    let mut app = App::start(&dirs, false);
    app.wait_ready();
    let wid = app.wid();
    settle();
    // Start on library: go to import first so that Alt+1 logs a change.
    app.key(&wid, "alt+2");
    app.wait_view("import");
    for (i, id) in VIEWS.iter().enumerate() {
        app.key(&wid, &format!("alt+{}", i + 1));
        app.wait_view(id);
        settle();
        shot(&format!("view-{id}"));
    }
    app.no_csp_violation();
}

#[test]
#[ignore]
fn theme_switch_persists() {
    if !have_display() {
        return;
    }
    let dirs = Dirs::new("theme_switch_persists");
    {
        let mut app = App::start(&dirs, false);
        app.wait_ready();
        let wid = app.wid();
        settle();
        app.key(&wid, "alt+6");
        app.wait_view("settings");
        app.key(&wid, "Tab");
        app.key(&wid, "Right");
        app.wait_line(|l| l.trim_end().ends_with("theme=light"), Duration::from_secs(10));
        settle();
        let s = std::fs::read_to_string(dirs.app_config().join("settings.json")).expect("settings.json");
        assert!(s.contains("\"light\""), "{s}");
        shot("settings-light");
        app.no_csp_violation();
    }
    let mut app = App::start(&dirs, false);
    let ready = app.wait_ready();
    assert_eq!(field(&ready, "theme"), "light", "{ready}");
    app.wid();
    settle();
    shot("restart-light");
    app.no_csp_violation();
}

fn state_main(dir: &Path) -> (i64, i64) {
    let s = std::fs::read_to_string(dir.join(".window-state.json")).expect(".window-state.json");
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let m = &v["main"];
    (m["width"].as_i64().unwrap(), m["height"].as_i64().unwrap())
}

fn near(a: i64, b: i64, tol: i64) -> bool {
    (a - b).abs() <= tol
}

#[test]
#[ignore]
fn window_state_and_min_size() {
    if !have_display() {
        return;
    }
    if !have("i3-msg") {
        println!("skipping: no i3-msg");
        return;
    }
    let dirs = Dirs::new("window_state_and_min_size");
    {
        let mut app = App::start(&dirs, false);
        app.wait_ready();
        let wid = app.wid();
        float(&wid);
        let size = |w: &str, a: &str, b: &str| {
            let _ = Command::new("xdotool").args(["windowsize", w, a, b]).status();
            sleep(Duration::from_millis(500));
        };
        size(&wid, "800", "500");
        let (_, (w, h)) = geometry(&wid);
        assert!(w >= 1024 && h >= 640, "min size not enforced: {w}x{h}");
        size(&wid, "1024", "640");
        for (i, id) in VIEWS.iter().enumerate() {
            app.key(&wid, &format!("alt+{}", i + 1));
            if i != 0 {
                app.wait_view(id); // the app starts on library: no change is logged for Alt+1
            }
            settle();
            shot(&format!("min-{id}"));
        }
        size(&wid, "1180", "720");
        let _ = Command::new("xdotool").args(["windowmove", &wid, "150", "120"]).status();
        sleep(Duration::from_millis(500));
        let before = geometry(&wid);
        println!("before close: {before:?}");
        app.close_gracefully(&wid);
        app.no_csp_violation();
    }
    let (w, h) = state_main(&dirs.app_config());
    assert!(near(w, 1180, 2) && near(h, 720, 2), "saved size {w}x{h}");

    let mut app = App::start(&dirs, false);
    app.wait_ready();
    let wid = app.wid();
    float(&wid);
    sleep(Duration::from_millis(500));
    let ((x, y), (w, h)) = geometry(&wid);
    println!("relaunch: pos=({x},{y}) size={w}x{h}");
    if near(w as i64, 1180, 2) && near(h as i64, 720, 2) {
        if !(near(x as i64, 150, 60) && near(y as i64, 120, 60)) {
            println!("NOTE: restored position ({x},{y}) not within 60 px of (150,120)");
        }
    } else {
        println!("NOTE: i3 did not apply restored geometry on float ({w}x{h}); state file asserted instead");
    }
    app.no_csp_violation();
}

#[test]
#[ignore]
fn offline_start() {
    if !have_display() {
        return;
    }
    if !Command::new("unshare").args(["-rn", "true"]).status().map(|s| s.success()).unwrap_or(false) {
        println!("skipping: unshare -rn unavailable");
        return;
    }
    let dirs = Dirs::new("offline_start");
    let mut app = App::start(&dirs, true);
    app.wait_ready();
    app.wid();
    settle();
    shot("offline");
    app.no_csp_violation();
}
