//! The real `calliope-stems` binary on 127.0.0.1 (temp work dir, the stub separator and the stub
//! beat tracker, never a real model), shared by the job tests (import and beat detection).

use std::fs;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Some process has `pattern` in its command line.
pub(crate) fn pgrep(pattern: &str) -> bool {
    Command::new("pgrep")
        .args(["-f", "--", pattern])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

pub(crate) fn project() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn stems_binary() -> PathBuf {
    // unit tests live in target/<profile>/deps/
    let exe = std::env::current_exe().unwrap();
    let bin = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("calliope-stems");
    assert!(
        bin.is_file(),
        "{} is missing: run cargo build -p calliope-stems (cargo test --workspace builds it)",
        bin.display()
    );
    bin
}

/// The real server; killed when dropped. Its stderr lines are collected.
pub(crate) struct TestServer {
    child: Child,
    pub(crate) url: String,
    pub(crate) log: Arc<Mutex<Vec<String>>>,
    pub(crate) outer: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TestServer {
    pub(crate) fn start(mode: &str) -> TestServer {
        TestServer::start_with(mode, &[])
    }

    pub(crate) fn start_with(mode: &str, extra_args: &[&str]) -> TestServer {
        TestServer::start_full(mode, None, extra_args)
    }

    /// With the stub beat tracker in `tracker_mode` (and the stub separator in "ok"); the
    /// tracker times out after 1 s unless `extra_args` say otherwise.
    pub(crate) fn start_beats(tracker_mode: &str, extra_args: &[&str]) -> TestServer {
        TestServer::start_full("ok", Some(tracker_mode), extra_args)
    }

    fn start_full(mode: &str, tracker_mode: Option<&str>, extra_args: &[&str]) -> TestServer {
        let outer = tempfile::tempdir().unwrap();
        let work = outer.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("stub-mode"), format!("{mode}\n")).unwrap();
        let mut cmd = Command::new(stems_binary());
        if let Some(t) = tracker_mode {
            fs::write(work.join("stub-tracker-mode"), format!("{t}\n")).unwrap();
            cmd.arg("--beat-tracker")
                .arg(project().join("tests/support/stub-beat-tracker"));
            if !extra_args.contains(&"--beat-timeout-ms") {
                cmd.args(["--beat-timeout-ms", "1000"]);
            }
        }
        let mut child = cmd
            .arg("--separator")
            .arg(project().join("tests/support/stub-separator"))
            .args(["--listen", "127.0.0.1:0", "--work-dir"])
            .arg(&work)
            .args(extra_args)
            .env_remove("STUB_SEPARATOR_MODE")
            .env_remove("STUB_TRACKER_MODE")
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        let addr: SocketAddr = loop {
            line.clear();
            assert!(
                lines.read_line(&mut line).unwrap() > 0,
                "server exited before listening"
            );
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
        TestServer {
            child,
            url: format!("http://{addr}"),
            log,
            outer,
        }
    }

    pub(crate) fn set_mode(&self, mode: &str) {
        fs::write(
            self.outer.path().join("work/stub-mode"),
            format!("{mode}\n"),
        )
        .unwrap();
    }

    pub(crate) fn saw(&self, needle: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(needle))
    }

    pub(crate) fn wait_for(&self, needle: &str) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if self.saw(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    pub(crate) fn stem_gets(&self, name: &str) -> usize {
        let suffix = format!("/stems/{name} ");
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("method=GET path=/v1/jobs/") && l.contains(&suffix))
            .count()
    }

    pub(crate) fn all_stem_gets(&self) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("method=GET path=/v1/jobs/") && l.contains("/stems/"))
            .count()
    }

    pub(crate) fn job_dirs(&self) -> usize {
        fs::read_dir(self.outer.path().join("work/jobs"))
            .map(|r| r.flatten().count())
            .unwrap_or(0)
    }
}
