# Plan: calliope-gui skeleton

## 1. Summary

Build the first `calliope-gui` binary: a single Rust crate (Rust code and static frontend
under `src/`) that uses Tauri 2 to open a window showing "hello from calliope" when run
without arguments, and prints the specified text for `--help` and `--version` without
opening a window. The version string `YY.MM.BBBB` is worked out at build time by `build.rs`.

## 2. Design

### Tooling found on this machine (2026-10-04)

| Tool | Status | Consequence |
|------|--------|-------------|
| cargo / rustc 1.92.0 (rustup) | present | Builds and tests run through plain `cargo`. |
| `cargo tauri` (tauri-cli) | **missing** | Not needed: we build with plain `cargo build`. Bundling (`.deb`, AppImage, …) is out of scope. |
| webkit2gtk-4.1 2.52.6, gtk+-3.0 3.24.52, libsoup-3.0 | present | Tauri 2 can compile and link on Linux. |
| node / npm | present | **Not used**: the frontend is one static HTML file, so there's no JS toolchain. |
| Xvfb / xvfb-run / weston / cage | **missing** | No headless display. GUI rendering can't be checked by agents; the GUI smoke test is `#[ignore]` and the user runs it (or installs `xorg-server-xvfb`). |
| tauri-driver / webkit2gtk-driver | missing | No WebDriver end-to-end tests. |
| Tauri crates in `~/.cargo/registry` | not cached | The first build needs network access to crates.io. |
| git, python3 | present | `build.rs` uses `git`. python3 may be used once to generate the placeholder icon. |

### Layout (single Cargo package at repo root)

```
Cargo.toml            package `calliope-gui`, bin `calliope-gui`, Cargo version 0.1.0
Cargo.lock            committed (binary crate)
build.rs              computes CALLIOPE_VERSION, then calls tauri_build::build()
tauri.conf.json       Tauri 2 config; frontendDist = "src/ui"
icons/icon.png        placeholder 32x32 RGBA icon (Tauri's generate_context! needs one)
src/main.rs           entry: parse args -> print/exit, or launch the GUI
src/cli.rs            pure arg parsing + help/version text (no Tauri dependency)
src/version.rs        pure version helpers; also used by build.rs via #[path]
src/gui.rs            tauri::Builder setup (only module that touches Tauri)
src/ui/index.html     static frontend: "hello from calliope"
tests/cli.rs          integration tests that run the built binary (headless)
tests/frontend.rs     static checks on src/ui/index.html and tauri.conf.json
tests/gui_smoke.rs    #[ignore] smoke test that needs a display
README.md             build/run instructions, system dependencies, version scheme
```

Why this layout: requirement 2 says "source code in `src`". Tauri's usual `src-tauri/` +
`src/` (JS) split would put the Rust code outside `src/`, so we use one Cargo package at the
root with the frontend in `src/ui/`. Later features add Rust modules under `src/`. If we ever
need extra crates, the root turns into a workspace and the new crates go under `src/<crate>/`
(see the architecture decision log).

### Startup flow

```
main() -> cli::parse_args(std::env::args().skip(1))
  Command::Help     -> print cli::help_text(VERSION) to stdout, exit 0
  Command::Version  -> print cli::version_line(VERSION) to stdout, exit 0
  Command::Gui      -> gui::run()   (tauri::Builder::default().run(generate_context!()))
  Command::Error(m) -> eprint "calliope-gui: {m}\n\n" + help text to stderr, exit 2
```

Arguments are parsed **before** any GTK/Tauri setup, so `--help` and `--version` work with
no display (that's what makes the CLI tests headless). The parser is written by hand, not
with clap, because the help text has to match requirement 8 exactly. Clap's generated help
looks different, and a parser for two flags is trivial.

### Key interfaces

```rust
// src/version.rs  (no external deps; compiled into both build.rs and the crate)
pub fn format_version(yy: u32, mm: u32, build: u32) -> String; // "26.10.0042", zero-padded 2/2/4
pub fn yymm_from_unix(secs: i64) -> (u32, u32);                 // UTC civil date -> (yy % 100, month)

// src/cli.rs
pub const BIN_NAME: &str = "calliope-gui";
#[derive(Debug, PartialEq, Eq)]
pub enum Command { Gui, Help, Version, Error(String) }
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Command;
pub fn help_text(version: &str) -> String;     // exact text below, ends with '\n'
pub fn version_line(version: &str) -> String;  // "calliope-gui {version}\n"

// src/main.rs
const VERSION: &str = env!("CALLIOPE_VERSION");   // set by build.rs
```

`parse_args` rules: no arguments gives `Gui`. Exactly one argument equal to `--help` gives
`Help`, and `--version` gives `Version`. Anything else (unknown option, extra arguments,
positional arguments) gives `Error("unknown option '<first offending arg>'")`, or
`Error("too many arguments")` when there are 2 or more arguments.

### Exact output (confirmed by the owner)

`calliope-gui --help` prints this. The 3-space markdown indent from the spec is removed, the
option lines keep their 3-space indent relative to the header, and there are no trailing
spaces. The output ends with a single `\n`:

```
calliope-gui, version YY.MM.BBBB
(c) 2026 Valentin Rusu

Usage: calliope-gui [options]

Options:
   --help: produces this output
   --version: produces short string containing the version number
```

`calliope-gui --version` prints `calliope-gui YY.MM.BBBB\n`.

### Version scheme (build.rs)

- `YY.MM` is the UTC build date. If `SOURCE_DATE_EPOCH` is set it's used as the date
  (reproducible builds), otherwise the current time.
- `BBBB` is the build sequence number, zero-padded to 4 digits:
  1. if `CALLIOPE_BUILD_NUMBER` is set (a release script or CI), use it; it must be an
     integer 0..=9999, otherwise the build fails with a clear message;
  2. otherwise use `git rev-list --count HEAD`, the number of commits. It goes up with every
     commit, needs no file to store, never dirties the tree, and gives the same number when
     the same commit is rebuilt;
  3. otherwise (no git, e.g. a source tarball) use `0`, and `cargo:warning` says so.
- `build.rs` emits `cargo:rustc-env=CALLIOPE_VERSION=<v>` and rerun triggers:
  `rerun-if-env-changed` for both env vars, and `rerun-if-changed` for `.git/HEAD` and
  `.git/refs/` (paths found with `git rev-parse --git-path`, so worktrees work), plus
  `src/version.rs`. Known limitation: the month only changes in the string when the build
  script runs again (after a new commit, a `cargo clean`, or an env change).
- The Cargo package version stays `0.1.0`. Semver forbids leading zeros (`26.10.0042`), so
  Cargo/Tauri metadata versions are kept separate from the user-facing calliope version.
- `build.rs` uses `#[path = "src/version.rs"] mod version;` so the format logic lives in one
  place and is unit-tested by `cargo test` through the crate.

### Tauri configuration

`tauri.conf.json` (Tauri 2 schema): `productName: "calliope-gui"`,
`identifier: "app.calliope.gui"`, `build.frontendDist: "src/ui"` (no devUrl, no
beforeBuildCommand), one window `{ "title": "calliope", "width": 800, "height": 600 }`,
`bundle.active: false`, `bundle.icon: ["icons/icon.png"]`. The `version` field is left out.
No IPC commands and no plugins: the page is static.

Dependencies: `tauri = "2"` (default features), build-dependency `tauri-build = "2"`. Nothing
else is needed.

Windows/macOS: out of scope for this feature. We don't set `windows_subsystem = "windows"`,
so `--help` stays visible in a Windows console; revisit when Windows support is a feature.

## 3. Tasks

Every task's test command is run from the repo root `/home/vali/src/calliope`.

### Task 1: Crate scaffold and version helpers
- **files**: `Cargo.toml`, `src/main.rs`, `src/version.rs`, `.gitignore`
- **does**: Create the package `calliope-gui` (edition 2021, version 0.1.0, explicit
  `[[bin]] name = "calliope-gui" path = "src/main.rs"`, no dependencies yet). `src/main.rs`
  declares `mod version;` and has an empty `main` (it may `#[allow(dead_code)]`). Implement
  `format_version` and `yymm_from_unix` in `src/version.rs`, with the civil-from-days
  algorithm (Howard Hinnant's) and no crates. Unit tests:
  `format_version(26,10,42) == "26.10.0042"`, `format_version(7,1,0) == "07.01.0000"`,
  `format_version(26,12,9999) == "26.12.9999"`, `yymm_from_unix(0) == (70,1)`,
  `yymm_from_unix(1_790_000_000) == (26,9)` (2026-09-21),
  `yymm_from_unix(1_798_761_599) == (26,12)` (2026-12-31T23:59:59Z),
  `yymm_from_unix(1_798_761_600) == (27,1)`. Leave `.gitignore` as it is if `target/` is
  already listed. Do not ignore `Cargo.lock`.
- **done when**: `cargo test` passes and `cargo build` produces `target/debug/calliope-gui`.
- **test command**: `cargo test`
- **routine**: yes

### Task 2: CLI parsing and help/version text
- **files**: `src/cli.rs`, `src/main.rs` (add `mod cli;` only)
- **does**: Implement `BIN_NAME`, `Command`, `parse_args`, `help_text` and `version_line`
  exactly as in Design > Key interfaces and Exact output. Unit tests: empty args give `Gui`;
  `["--help"]` gives `Help`; `["--version"]` gives `Version`; `["--foo"]` gives `Error`
  containing `--foo`; `["--help","--version"]` gives `Error`; `["file.mp3"]` gives `Error`.
  `help_text("26.10.0042")` equals the full expected string literal, byte for byte (first
  line `calliope-gui, version 26.10.0042`, the `(c) 2026 Valentin Rusu` line, option lines
  starting with 3 spaces, ending with exactly one `\n`, no trailing whitespace on any line).
  `version_line("26.10.0042") == "calliope-gui 26.10.0042\n"`.
- **done when**: `cargo test` passes, including the new `cli::tests`.
- **test command**: `cargo test`
- **routine**: yes

### Task 3: build.rs version generation and the CLI path in main
- **files**: `build.rs`, `src/main.rs`, `tests/cli.rs`
- **does**: Write `build.rs` as described in Design > Version scheme (env overrides, git
  commit count, fallback 0 with a warning, rerun triggers, reusing `src/version.rs` via
  `#[path]`). In `main.rs`, read `env!("CALLIOPE_VERSION")` and follow the startup flow for
  Help/Version/Error (exit codes 0/0/2). For now `Command::Gui` prints nothing and exits 0
  (placeholder until Task 4). Integration tests in `tests/cli.rs` run
  `env!("CARGO_BIN_EXE_calliope-gui")` with `DISPLAY` and `WAYLAND_DISPLAY` **removed** from
  the environment (this shows the CLI path is headless) and check:
  - `--version`: exit 0, stdout matches `^calliope-gui \d{2}\.\d{2}\.\d{4}\n$`, stderr empty;
  - `--help`: exit 0, stdout equals `help_text(v)`, where `v` is the version taken from the
    `--version` output (rebuild the expected literal in the test; don't import the crate);
    the first line matches `^calliope-gui, version \d{2}\.\d{2}\.\d{4}$`;
  - `--bogus`: exit code 2, stdout empty, stderr contains `unknown option '--bogus'` and
    `Usage: calliope-gui [options]`.
  - No regex crate: check digits by hand or with a small helper.
- **done when**: `cargo test` passes. `CALLIOPE_BUILD_NUMBER=42 cargo run -q -- --version`
  prints `calliope-gui YY.MM.0042` (current UTC year/month).
  `SOURCE_DATE_EPOCH=1790000000 CALLIOPE_BUILD_NUMBER=7 cargo run -q -- --version` prints
  `calliope-gui 26.09.0007`. `CALLIOPE_BUILD_NUMBER=abc cargo build` fails with a clear
  message.
- **test command**: `cargo test`
- **routine**: no (build-script rerun semantics, git/env edge cases)

### Task 4: Tauri window with "hello from calliope"
- **files**: `Cargo.toml`, `build.rs`, `tauri.conf.json`, `icons/icon.png`, `src/gui.rs`,
  `src/main.rs`, `src/ui/index.html`, `tests/frontend.rs`
- **does**: Add `tauri = "2"` and build-dependency `tauri-build = "2"`. At the end of
  `build.rs`, call `tauri_build::build()`. Write `tauri.conf.json` as in Design > Tauri
  configuration. Generate a valid 32x32 RGBA PNG placeholder at `icons/icon.png` (e.g. a
  one-off python3 script using `zlib`/`struct`; commit only the PNG). `src/ui/index.html` is
  a minimal HTML5 page with title `calliope` and a visible `<h1>hello from calliope</h1>`,
  and no external resources. `src/gui.rs` has
  `pub fn run() { tauri::Builder::default().run(tauri::generate_context!()).expect("error while running calliope-gui"); }`.
  `main.rs` calls `gui::run()` for `Command::Gui`. Make sure the CLI path still returns
  before any Tauri/GTK call. `tests/frontend.rs` (pure file checks, no display): it reads
  `src/ui/index.html` (path from `env!("CARGO_MANIFEST_DIR")`), which must contain
  `hello from calliope`, and parses `tauri.conf.json` (with `serde_json` as a
  dev-dependency) to check that `build.frontendDist == "src/ui"`,
  `productName == "calliope-gui"` and there is exactly one window.
- **done when**: `cargo build --release` succeeds and creates
  `target/release/calliope-gui`. `cargo test` passes, including all `tests/cli.rs` tests
  (still headless) and `tests/frontend.rs`. `cargo clippy --all-targets -- -D warnings`
  is clean.
- **test command**: `cargo test && cargo clippy --all-targets -- -D warnings`
- **routine**: no (framework integration; tauri-build/config/icon pitfalls)

### Task 5: Display-dependent GUI smoke test (ignored by default)
- **files**: `tests/gui_smoke.rs`
- **does**: One `#[test] #[ignore]` test. If neither `DISPLAY` nor `WAYLAND_DISPLAY` is
  set, print `skipping: no display` and return. Otherwise spawn
  `env!("CARGO_BIN_EXE_calliope-gui")` with no arguments, wait 5 seconds, and assert that
  `try_wait()` gives `None` (still running, so it didn't crash on startup). Then kill it
  and wait.
- **done when**: `cargo test` still passes (the test shows as ignored).
  `cargo test --test gui_smoke -- --ignored` compiles and prints the skip message in an
  agent shell with no display.
- **test command**: `cargo test && cargo test --test gui_smoke -- --ignored`
- **routine**: yes

### Task 6: README
- **files**: `README.md`
- **does**: Document the Arch system packages (`webkit2gtk-4.1`, `gtk3`, `base-devel`,
  plus optional `xorg-server-xvfb` for headless smoke runs), build (`cargo build --release`
  produces `target/release/calliope-gui`), run (`calliope-gui`, `--help`, `--version`), the
  version scheme (`YY.MM.BBBB`, `CALLIOPE_BUILD_NUMBER`, `SOURCE_DATE_EPOCH`,
  git-commit-count default), and the test commands from section 4 below.
- **done when**: `README.md` exists. `grep -q 'CALLIOPE_BUILD_NUMBER' README.md && grep -q 'cargo test' README.md && grep -q 'webkit2gtk-4.1' README.md` succeeds.
- **test command**: `grep -q 'CALLIOPE_BUILD_NUMBER' README.md && grep -q 'cargo test' README.md && grep -q 'webkit2gtk-4.1' README.md`
- **routine**: yes

## 4. Test strategy

- **Unit tests** (`cargo test`): version formatting and the date math (`src/version.rs`),
  argument parsing and exact help/version text (`src/cli.rs`).
- **Headless integration tests** (`tests/cli.rs`): run the real built binary with the
  display env vars removed, and check stdout/stderr/exit codes for `--help`, `--version`
  and an unknown option. This covers acceptance criteria 3 and 4 end to end.
- **Static frontend checks** (`tests/frontend.rs`): the page that gets embedded contains
  "hello from calliope", and `tauri.conf.json` points the window at it. This is the
  headless stand-in for acceptance criterion 2.
- **GUI smoke** (`tests/gui_smoke.rs`, `#[ignore]`): needs a display. Agents can't run it
  for real (no Xvfb on this machine). The user runs it on the desktop, or anywhere with
  `xvfb-run cargo test --test gui_smoke -- --ignored` after `pacman -S xorg-server-xvfb`.
- No external gear or network services are involved in this feature, so no simulators are
  needed. The first build downloads crates from crates.io.
- **Whole suite**: `cargo test && cargo clippy --all-targets -- -D warnings`

## 5. Deployment

Single machine (the laptop). The user builds locally with `cargo build --release` and runs
`target/release/calliope-gui`. There's no packaging or installation in this feature.

## 6. Manual checks (user, on the laptop desktop session)

1. `cargo build --release`: it finishes without errors and
   `ls target/release/calliope-gui` shows the binary.
2. `target/release/calliope-gui`: a window titled "calliope" opens and shows
   **hello from calliope**. Close the window: the process exits and the terminal prompt
   comes back.
3. `target/release/calliope-gui --help`: the text matches requirement 8 (with the
   decisions in section 8), and no window opens.
4. `target/release/calliope-gui --version`: prints `calliope-gui YY.MM.BBBB` with the
   current year/month and a 4-digit number, and no window opens.
5. Optional: `cargo test --test gui_smoke -- --ignored` passes in the desktop session.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests / checks |
|---|---|---|
| Compiling produces the `calliope-gui` binary | 1, 4 | `cargo build --release` creates `target/release/calliope-gui`; `CARGO_BIN_EXE_calliope-gui` used by `tests/cli.rs`; manual check 1 |
| No args: GUI visible, displays "hello from calliope" | 4, 5 | `tests/frontend.rs` (content + config wiring); `tests/gui_smoke.rs` (ignored, needs display); manual check 2 |
| `--help` shows requirement 8 output | 2, 3 | `cli::tests` (exact text); `tests/cli.rs` help test (headless); manual check 3 |
| `--version` shows requirement 9 output | 1, 2, 3 | `version::tests`, `cli::tests`; `tests/cli.rs` version test (headless); manual check 4 |
| Req 1-3 (Rust, `src/`, Tauri) | 1, 4 | layout above; `cargo build` |
| Req 4 (takes arguments) | 2, 3 | `cli::tests`, `tests/cli.rs` (incl. unknown-option exit 2) |
| Req 7 (date + build-time sequence number) | 1, 3 | `version::tests`; Task 3 `CALLIOPE_BUILD_NUMBER` / `SOURCE_DATE_EPOCH` checks |

## 8. Decisions (confirmed by the owner, 2026-10-04)

1. **Copyright year**: `(c) 2026`.
2. **Build number**: the git commit count (`git rev-list --count HEAD`), overridable with
   `CALLIOPE_BUILD_NUMBER`. It never resets and there is no counter file.
3. **Width**: 4 digits, zero-padded, in both `--help` and `--version`.
4. **Help text**: the spec's 3-space markdown indent is removed; the option lines keep their
   3-space indent relative to the header.

## 9. Assumptions (defaults, not commented on by the owner)

1. **Unknown or extra arguments**: print a message plus the help text to stderr, exit with
   code 2, and don't open a window. There are no `-h`/`-V` short aliases.
2. **Date used for `YY.MM`**: the UTC build date (or `SOURCE_DATE_EPOCH` if set), not the
   commit date.

## 10. Open questions

None.
