//! Locking tests of the editor IPC layer (plan 2.8): a stalled audio device must not freeze
//! the other commands, the ticker or the app exit. A fake backend whose `open` and handle
//! drop block on gates stands in for PipeWire/ALSA; no real device is ever opened.

use super::*;
use crate::audio_out::{ManualBackend, OutputBackend, OutputHandle, RenderFn};
use crate::picker::ScriptedPicker;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Condvar;
use std::time::Instant;

const FOUR: &str = "0199c0a0-0000-7000-8000-000000000201";
const MIXED: &str = "0199c0a0-0000-7000-8000-000000000202";
const WAIT: Duration = Duration::from_secs(5);

/// A door that is open or closed; `pass` blocks while it is closed.
#[derive(Clone)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn new(open: bool) -> Self {
        Self(Arc::new((Mutex::new(open), Condvar::new())))
    }
    fn set(&self, open: bool) {
        *self.0 .0.lock().unwrap() = open;
        self.0 .1.notify_all();
    }
    fn pass(&self) {
        let mut g = self.0 .0.lock().unwrap();
        while !*g {
            g = self.0 .1.wait(g).unwrap();
        }
    }
}

#[derive(Clone)]
struct FakeBackend {
    inner: ManualBackend,
    open_gate: Gate,
    drop_gate: Gate,
    fail_open: Arc<AtomicBool>,
    opens: Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            inner: ManualBackend::new(),
            open_gate: Gate::new(true),
            drop_gate: Gate::new(true),
            fail_open: Arc::new(AtomicBool::new(false)),
            opens: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

struct FakeHandle {
    inner: Option<Box<dyn OutputHandle>>,
    drop_gate: Gate,
}

impl OutputHandle for FakeHandle {
    fn error(&self) -> Option<String> {
        self.inner.as_ref().and_then(|h| h.error())
    }
}

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.drop_gate.pass();
        self.inner.take();
    }
}

impl OutputBackend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn open(&self, rate: u32, render: RenderFn) -> Result<Box<dyn OutputHandle>, String> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.open_gate.pass();
        if self.fail_open.load(Ordering::SeqCst) {
            return Err("The audio output failed to start".into());
        }
        let inner = self.inner.open(rate, render)?;
        Ok(Box::new(FakeHandle { inner: Some(inner), drop_gate: self.drop_gate.clone() }))
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

struct Env {
    _dir: tempfile::TempDir,
    repo: Arc<RepoState>,
    st: Arc<EditorState>,
    backend: FakeBackend,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library-editor"), &root);
    let repo = Arc::new(RepoState::new(root, Box::new(ScriptedPicker::new(dir.path().join("answers")))));
    let backend = FakeBackend::new();
    let st = Arc::new(EditorState::new(Box::new(backend.clone())));
    st.set_sink(Arc::new(|_| {}));
    Env { _dir: dir, repo, st, backend }
}

/// Opens FOUR with its first lane checked.
fn open_checked(e: &Env) {
    do_open_editor(&e.st, &e.repo, FOUR, Arc::new(|_| {})).unwrap();
    e.st.with(|m| m.set_stem(FOUR, "vocals", Some(0.0), true)).unwrap();
}

/// Runs `f` on a thread and fails when it does not finish in time.
fn finishes<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(WAIT).unwrap_or_else(|_| panic!("{what} did not finish: the manager is blocked"))
}

fn spawn_play(e: &Env) -> std::thread::JoinHandle<Result<TransportState, String>> {
    let st = e.st.clone();
    std::thread::spawn(move || do_editor_play(&st, FOUR))
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_stalled_device_open_does_not_block_other_commands_or_the_ticker() {
    let e = env();
    open_checked(&e);
    e.backend.open_gate.set(false);
    let play = spawn_play(&e);
    wait_until("the device open to start", || e.backend.opens.load(Ordering::SeqCst) == 1);
    let st = e.st.clone();
    finishes("other commands during the open", move || {
        st.tick(Instant::now());
        do_editor_seek(&st, FOUR, 1000).unwrap();
        st.with(|m| m.set_stem(FOUR, "drums", Some(-3.0), false)).unwrap();
        assert!(st.with(|m| m.current()).is_some());
    });
    e.backend.open_gate.set(true);
    let t = play.join().unwrap().unwrap();
    assert!(t.playing);
    assert_eq!(t.position_ms, 1000);
}

#[test]
fn a_stalled_device_close_does_not_block_other_commands_or_the_ticker() {
    let e = env();
    open_checked(&e);
    do_editor_play(&e.st, FOUR).unwrap();
    e.backend.drop_gate.set(false);
    let st = e.st.clone();
    let stop = std::thread::spawn(move || st.with(|m| m.stop(FOUR)));
    // The stop itself waits for the device (outside the lock); everything else goes on.
    wait_until("the stop to have published", || {
        e.st.with(|m| m.current().is_some_and(|s| !s.transport.playing))
    });
    let st = e.st.clone();
    finishes("other commands during the close", move || {
        st.tick(Instant::now());
        do_editor_seek(&st, FOUR, 500).unwrap();
        st.with(|m| m.set_stem(FOUR, "drums", Some(-3.0), true)).unwrap();
    });
    assert!(!stop.is_finished());
    e.backend.drop_gate.set(true);
    assert!(!stop.join().unwrap().unwrap().playing);
}

#[test]
fn closing_the_editor_with_a_stalled_device_leaves_the_manager_free() {
    let e = env();
    open_checked(&e);
    do_editor_play(&e.st, FOUR).unwrap();
    e.backend.drop_gate.set(false);
    let st = e.st.clone();
    let close = std::thread::spawn(move || do_close_editor(&st));
    wait_until("the session to be closed", || e.st.with(|m| m.current().is_none()));
    let st = e.st.clone();
    finishes("a command during the close", move || st.tick(Instant::now()));
    e.backend.drop_gate.set(true);
    close.join().unwrap();
}

#[test]
fn shutdown_returns_within_its_bound_when_the_manager_is_stuck_and_still_cancels_the_save() {
    let e = env();
    let shared = crate::backing_render::SaveShared::new();
    *e.st.live_save.lock().unwrap() = Some(shared.clone());
    let (held_tx, held_rx) = mpsc::channel();
    let release = Gate::new(false);
    let (st, gate) = (e.st.clone(), release.clone());
    let stuck = std::thread::spawn(move || {
        st.with(|_| {
            held_tx.send(()).unwrap();
            gate.pass();
        })
    });
    held_rx.recv_timeout(WAIT).unwrap();
    let t0 = Instant::now();
    let st = e.st.clone();
    finishes("shutdown", move || st.shutdown(Duration::from_millis(300)));
    assert!(t0.elapsed() < Duration::from_secs(2), "shutdown took {:?}", t0.elapsed());
    assert!(shared.cancel.load(Ordering::Relaxed), "the save was not cancelled");
    release.set(true);
    stuck.join().unwrap();
}

#[test]
fn shutdown_returns_within_its_bound_when_the_device_close_hangs() {
    let e = env();
    open_checked(&e);
    do_editor_play(&e.st, FOUR).unwrap();
    e.backend.drop_gate.set(false);
    let t0 = Instant::now();
    let st = e.st.clone();
    finishes("shutdown", move || st.shutdown(Duration::from_millis(300)));
    assert!(t0.elapsed() < Duration::from_secs(2), "shutdown took {:?}", t0.elapsed());
    e.backend.drop_gate.set(true); // lets the helper thread end
}

#[test]
fn shutdown_closes_the_session_normally() {
    let e = env();
    open_checked(&e);
    do_editor_play(&e.st, FOUR).unwrap();
    assert!(e.backend.inner.is_open());
    e.st.shutdown(Duration::from_secs(2));
    assert!(!e.backend.inner.is_open());
    assert!(e.st.with(|m| m.current()).is_none());
}

#[test]
fn a_failed_device_open_changes_no_state() {
    let e = env();
    do_open_editor(&e.st, &e.repo, MIXED, Arc::new(|_| {})).unwrap();
    let before = e.st.with(|m| m.snapshot(MIXED)).unwrap();
    e.backend.fail_open.store(true, Ordering::SeqCst);
    let name = before.stems.iter().find(|l| !l.unmuted).unwrap().name.clone();
    let err = do_editor_lane_play(&e.st, MIXED, &name).unwrap_err();
    assert!(err.contains("failed to start"), "{err}");
    let after = e.st.with(|m| m.snapshot(MIXED)).unwrap();
    assert_eq!(before.stems, after.stems, "the lane must not be checked");
    assert_eq!(after.transport.solo, None);
    assert!(!after.transport.playing);
    // Mix Play keeps the solo it had.
    e.backend.fail_open.store(false, Ordering::SeqCst);
    do_editor_lane_play(&e.st, MIXED, &name).unwrap();
    e.st.with(|m| m.stop(MIXED)).unwrap();
    e.st.with(|m| m.set_stem(MIXED, &name, Some(0.0), true)).unwrap();
    e.backend.fail_open.store(true, Ordering::SeqCst);
    assert!(do_editor_play(&e.st, MIXED).is_err());
    assert_eq!(e.st.with(|m| m.snapshot(MIXED)).unwrap().transport.solo, None);
}

#[test]
fn stop_silences_before_it_resets_the_position() {
    // The block rendered after Stop is silent and the position stays at 0.
    let e = env();
    open_checked(&e);
    do_editor_play(&e.st, FOUR).unwrap();
    assert!(e.backend.inner.pull(512).iter().any(|x| *x != 0.0));
    e.st.with(|m| m.stop(FOUR)).unwrap();
    assert!(e.backend.inner.pull(512).iter().all(|x| *x == 0.0));
    assert_eq!(e.st.with(|m| m.snapshot(FOUR)).unwrap().transport.position_ms, 0);
}

// ---- the event sink (m3, m4)

fn collecting() -> (EventSink, Arc<Mutex<Vec<EditorEvent>>>) {
    let got: Arc<Mutex<Vec<EditorEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    (Arc::new(move |ev| g.lock().unwrap().push(ev)), got)
}

#[test]
fn a_late_watch_does_not_take_the_sink_from_an_open_in_progress() {
    let e = env();
    let (open_sink, open_events) = collecting();
    let (watch_sink, watch_events) = collecting();
    // Hold the repository so the open stays in progress.
    let repo = e.repo.clone();
    let guard = repo.lock();
    let st = e.st.clone();
    let r2 = repo.clone();
    let open = std::thread::spawn(move || do_open_editor(&st, &r2, FOUR, open_sink));
    wait_until("the open to begin", || e.st.with(|m| m.is_opening()));
    assert!(do_watch_editor(&e.st, watch_sink.clone()).is_none());
    drop(guard);
    open.join().unwrap().unwrap();
    // The open's loading events went to the open's sink only.
    assert!(!open_events.lock().unwrap().is_empty());
    assert!(watch_events.lock().unwrap().is_empty());
    // A watch with no open running is a re-attach and does take the sink.
    let snap = do_watch_editor(&e.st, watch_sink).unwrap();
    assert_eq!(snap.id, FOUR);
    do_editor_seek(&e.st, FOUR, 500).unwrap();
    assert!(!watch_events.lock().unwrap().is_empty());
}

#[test]
fn a_failed_open_is_not_left_in_progress() {
    let e = env();
    assert!(do_open_editor(&e.st, &e.repo, "no-such-track", Arc::new(|_| {})).is_err());
    assert!(!e.st.with(|m| m.is_opening()));
    let (sink, _) = collecting();
    do_watch_editor(&e.st, sink);
}

#[test]
fn an_open_is_registered_before_the_record_is_read() {
    // A→B→A: the open claims its place (and cancels the older one) before the slow part.
    let e = env();
    let repo = e.repo.clone();
    let guard = repo.lock();
    let st = e.st.clone();
    let r2 = repo.clone();
    let a = std::thread::spawn(move || do_open_editor(&st, &r2, FOUR, Arc::new(|_| {})));
    wait_until("the first open to begin", || e.st.with(|m| m.is_opening()));
    let first = e.st.with(|m| m.generation_for_test());
    let st = e.st.clone();
    let r3 = repo.clone();
    let b = std::thread::spawn(move || do_open_editor(&st, &r3, MIXED, Arc::new(|_| {})));
    wait_until("the second open to begin", || e.st.with(|m| m.generation_for_test()) > first);
    drop(guard);
    let ra = a.join().unwrap();
    let rb = b.join().unwrap();
    // The older one is superseded, the newer one is the session.
    assert_eq!(ra.unwrap_err(), editor::SUPERSEDED);
    assert_eq!(rb.unwrap().id, MIXED);
    assert!(e.st.with(|m| m.is_open(MIXED)));
}
