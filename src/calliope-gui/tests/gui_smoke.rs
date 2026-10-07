//! This test requires a display and is run with: cargo test --test gui_smoke -- --ignored

use std::env;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

/// Kills the child on drop, so a failing assertion never leaves a window behind.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore]
fn gui_starts_and_stays_up() {
    // If neither env var DISPLAY nor WAYLAND_DISPLAY is set, skip
    if env::var_os("DISPLAY").is_none() && env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("skipping: no display");
        return;
    }
    // Never touch the user's real config: use a fresh dir under target/.
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/gui-e2e/smoke");
    let _ = std::fs::remove_dir_all(&base);
    for d in ["config", "data", "cache"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let mut guard = Guard(
        Command::new(env!("CARGO_BIN_EXE_calliope-gui"))
            .env("XDG_CONFIG_HOME", base.join("config"))
            .env("XDG_DATA_HOME", base.join("data"))
            .env("XDG_CACHE_HOME", base.join("cache"))
            .spawn()
            .expect("failed to spawn gui"),
    );
    sleep(Duration::from_secs(5));
    assert!(
        guard.0.try_wait().unwrap().is_none(),
        "calliope-gui exited early"
    );
}
