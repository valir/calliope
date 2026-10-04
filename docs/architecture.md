# Architecture

<!-- Maintained by the architect agent. It records the technical decisions that span
     features, so each new feature stays consistent with the previous ones. You can edit
     it too; the architect treats your edits as decisions. -->

## Components
<!-- name | runs on | responsibility | language/stack | code location -->

| Name | Runs on | Responsibility | Stack | Code location |
|------|---------|----------------|-------|---------------|
| calliope-gui (CLI front) | Laptop | Parses arguments before any GUI init; prints `--help` / `--version` | Rust, hand-written parser (no clap) | `src/main.rs`, `src/cli.rs` |
| calliope-gui (version) | build time + Laptop | Formats the `YY.MM.BBBB` version; shared by `build.rs` and the crate | Rust, no deps | `src/version.rs`, `build.rs` |
| calliope-gui (GUI shell) | Laptop | Tauri 2 app that opens the main window | Rust + Tauri 2 (webkit2gtk-4.1 on Linux) | `src/gui.rs`, `tauri.conf.json` |
| calliope-gui (frontend) | Laptop (embedded webview) | UI pages, static HTML/CSS/JS embedded at compile time | Plain HTML/JS, no node toolchain | `src/ui/` |

## Interfaces
<!-- Between components and with external gear: protocol, message/data formats, timing. -->

- **Command line**: `calliope-gui` (no args) opens the GUI. `--help` prints the help text to
  stdout and exits 0. `--version` prints `calliope-gui YY.MM.BBBB\n` and exits 0. Any other
  argument prints a message plus help to stderr and exits 2. The CLI path never initialises
  GTK/Tauri, so it works without a display.
- **Version string**: `YY.MM.BBBB`. `YY.MM` is the UTC build date (`SOURCE_DATE_EPOCH` is
  used if set). `BBBB` is `CALLIOPE_BUILD_NUMBER` if set (0..=9999), else
  `git rev-list --count HEAD`, else 0, zero-padded to 4 digits. `build.rs` exposes it as the
  compile-time env `CALLIOPE_VERSION`. The Cargo/Tauri package version is separate (semver,
  currently `0.1.0`).
- **Rust <-> frontend**: none yet; the page is static. Later features add Tauri IPC
  commands in Rust modules under `src/` and list them here.

## Repository layout

```
Cargo.toml / Cargo.lock   single package `calliope-gui` at repo root (lock committed)
build.rs                  version generation + tauri_build::build()
tauri.conf.json           Tauri 2 config (frontendDist = "src/ui", bundling disabled)
icons/                    app icons
src/                      ALL source: Rust modules (*.rs) + src/ui/ (frontend)
tests/                    cargo integration tests (cli.rs, frontend.rs, gui_smoke.rs)
specs/, docs/             specs, plans, architecture
```

All source lives under `src/` (an owner requirement). If more crates are ever needed, the
root becomes a Cargo workspace and the new crates go in `src/<crate>/`.

## Testing without hardware
<!-- Simulators/fakes per device, and how to run them. -->

- Whole suite: `cargo test && cargo clippy --all-targets -- -D warnings`.
- Pure logic (parsing, formatting, later: metadata, timing math) is unit-tested in its module.
- CLI behaviour is tested headlessly by running the built binary (`CARGO_BIN_EXE_calliope-gui`)
  with `DISPLAY`/`WAYLAND_DISPLAY` removed.
- GUI: agent machines have no display server (no Xvfb, weston, cage or tauri-driver), so
  frontend content is checked statically (`tests/frontend.rs`). Display-dependent tests are
  `#[ignore]` and the user runs them (`cargo test -- --ignored`, or under `xvfb-run` once
  `xorg-server-xvfb` is installed). What the window actually shows is a manual check.
- Hardware (MIDI, audio) and the edge-AI server: no fakes yet. Each feature that adds one
  must also add a simulator/fake and list it here.

## Decision log
<!-- Newest last. Format:
### YYYY-MM-DD: <decision>   (feature: <spec>)
Context, the choice, alternatives considered, consequences. -->

### 2026-10-04: Single Cargo package at the repo root, all source in `src/`   (feature: gui-skeleton)
The owner requires source under `src/`. The standard Tauri layout (`src-tauri/` for Rust,
`src/` for JS) would break that. Choice: one package at the root, Rust in `src/*.rs`,
frontend in `src/ui/`. Alternative considered: the standard Tauri layout, rejected because of
the requirement. Consequence: `tauri.conf.json` sits at the root next to `Cargo.toml`.

### 2026-10-04: Static frontend, no node/npm toolchain   (feature: gui-skeleton)
The skeleton only shows a greeting, and the overview puts the logic (timing, MIDI) in Rust.
Choice: plain HTML/JS in `src/ui/`, embedded with `frontendDist`, built with plain
`cargo build`. tauri-cli isn't needed either. Alternative: Vite plus a JS framework; we can
revisit if the UI gets complex (tablature rendering may be the trigger).

### 2026-10-04: Hand-written argument parser   (feature: gui-skeleton)
The help text has an exact owner-specified format. clap's generated help can't match it
without fighting the library, and there are only two flags. Revisit when the CLI grows.

### 2026-10-04: Version = build date + git commit count   (feature: gui-skeleton, confirmed by the owner)
`YY.MM.BBBB`, computed in `build.rs`. Choice: the git commit count, overridable with
`CALLIOPE_BUILD_NUMBER`. Alternatives: a committed counter file (dirty tree, merge
conflicts) or a per-compile counter (not reproducible, unreliable with cargo's build-script
caching). Consequence: the number goes up with the commit history and never resets (owner decision:
no counter file, 4 digits zero-padded in both `--help` and `--version`); `YY.MM` only updates when the build script reruns. The Cargo version stays semver
(`0.1.0`) because `26.10.0042` isn't valid semver.

### 2026-10-04: Parse CLI before GUI init   (feature: gui-skeleton)
`--help`/`--version` must work in terminals and in headless CI/agent shells, so
`main` dispatches on the arguments before touching Tauri/GTK.
