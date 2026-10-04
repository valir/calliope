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
| calliope-gui (GUI shell) | Laptop | Tauri 2 app: main window, plugins, managed state, IPC handler registration | Rust + Tauri 2 (webkit2gtk-4.1 on Linux), `tauri-plugin-window-state` | `src/gui.rs`, `tauri.conf.json` |
| calliope-gui (IPC commands) | Laptop | All `#[tauri::command]` functions; thin wrappers over pure modules | Rust | `src/ipc.rs` |
| calliope-gui (settings) | Laptop | Loads/saves user settings (theme now; MIDI/audio/server later) as JSON | Rust + serde | `src/settings.rs` |
| calliope-gui (frontend) | Laptop (embedded webview) | Navigation shell, views, theme, keyboard shortcuts | Svelte 5 + TypeScript + Vite 8, shadcn-svelte (bits-ui, Tailwind v4), Inter font; built to `dist/` and embedded at compile time | `src/ui/` |

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
- **Rust <-> frontend (Tauri IPC)**. The pattern every feature follows:
  - Rust: commands live in `src/ipc.rs` as thin `#[tauri::command]` wrappers that delegate to
    pure, unit-tested modules (e.g. `src/settings.rs`), and are registered in `src/gui.rs` with
    `tauri::generate_handler![...]`. They use snake_case names. Fallible commands return
    `Result<T, String>`, with a human-readable error. Shared state is `app.manage(...)`'d in
    `setup` and taken as `tauri::State<'_, T>`. Payload types derive serde
    `Serialize`/`Deserialize`; enums are lowercase strings.
  - Frontend: `src/ui/lib/ipc.ts` is the **only** module importing `@tauri-apps/api`. It has
    one typed wrapper per command, with TS types mirroring the Rust types. Components call the
    wrappers, never `invoke` directly, and tests fake IPC with `@tauri-apps/api/mocks`.
  - Permissions: app commands from the local origin need no capability (Tauri 2.12, as long
    as there is no app ACL manifest). No capabilities are defined. If JS ever needs a
    plugin/core API, add an inline capability under `app.security.capabilities` in
    `tauri.conf.json` with the minimum permissions.
  - Commands today: `app_version() -> String` (same value as `--version`),
    `get_settings() -> Settings`, `set_theme(theme) -> Result<Settings, String>`,
    `frontend_log(message)` (prints `calliope-ui: <message>` to stderr).
  - The frontend never keeps time. Audio/MIDI/sync logic stays in Rust; the frontend displays
    state and sends commands.
- **Frontend log lines** (stderr, prefix `calliope-ui: `), used by the GUI tests:
  `ready view=<id> theme=<t> version=<v>`, `view=<id>`, `theme=<t>`,
  `csp-violation directive=<d> blocked=<uri>`, `error <context>: <msg>`.
- **Settings file**: `$XDG_CONFIG_HOME/app.calliope.gui/settings.json` (Tauri
  `app_config_dir()`), e.g. `{"theme": "dark"}`. Missing or invalid files give the defaults;
  writes are atomic (tmp + rename); unknown fields are ignored. Window geometry is in
  `.window-state.json` in the same directory (written by `tauri-plugin-window-state` on
  graceful exit).
- **Window**: label `main`, title `calliope`, default 1280x800 centred, minimum 1024x640,
  created hidden and shown by the window-state plugin after it restores the geometry.

## Repository layout

```
Cargo.toml / Cargo.lock   single package `calliope-gui` at repo root (lock committed)
build.rs                  version generation, frontend-built check, tauri_build::build()
tauri.conf.json           Tauri 2 config (frontendDist = "dist", strict CSP, bundling disabled; never devUrl/devCsp)
tauri.dev.conf.json       dev-only overlay (devUrl, beforeDevCommand, devCsp), used only by `npm run dev:app`
.taurignore               keeps `tauri dev` from rebuilding Rust on frontend edits
package.json / -lock      npm project at the root (scripts below; lock committed)
vite.config.ts            Vite root = src/ui, outDir = dist/, $lib alias, vitest config
svelte.config.js, tsconfig.json, components.json (shadcn-svelte CLI config)
icons/                    app icons
src/                      ALL source
  *.rs                    Rust modules (main, cli, version, gui, ipc, settings, ...)
  ui/                     frontend: index.html, main.ts, app.css, App.svelte,
                          components/, views/ (one file per main view),
                          lib/ (ipc.ts, views.ts, theme.ts, app-state.svelte.ts,
                          utils.ts, components/ui/ = shadcn components owned by us),
                          *.test.ts next to the code
dist/                     generated by `vite build`, gitignored, embedded into the binary
tests/                    cargo integration tests (cli, frontend, acceptance_*, gui_smoke, gui_e2e)
specs/, docs/             specs, plans, architecture, UI guide (docs/ui.md)
```

All source lives under `src/` (an owner requirement). Config files stay at the root next to
`Cargo.toml`. If more crates are ever needed, the root becomes a Cargo workspace and the new
crates go in `src/<crate>/`.

## Build and run

- Prerequisites: Rust, `webkit2gtk-4.1 gtk3 base-devel`, Node.js >= 22.12 with npm; run
  `npm ci` once after cloning and after `package-lock.json` changes.
- Build: `npm run build:app` (= `vite build && cargo build --release`) gives
  `target/release/calliope-gui`.
- Run: `npm run app` (= `vite build && cargo run`).
- Plain `cargo build` does not run npm. It works once `dist/` exists. If
  `dist/index.html` is missing, `build.rs` stops with a message naming `npm run build:app`.
  If `src/ui/` is newer than `dist/`, it prints a `cargo:warning`.
- Dev: `npm run dev:app` (= `tauri dev --config tauri.dev.conf.json`, with `@tauri-apps/cli`
  as an npm dev-dependency). It starts the Vite dev server on `http://localhost:5173` (hot
  reload), then `cargo run` with the dev overlay passed in `TAURI_CONFIG`. The overlay has a
  dev-only `devCsp` = the production CSP plus `'unsafe-inline'` in `style-src` and
  `ws://localhost:5173` in `connect-src`, and nothing else.
- Release/normal builds are plain cargo without `TAURI_CONFIG`. Plain cargo builds are
  `tauri::is_dev()` (no `custom-protocol` feature), so Tauri would honour a devUrl/devCsp
  found in the config. Hence:
  - `tauri.conf.json` never contains `devUrl`/`devCsp`/`beforeDevCommand` (static test);
  - `build.rs` fails a release build whose `TAURI_CONFIG` mentions `devUrl`/`devCsp`, and
    skips the `dist/` checks only in debug dev runs.

## Frontend rules

- CSP-safe code (the production build runs under the strict CSP; dev mode's looser `devCsp`
  must not hide violations, so check with `npm run app` too): no inline `<script>`/`<style>` in HTML, no static `style="..."` attributes in
  `.svelte` files (Svelte parses them from an HTML template, so the CSP blocks them). Use
  Tailwind classes or `style:` directives (CSSOM, which is allowed). No `data:` assets
  (`assetsInlineLimit: 0`). No remote URLs. No libraries that inject inline scripts/styles
  at runtime (e.g. `mode-watcher`, `sonner` as shipped).
- Theme: `.dark` class on `<html>` (static in `index.html`, so first paint is dark), toggled
  by `src/ui/lib/theme.ts`. Tokens live in `src/ui/app.css` (shadcn neutral + amber accent).
- Shared UI state: `src/ui/lib/app-state.svelte.ts` (module-level `$state`). Views go in
  `src/ui/views/`; the view registry and shortcuts are in `src/ui/lib/views.ts`.
- Every view's heading is `<h1 id="view-heading" tabindex="-1">` (focus target after a
  shortcut).
- Look and behaviour: `docs/ui.md` (including "Decided by the team").

## Testing without hardware
<!-- Simulators/fakes per device, and how to run them. -->

- Whole headless suite: `npm test` (= `npm run check && npm run test:ui && npm run build && cargo test && cargo clippy --all-targets -- -D warnings`).
- Pure logic (parsing, formatting, settings, later: metadata, timing math) is unit-tested in
  its Rust module.
- CLI behaviour is tested headlessly by running the built binary (`CARGO_BIN_EXE_calliope-gui`)
  with `DISPLAY`/`WAYLAND_DISPLAY` removed.
- Static checks (`tests/frontend.rs`): strict production CSP (no `'unsafe-inline'`/`'unsafe-eval'`/`ws:`), no `devCsp`/`devUrl` in `tauri.conf.json`, `devCsp` in `tauri.dev.conf.json` = production CSP plus only the two allowed tokens, window config, no
  inline code in `src/ui/index.html` or `dist/index.html`, no static `style=` in `.svelte`,
  all assets local (woff2 bundled, no `http(s)://` or `data:` URLs in CSS).
- Frontend logic and components: vitest + jsdom + @testing-library/svelte, with IPC faked by
  `mockIPC` from `@tauri-apps/api/mocks` (`src/ui/**/*.test.ts`). Type check: `svelte-check`.
- **GUI on a real display**: agents have X display `:1` (i3, 1920x1200). `gui-shot <out.png>
  [window-regex] [timeout]` takes a screenshot (the agent then views the PNG), `xdotool` sends
  keys and clicks, and `i3-msg` floats, resizes and closes windows (i3 tiles windows and
  ignores the requested size/position unless they're floating). GUI tests are `#[ignore]`d
  cargo tests (`tests/gui_smoke.rs`, `tests/gui_e2e.rs`), run with
  `DISPLAY=:1 npm run test:gui`. They launch the real binary with temporary
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` (never the user's real config), assert
  on the `calliope-ui:` stderr lines and on the settings/window-state files, fail on any
  `csp-violation`, and save screenshots to `target/gui-shots/` for visual review. "Offline"
  is a launch under `unshare -rn` (no network namespace; X11 still reachable). Agents never
  open windows on any other display. (The earlier note that agents have no display server is
  outdated.)
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
**Superseded** by the gui-frontend-foundation toolchain decision below.

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

### 2026-10-04: Strict CSP for Tauri windows   (feature: gui-skeleton, reviewer follow-up)
Tauri windows run with a strict CSP (`app.security.csp` in `tauri.conf.json`: everything
`'self'`, `object-src 'none'`, no `unsafe-inline`/`unsafe-eval`; only the `ipc:` schemes are
allowed beyond `'self'`). Loosening it requires an explicit decision. Tauri's automatic
nonce/hash injection stays enabled. Enforced by a test in `tests/frontend.rs`.

### 2026-10-04: Frontend toolchain: Svelte 5 + TypeScript + Vite + shadcn-svelte   (feature: gui-frontend-foundation, owner decision)
Revisits "Static frontend, no node/npm toolchain": real views are coming (tablature with
alphaTab, editors, player). Stack chosen by the owner: Svelte 5 + TS + Vite 8, with
shadcn-svelte (bits-ui + Tailwind v4) components copied into `src/ui/lib/components/ui/`.
npm is the package manager (the only one installed); `package-lock.json` is committed.
TypeScript is pinned to ^6 because svelte-check 4.7 doesn't accept TS 7 yet. Inter is bundled
via @fontsource (no CDN).

### 2026-10-04: npm project at the root, Vite root `src/ui`, output `dist/`   (feature: gui-frontend-foundation)
Keeps all source in `src/` and `node_modules/` out of it. Config files sit next to
`Cargo.toml`. `dist/` is generated and gitignored. Alternative: package.json inside
`src/ui/` (rejected: `node_modules` inside `src/`, and two project roots).

### 2026-10-04: Plain cargo stays the Rust build; no tauri-cli; build.rs guards `dist/`   (feature: gui-frontend-foundation)
`npm run build:app` = `vite build && cargo build --release`. `build.rs` does not invoke
npm (keeps cargo fast, hermetic, and node-free for Rust-only work); it fails with a clear
message if `dist/index.html` is missing, and warns if it is stale. Alternative: the
`@tauri-apps/cli` npm package (`tauri build/dev`). Rejected for now: a large native
dependency only needed for bundling, which is out of scope. Revisit with installers.
Consequence: binaries are built without the `custom-protocol` feature (`tauri::is_dev()` is
true); with no `devUrl` the assets are embedded anyway.

### 2026-10-04: CSP unchanged; frontend written to fit it   (feature: gui-frontend-foundation)
Checked the sources of Svelte 5.57 and bits-ui 2.19: dynamic styles go through CSSOM (allowed);
component `<style>` blocks are extracted to CSS files by `vite build`; Svelte 5 transitions
use WAAPI. What would break the strict CSP, so we avoid it: static `style="..."` attributes,
`mode-watcher`'s inline script, Vite's `data:` asset inlining, and the Vite dev server/HMR.
The dev server is allowed only with a dev-only `devCsp` (see the next entries). A
`securitypolicyviolation` listener reports violations to stderr, and the GUI tests fail on them.

### 2026-10-04: IPC pattern: commands in `src/ipc.rs`, typed wrappers in `src/ui/lib/ipc.ts`, no capabilities   (feature: gui-frontend-foundation)
See Interfaces. With one module per side, the full surface is visible at a glance and the
frontend tests can fake it in one place. No capabilities are defined, because app commands
from the local origin are allowed by Tauri when there is no app ACL manifest. This is the
smallest possible surface.

### 2026-10-04: Settings owned by Rust in `app_config_dir()/settings.json`; window geometry via tauri-plugin-window-state   (feature: gui-frontend-foundation)
The theme is the first persisted setting. It's stored by Rust (not `localStorage`) because
the later settings (MIDI device, audio output, edge-AI server) are consumed by Rust, and a
plain JSON file is easy to inspect, hack, and redirect in tests via `XDG_CONFIG_HOME`.
Window size and position use the official `tauri-plugin-window-state` (saves on graceful
exit, restores before showing the hidden window) instead of hand-written code.

### 2026-10-04: GUI testing on display `:1`   (feature: gui-frontend-foundation)
Replaces the "no display server" assumption from gui-skeleton. Agents run `#[ignore]`d GUI
tests on `:1` with temporary XDG dirs, use `xdotool`/`i3-msg` for input and window
management, use structured `calliope-ui:` stderr lines for assertions, and use `gui-shot`
screenshots for visual review. No WebDriver (tauri-driver) is needed at this stage.

### 2026-10-04: Dev-only `devCsp` for the Vite dev server with hot reload   (feature: gui-frontend-foundation, owner decision)
The owner allowed relaxing the CSP **for dev runs only**, so the Vite dev server with hot
reload can be used. The production `app.security.csp` stays strict and unchanged. The
`devCsp` adds exactly `'unsafe-inline'` to `style-src` (Vite injects `<style>` elements) and
`ws://localhost:5173` to `connect-src` (HMR websocket); widening it further needs the
owner again. It lives in `tauri.dev.conf.json`, which only `npm run dev:app` merges in
(through tauri-cli `--config` and the `TAURI_CONFIG` env var), never in `tauri.conf.json`,
because plain cargo builds count as Tauri "dev" builds and would otherwise pick it up. A
static test compares the two CSPs, and `build.rs` refuses dev config in release builds.
`@tauri-apps/cli` is added as an npm dev-dependency for `tauri dev` only (beforeDevCommand,
waiting for the dev server, Rust rebuilds); release builds stay plain cargo.

### 2026-10-04: Navigation order Library, Import, Track, Playlists, Player, Settings   (feature: gui-frontend-foundation, owner decision)
Alt+1...Alt+6 follow this order (Alt+4 = Playlists, Alt+5 = Player). The single source is
`VIEWS` in `src/ui/lib/views.ts`.

