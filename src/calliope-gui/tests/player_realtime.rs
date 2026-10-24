//! The audio callback never allocates (plan 2.4 of gui-player-speed): a counting global allocator watches
//! the thread that renders. Allocations made by the C++ time-stretcher go through `malloc` and are not
//! seen here; that part is covered by reading the code (the stretcher is built, and its buffers grown,
//! before the callback runs). That reading is of `signalsmith-stretch` 0.1.3, which Cargo.toml pins
//! (`=0.1.3`); read the new version again before moving it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;

use calliope_gui::player::{Beat, Cmd, Engine, LoopSpec, Shared};
use calliope_gui::player_source::Pcm;

struct CountingAlloc;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    if COUNTING.with(Cell::get) {
        ALLOCS.with(|a| a.set(a.get() + 1));
    }
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 22050;

#[test]
fn render_and_apply_never_allocate_in_stretch_mode() {
    let frames = 3 * RATE as usize;
    let samples: Vec<i16> = (0..frames)
        .map(|i| ((i as f32 * 0.05).sin() * 12000.0) as i16)
        .collect();
    let pcm = Arc::new(Pcm {
        rate: RATE,
        channels: 1,
        frames: frames as u64,
        samples,
    });
    let shared = Arc::new(Shared::default());
    let (tx, _rx) = sync_channel(8);
    // everything the commands carry is built before the callback runs, as the manager does
    let mut en = Engine::new(pcm, 0, shared.clone()).with_trash(tx);
    let grid: Arc<Vec<Beat>> = Arc::new((0..8).map(|k| (k * 11025, false)).collect());
    let grid2: Arc<Vec<Beat>> = Arc::new((0..8).map(|k| (k * 7000, false)).collect());
    let empty: Arc<Vec<Beat>> = Arc::new(Vec::new());
    let r = RATE as u64;
    let plain = LoopSpec {
        start: r / 4,
        end: r / 2,
        repeat: true,
        count_in_bpm_milli: None,
        count_in_lead: 0,
        training: None,
    };
    let counted = LoopSpec {
        start: r / 2,
        end: r,
        repeat: true,
        count_in_bpm_milli: Some(120_000),
        count_in_lead: 0,
        training: None,
    };
    let mut out = vec![0.0f32; 2 * 512];

    COUNTING.with(|c| c.set(true));
    ALLOCS.with(|a| a.set(0));
    en.apply(Cmd::SetGrid(empty));
    en.apply(Cmd::SetGrid(grid));
    en.apply(Cmd::SetMetronome(true));
    en.apply(Cmd::SetSpeed(85));
    en.apply(Cmd::SetLoop(Some(plain)));
    en.apply(Cmd::Seek(r / 5));
    en.apply(Cmd::Play { count_in: None });
    en.apply(Cmd::Resume);
    for _ in 0..20 {
        en.render(&mut out);
    }
    en.apply(Cmd::Jump(r / 10));
    for _ in 0..10 {
        en.render(&mut out);
    }
    en.apply(Cmd::SetSpeed(60)); // stretched -> stretched
    for _ in 0..10 {
        en.render(&mut out);
    }
    en.apply(Cmd::SetLoop(Some(counted)));
    en.apply(Cmd::SetGrid(grid2));
    en.apply(Cmd::Jump(r * 9 / 10));
    for _ in 0..50 {
        en.render(&mut out); // a wrap with count-in
    }
    en.apply(Cmd::SetSpeed(100)); // stretched at 100 %
    for _ in 0..30 {
        en.render(&mut out);
    }
    en.apply(Cmd::SetLoop(None));
    en.apply(Cmd::SetSpeed(70));
    en.apply(Cmd::Seek(r / 3));
    en.apply(Cmd::Resume);
    for _ in 0..10 {
        en.render(&mut out);
    }
    en.apply(Cmd::SetSpeed(100)); // stretched -> the bypass again, crossfaded
    for _ in 0..10 {
        en.render(&mut out);
    }
    en.apply(Cmd::SetSpeed(70)); // bypass -> stretched
    for _ in 0..4 {
        en.render(&mut out);
    }
    en.apply(Cmd::Jump(frames as u64 - r / 4));
    for _ in 0..40 {
        en.render(&mut out); // the track end
    }
    let n = ALLOCS.with(Cell::get);
    COUNTING.with(|c| c.set(false));
    assert_eq!(n, 0, "the callback allocated");
    assert!(shared.wraps.load(Ordering::Acquire) >= 2);
    assert!(shared.ended.load(Ordering::Acquire));
}
