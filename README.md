# calliope-gui

Rust desktop application with a Tauri 2 GUI; run with no arguments it opens a window showing "hello from calliope".

## Prerequisites (Arch Linux)

* Rust toolchain (cargo)
* Packages: webkit2gtk-4.1, gtk3, base-devel
* Optional: xorg-server-xvfb for headless GUI smoke runs
* No tauri-cli or node needed. First build downloads crates from crates.io.

## Build

```bash
cargo build --release
```

Produces `target/release/calliope-gui`.

## Run

```bash
calliope-gui
calliope-gui --help
calliope-gui --version
```

Unknown options print an error plus help to `stderr` and exit with code 2.

## Version scheme

`YY.MM.BBBB`

* `YY.MM` is the UTC build date (or from `SOURCE_DATE_EPOCH` if set).
* `BBBB` is the build number, zero-padded to 4 digits:
  * `CALLIOPE_BUILD_NUMBER` env var if set (0-9999)
  * otherwise the git commit count (`git rev-list --count HEAD`)
  * otherwise 0

Example:

```bash
CALLIOPE_BUILD_NUMBER=42 cargo build --release
```

## Tests

```bash
cargo test           # unit, CLI integration, static frontend checks
cargo clippy --all-targets -- -D warnings
```

GUI smoke test requires a display:

```bash
cargo test --test gui_smoke -- --ignored
# or headless
xvfb-run cargo test --test gui_smoke -- --ignored
```

## Project layout

* `src/main.rs` - entry, argument handling
* `src/cli.rs` - parsing and help/version text
* `src/version.rs` - version formatting
* `src/gui.rs` - Tauri integration
* `src/ui/index.html` - frontend page
* `build.rs` - version generation
* `tauri.conf.json`
* `tests/`
