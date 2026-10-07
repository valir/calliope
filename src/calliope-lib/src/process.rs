//! Runs an external program: argv only (never a shell), stdin closed, output delivered line by
//! line to callbacks, and cancellable. On Linux the child leads its own process group (so
//! cancelling also stops its children) and dies with the spawning thread (`PR_SET_PDEATHSIG`;
//! the thread that spawned must therefore stay alive until `wait` returns).

use std::ffi::OsStr;
use std::io::{self, BufRead, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Time between the polite stop request (SIGTERM) and the forced one (SIGKILL).
pub const KILL_GRACE: Duration = Duration::from_secs(2);
/// Longest line delivered to a callback (the rest of a longer line is dropped).
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Cheap, cloneable handle that can stop the child from any thread.
#[derive(Clone)]
pub struct CancelHandle {
    pid: u32,
    /// Set once the leader has been reaped.
    reaped: Arc<AtomicBool>,
    #[cfg(not(unix))]
    child: Arc<Mutex<Child>>,
}

pub struct Running {
    child: Arc<Mutex<Child>>,
    handle: CancelHandle,
    readers: Vec<JoinHandle<()>>,
}

fn spawn_error(program: &OsStr, e: &io::Error) -> io::Error {
    let name = program.to_string_lossy();
    let msg = match e.kind() {
        io::ErrorKind::NotFound => format!("could not start {name}: program not found"),
        io::ErrorKind::PermissionDenied => format!("could not start {name}: permission denied"),
        _ => format!("could not start {name}: {e}"),
    };
    io::Error::new(e.kind(), msg)
}

/// Starts `program` with `args` (each one argv entry), optionally in `cwd`. Every complete line
/// of stdout / stderr (without the line ending, lossily decoded) is passed to the callbacks,
/// which run on reader threads.
pub fn spawn<S: AsRef<OsStr>>(
    program: impl AsRef<OsStr>,
    args: &[S],
    cwd: Option<&Path>,
    on_stdout_line: impl FnMut(String) + Send + 'static,
    on_stderr_line: impl FnMut(String) + Send + 'static,
) -> io::Result<Running> {
    let program = program.as_ref();
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: only an async-signal-safe call (prctl) runs between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    let pid = child.id();
    let out = child.stdout.take().expect("piped stdout");
    let err = child.stderr.take().expect("piped stderr");
    let readers = vec![
        std::thread::spawn(move || read_lines(out, on_stdout_line)),
        std::thread::spawn(move || read_lines(err, on_stderr_line)),
    ];
    let child = Arc::new(Mutex::new(child));
    let handle = CancelHandle {
        pid,
        reaped: Arc::new(AtomicBool::new(false)),
        #[cfg(not(unix))]
        child: child.clone(),
    };
    Ok(Running { child, handle, readers })
}

fn read_lines(source: impl Read, mut on_line: impl FnMut(String)) {
    let mut reader = io::BufReader::new(source);
    let mut line: Vec<u8> = Vec::new();
    let mut overflow = false;
    loop {
        let (used, found) = match reader.fill_buf() {
            Ok([]) => break,
            Ok(buf) => match buf.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    push_capped(&mut line, &mut overflow, &buf[..i]);
                    (i + 1, true)
                }
                None => {
                    push_capped(&mut line, &mut overflow, buf);
                    (buf.len(), false)
                }
            },
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        reader.consume(used);
        if found {
            deliver(&mut line, &mut on_line);
            overflow = false;
        }
    }
    if !line.is_empty() {
        deliver(&mut line, &mut on_line);
    }
}

fn push_capped(line: &mut Vec<u8>, overflow: &mut bool, data: &[u8]) {
    let room = MAX_LINE_BYTES.saturating_sub(line.len());
    if data.len() > room {
        *overflow = true;
    }
    line.extend_from_slice(&data[..data.len().min(room)]);
}

fn deliver(line: &mut Vec<u8>, on_line: &mut impl FnMut(String)) {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    on_line(String::from_utf8_lossy(line).into_owned());
    line.clear();
}

impl CancelHandle {
    /// Asks the child (and its process group) to stop: SIGTERM now, SIGKILL after
    /// [`KILL_GRACE`] for whatever is still running. Returns at once.
    pub fn cancel(&self) {
        #[cfg(unix)]
        {
            signal_group(self.pid, libc::SIGTERM);
            let pid = self.pid;
            std::thread::spawn(move || {
                // Group members may outlive the leader, so the forced kill is sent regardless;
                // a group that is already gone makes it a harmless ESRCH.
                std::thread::sleep(KILL_GRACE);
                signal_group(pid, libc::SIGKILL);
            });
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.lock().map(|mut c| c.kill());
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// True once the child has been reaped by `wait`.
    pub fn is_finished(&self) -> bool {
        self.reaped.load(Ordering::SeqCst)
    }
}

#[cfg(unix)]
fn signal_group(pgid: u32, signal: libc::c_int) {
    // The child is its own group leader (process_group(0)), so its pid is the group id.
    // SAFETY: plain syscall with integer arguments.
    unsafe {
        libc::kill(-(pgid as libc::pid_t), signal);
    }
}

impl Running {
    pub fn id(&self) -> u32 {
        self.handle.pid
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        self.handle.clone()
    }

    /// See [`CancelHandle::cancel`].
    pub fn cancel(&self) {
        self.handle.cancel();
    }

    /// Waits for the child to exit and for all its output to be delivered. Output stays open
    /// as long as any descendant holds it, so a descendant that outlives its parent delays this
    /// until it ends (cancel kills the whole group).
    pub fn wait(self) -> io::Result<ExitStatus> {
        let status = {
            #[cfg(unix)]
            {
                // Nothing else locks the child on unix (cancel uses signals only).
                self.child.lock().expect("child lock").wait()?
            }
            #[cfg(not(unix))]
            {
                // Poll so that cancel (Child::kill) can take the lock.
                loop {
                    let r = self.child.lock().expect("child lock").try_wait();
                    match r? {
                        Some(s) => break s,
                        None => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            }
        };
        self.handle.reaped.store(true, Ordering::SeqCst);
        for r in self.readers {
            let _ = r.join();
        }
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    fn sh(script: &str, out: impl FnMut(String) + Send + 'static, err: impl FnMut(String) + Send + 'static)
        -> io::Result<Running> {
        spawn("bash", &["-c", script], None, out, err)
    }

    #[test]
    fn delivers_lines_and_exit_status() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let (o, e) = (lines.clone(), lines.clone());
        let r = sh(
            "echo one; echo two >&2; printf 'three\\r\\n'; printf 'tail-no-newline'; exit 3",
            move |l| o.lock().unwrap().push(format!("out:{l}")),
            move |l| e.lock().unwrap().push(format!("err:{l}")),
        )
        .unwrap();
        let st = r.wait().unwrap();
        assert_eq!(st.code(), Some(3));
        let mut got = lines.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, ["err:two", "out:one", "out:tail-no-newline", "out:three"]);
    }

    #[test]
    fn args_are_not_interpreted_by_a_shell() {
        let (tx, rx) = mpsc::channel();
        let r = spawn("printf", &["%s\\n", "a b; echo hacked", "$HOME"], None, move |l| tx.send(l).unwrap(), |_| {})
            .unwrap();
        assert!(r.wait().unwrap().success());
        assert_eq!(rx.iter().collect::<Vec<_>>(), ["a b; echo hacked", "$HOME"]);
    }

    #[test]
    fn runs_in_the_given_directory_with_closed_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let r = spawn("bash", &["-c", "pwd; cat; echo eof"], Some(dir.path()), move |l| tx.send(l).unwrap(), |_| {})
            .unwrap();
        assert!(r.wait().unwrap().success());
        let got: Vec<String> = rx.iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(std::fs::canonicalize(&got[0]).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
        assert_eq!(got[1], "eof");
    }

    #[test]
    fn missing_program_is_a_clear_error() {
        let e = spawn("/nonexistent/calliope-no-such-tool", &["x"], None, |_| {}, |_| {}).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        let text = e.to_string();
        assert!(text.contains("calliope-no-such-tool") && text.contains("not found"), "{text}");
    }

    #[test]
    fn overlong_lines_are_capped() {
        let (tx, rx) = mpsc::channel();
        let r = sh("head -c 200000 /dev/zero | tr '\\0' x; echo; echo next", move |l| tx.send(l.len()).unwrap(), |_| {})
            .unwrap();
        r.wait().unwrap();
        assert_eq!(rx.iter().collect::<Vec<_>>(), [MAX_LINE_BYTES, 4]);
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        // A zombie still answers kill -0 but is dead for our purposes.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => !s.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'),
            Err(_) => false,
        }
    }

    #[cfg(unix)]
    fn wait_gone(pid: i32) -> bool {
        for _ in 0..100 {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn cancel_kills_the_script_and_its_grandchild() {
        let (tx, rx) = mpsc::channel();
        // The grandchild's pid is announced on stdout; stdout stays open through it.
        let r = sh("sleep 60 & echo $!; wait", move |l| tx.send(l).unwrap(), |_| {}).unwrap();
        let grandchild: i32 = rx.recv_timeout(Duration::from_secs(5)).unwrap().trim().parse().unwrap();
        assert!(alive(grandchild));
        let leader = r.id() as i32;
        let started = Instant::now();
        r.cancel();
        let st = r.wait().unwrap();
        assert!(!st.success());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(wait_gone(grandchild), "grandchild {grandchild} survived");
        assert!(wait_gone(leader));
    }

    #[cfg(unix)]
    #[test]
    fn cancel_escalates_to_sigkill_for_a_process_that_ignores_sigterm() {
        let (tx, rx) = mpsc::channel();
        let r = sh("trap '' TERM; (trap '' TERM; sleep 60) & echo $!; wait", move |l| tx.send(l).unwrap(), |_| {})
            .unwrap();
        let grandchild: i32 = rx.recv_timeout(Duration::from_secs(5)).unwrap().trim().parse().unwrap();
        let started = Instant::now();
        r.cancel();
        let _ = r.wait().unwrap();
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(1500) && took < Duration::from_secs(8), "{took:?}");
        assert!(wait_gone(grandchild));
    }

    #[cfg(unix)]
    #[test]
    fn cancel_handle_works_from_another_thread() {
        let r = sh("sleep 60", |_| {}, |_| {}).unwrap();
        let h = r.cancel_handle();
        let pid = h.pid() as i32;
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            h.cancel();
        });
        let st = r.wait().unwrap();
        t.join().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(st.signal(), Some(libc::SIGTERM));
        assert!(wait_gone(pid));
    }
}
