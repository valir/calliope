//! This test requires a display and is run with: cargo test --test gui_smoke -- --ignored

use std::env;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

#[test]
#[ignore]
fn gui_starts_and_stays_up() {
    // If neither env var DISPLAY nor WAYLAND_DISPLAY is set, skip
    if env::var_os("DISPLAY").is_none() && env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("skipping: no display");
        return;
    }
    // Spawn the gui binary
    let mut child = Command::new(env!("CARGO_BIN_EXE_calliope-gui"))
        .spawn()
        .expect("failed to spawn gui");
    // Wait 5 seconds
    sleep(Duration::from_secs(5));
    // Check if process exited
    assert!(
        child.try_wait().unwrap().is_none(),
        "calliope-gui exited early"
    );
    // Kill the process
    let _ = child.kill();
    let _ = child.wait();
}
