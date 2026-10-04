# Plan: calliope-gui frontend foundation

## 1. Summary

Replace the static "hello" page with a Svelte 5 + TypeScript + Vite frontend, styled with
shadcn-svelte (Tailwind v4, copied into the repo) with a dark default and a light theme, and
built into `dist/` that Tauri embeds. The app gets a navigation shell (collapsible left nav,
content area, slim footer) with placeholder views for Library, Import, Track, Playlists,
Player and Settings. Keyboard shortcuts switch views. The first Tauri IPC commands are
`app_version`, settings (theme) and a frontend log line. Window size and position are kept
across runs, and the minimum size is 1024x640. The production CSP stays strict and unchanged.
A dev-only `devCsp` (owner decision) allows the Vite dev server with hot reload, and only in
`npm run dev:app`.

## 2. Design

### Tooling found on this machine (2026-10-04)

| Tool | Status | Consequence |
|------|--------|-------------|
| node v26.10.0, npm 12.1.0 | present | npm is the JS package manager (no pnpm/bun). `package-lock.json` is committed. |
| rustc/cargo 1.92.0 | present | unchanged |
| tauri 2.12.1, tauri-build 2.7.1, wry 0.57 (in Cargo.lock) | present | `@tauri-apps/api` ^2.12 matches. `tauri-plugin-window-state` 2.5.0 is current. |
| tauri-cli (`cargo tauri` / `@tauri-apps/cli`) | missing | **`@tauri-apps/cli` ^2.12 added as an npm dev-dependency, used only by `npm run dev:app`** (dev server + hot reload). Release builds stay plain cargo (see "Build and run"). |
| npm latest: svelte 5.57, vite 8.3, @sveltejs/vite-plugin-svelte 7.3 (needs vite 8), tailwindcss + @tailwindcss/vite 4.3, bits-ui 2.19, shadcn-svelte CLI 1.7, vitest 5.0, jsdom 30, @testing-library/svelte 5.4, svelte-check 4.7 | available | **TypeScript must be pinned to ^6**: `latest` is 7.0, but svelte-check 4.7 accepts only `^5 || ^6`. |
| X display `:1` (i3 4.25, 1920x1200), `gui-shot`, `xdotool`, `maim`, `xprop`, `i3-msg` | present | Real GUI checks: screenshots, synthetic keys/clicks, and `i3-msg` to float/resize/close the window. i3 tiles windows (ignores requested size/position) unless they are floating. i3 `$mod` is Super; kill is `Super+Shift+a` on this config (tests use `i3-msg`, not keybindings). |
| `unshare -rn` (unprivileged user + network namespace) | works, X11 reachable inside | Automated "offline" launch of the app. |
| wmctrl, Xvfb | missing | Not needed. |

### CSP findings (requirement 8): no relaxation needed for the production build

Checked against the actual package sources (svelte 5.57.1, bits-ui 2.19.5, shadcn-svelte registry):

- **Svelte dynamic styles** (`style:prop={x}`, `style={expr}`, and style props spread by bits-ui
  for floating positioning) compile to `set_style()`, which uses `el.style.cssText` /
  `el.style.setProperty()` (CSSOM). CSP `style-src` does **not** block CSSOM writes. OK.
- **Static `style="..."` attributes in our own `.svelte` templates** are compiled into the
  HTML template string (`$.from_html(...)`) and parsed. CSP **blocks** them under
  `style-src 'self'`. Rule: **never write a static `style="…"` attribute**; use Tailwind
  classes or a `style:` directive. Enforced by a test (Task 3). The shadcn components we use
  (button, input, label, dialog, tabs, radio-group, separator, card) contain none. `sonner`
  does, so we don't use it.
- **Component `<style>` blocks** (ours, and bits-ui's select/scroll-area) are extracted into
  `.css` files by `vite build`. OK. bits-ui's body scroll lock uses CSSOM. OK.
- **Svelte 5 transitions** use the Web Animations API, not injected `<style>` elements. OK.
- **shadcn's `mode-watcher`** injects an inline `<script>` to prevent theme flashes, which the
  CSP blocks. **Not used**; we have our own small theme module.
- **Vite asset inlining** would turn small assets (font subsets) into `data:` URLs, and
  `default-src 'self'` blocks `data:` fonts. Fix: `build.assetsInlineLimit: 0`.
- **Tauri nonce/hash injection** only acts on inline `<script>`/`<style>` in the HTML. The Vite
  output has none, so nothing gets hashed, and the Tauri IPC init scripts are WebKit user
  scripts, which the CSP doesn't apply to. App commands go over `ipc:` /
  `http://ipc.localhost`, which the CSP already allows.
- **Vite dev server / HMR** needs `connect-src ws://localhost:5173` (the HMR websocket) and
  `style-src 'unsafe-inline'` (Vite injects CSS as `<style>` elements in dev). **Owner
  decision: allowed in a dev-only `devCsp`**, which lives only in `tauri.dev.conf.json` and
  is used only by `npm run dev:app` (see "Dev mode"). The production `csp` stays exactly as
  it is. Nothing else is relaxed: no `'unsafe-eval'`, no other hosts, no `data:`/`blob:`. If
  the dev server turns out to need more, stop and ask the owner. The production safeguards
  above still apply, because the shipped build must work under the strict CSP.
- Runtime guard: `main.ts` listens for `securitypolicyviolation` and reports each one to
  stderr via IPC (`calliope-ui: csp-violation …`). The GUI e2e test fails on any such line.

### Layout (config at the root, all source under `src/`)

```
package.json, package-lock.json    npm project at the repo root (scripts below)
vite.config.ts                     root: src/ui, outDir: dist/, $lib alias, vitest config
svelte.config.js                   vitePreprocess
tsconfig.json                      strict TS, paths $lib -> src/ui/lib
components.json                    shadcn-svelte CLI config (aliases into src/ui/lib)
dist/                              BUILD OUTPUT of `vite build` (gitignored), embedded by Tauri
Cargo.toml, build.rs, tauri.conf.json, icons/   as before (frontendDist -> "dist")
tauri.dev.conf.json                dev-only config: devUrl, beforeDevCommand, devCsp (used only by npm run dev:app)
.taurignore                        keeps tauri dev from rebuilding Rust on frontend edits
src/*.rs                           Rust: main, cli, version, gui, ipc (new), settings (new)
src/ui/index.html                  Vite entry HTML (no inline code)
src/ui/main.ts                     bootstrap: CSP listener, load theme, mount App
src/ui/app.css                     Tailwind v4 + tw-animate-css + Inter font + theme tokens
src/ui/App.svelte                  shell: NavBar | view | StatusFooter, global shortcuts
src/ui/components/                 NavBar, StatusFooter, AboutDialog, ViewPlaceholder
src/ui/views/                      LibraryView … SettingsView (one file per view)
src/ui/lib/views.ts                view registry + shortcut mapping (pure, unit-tested)
src/ui/lib/ipc.ts                  the ONLY module importing @tauri-apps/api; typed wrappers
src/ui/lib/theme.ts                applyTheme()
src/ui/lib/app-state.svelte.ts     shared rune state (view, theme, version, navCollapsed)
src/ui/lib/utils.ts                shadcn `cn()` helper (generated)
src/ui/lib/components/ui/          shadcn-svelte components (generated, then owned by us)
src/ui/**/*.test.ts                vitest unit/component tests next to the code
tests/                             cargo tests (cli, frontend static checks, acceptance, gui_*)
```

Why: the config files (`package.json`, `vite.config.ts`, …) sit at the root next to
`Cargo.toml` and `tauri.conf.json`. They're config, not source, and `node_modules/` stays out
of `src/`. Vite's `root` is `src/ui`, so every frontend source file lives in `src/ui/`. The
output goes to `dist/` at the root: generated, gitignored, and not source.

### Build and run (requirement 2)

npm scripts (in `package.json`):

| Script | Runs | Purpose |
|--------|------|---------|
| `npm run build:app` | `vite build && cargo build --release` | **the documented build command** gives `target/release/calliope-gui` |
| `npm run app` | `vite build && cargo run` | **the documented run command** (debug build, opens the window) |
| `npm run dev:app` | `tauri dev --config tauri.dev.conf.json` | **the documented dev command**: Vite dev server with hot reload + debug app (see "Dev mode") |
| `npm run dev:ui` | `vite --port 5173 --strictPort --host localhost` | dev server only (started by `dev:app` via `beforeDevCommand`) |
| `npm run build` | `vite build` | frontend only, into `dist/` |
| `npm run check` | `svelte-check --tsconfig ./tsconfig.json --fail-on-warnings` | type check |
| `npm run test:ui` | `vitest run` | frontend unit/component tests |
| `npm test` | `npm run check && npm run test:ui && npm run build && cargo test && cargo clippy --all-targets -- -D warnings` | **whole suite** (headless) |
| `npm run test:gui` | `npm run build && cargo test --test gui_smoke --test gui_e2e -- --ignored --test-threads=1` | GUI tests on a display (`DISPLAY=:1` for agents) |

Prerequisite (documented in the README): run `npm ci` once after cloning, and again whenever
`package-lock.json` changes.

**Plain `cargo build`**: `build.rs` does not run npm, so cargo stays fast and never needs node.
Instead:
- if `dist/index.html` is missing, the build fails with
  `error: the frontend is not built (dist/index.html is missing). Build the app with: npm ci && npm run build:app   (or only the frontend: npm run build)`.
  This check runs before `tauri_build::build()`, so the developer sees this message instead
  of Tauri's generic one.
- if any file under `src/ui/` is newer than `dist/index.html`, it prints a
  `cargo:warning=` saying the frontend in `dist/` is stale and to run `npm run build`
  (a warning, not an error, so `cargo test` keeps working while developing Rust code).
- `cargo:rerun-if-changed=src/ui` and `=dist/index.html` keep the check current.

So once `dist/` exists, plain `cargo build` still works. In a fresh clone it fails with a
clear message. Tauri embeds `dist/` at compile time because there's no `devUrl`
(checked in tauri-codegen 2.7.1: assets are embedded unless dev mode *and* a devUrl are both
set), so the binary needs no files at runtime.

Why release builds stay plain cargo: it already builds a working binary, and the build
command doesn't depend on tauri-cli. Consequence: the binary is built without the
`custom-protocol` feature, so `tauri::is_dev()` is true even in `--release`. Tauri would
therefore use a `devUrl`/`devCsp` from the config in **any** plain cargo build. That's why
`tauri.conf.json` must never contain `devUrl`, `devCsp` or `beforeDevCommand`; they live
only in `tauri.dev.conf.json` (see "Dev mode"). Devtools follow `debug_assertions` (on in
debug builds, off in `--release`).

### Dev mode with hot reload (owner decision)

- `npm run dev:app` runs `tauri dev --config tauri.dev.conf.json` (`@tauri-apps/cli`
  dev-dependency). tauri-cli runs `beforeDevCommand` (`npm run dev:ui`), waits for
  `http://localhost:5173`, then runs `cargo run` with the merged config passed in the
  `TAURI_CONFIG` env var (supported by tauri-build 2.7.1 / tauri-codegen 2.7.1, which also
  emit `rerun-if-env-changed=TAURI_CONFIG`, so switching between dev and normal runs rebuilds).
  It rebuilds and restarts the Rust side when Rust files change.
- `tauri.dev.conf.json` (root, config only; **not** auto-merged by Tauri, which merges only
  platform files like `tauri.linux.conf.json`):
  ```json
  {
    "build": {
      "devUrl": "http://localhost:5173",
      "beforeDevCommand": "npm run dev:ui"
    },
    "app": {
      "security": {
        "devCsp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self' ipc: http://ipc.localhost ws://localhost:5173; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
      }
    }
  }
  ```
  The devCsp is the production CSP plus exactly two tokens: `'unsafe-inline'` in `style-src`
  and `ws://localhost:5173` in `connect-src`.
- `.taurignore` (root) lists `src/ui/`, `dist/` and `node_modules/`, so frontend edits are
  handled by Vite HMR instead of triggering a Rust rebuild.
- `vite.config.ts` `server: { port: 5173, strictPort: true, host: 'localhost' }`.
- **Release builds never use devUrl/devCsp**, enforced in three places:
  1. `tauri.conf.json` contains no `devUrl`/`devCsp`/`beforeDevCommand` (static test);
  2. `npm run build:app` / `npm run app` are plain cargo without `TAURI_CONFIG`;
  3. `build.rs`: if `PROFILE == "release"` and `TAURI_CONFIG` mentions `devUrl` or `devCsp`, the
     build fails with `error: dev configuration (devUrl/devCsp) must not be used in a release build; use npm run build:app`.
     When `TAURI_CONFIG` sets a `devUrl` in a debug build, the `dist/` checks are skipped
     (dev mode doesn't embed `dist/`).

### Frontend stack and theme

- **Svelte 5** (runes), **TypeScript** (strict), **Vite 8** with `@sveltejs/vite-plugin-svelte`
  and `@tailwindcss/vite`. Vite settings: `root: 'src/ui'`, `build.outDir: '../../dist'`,
  `emptyOutDir: true`, `assetsInlineLimit: 0`, `modulePreload: { polyfill: false }`,
  `resolve.alias: { $lib: <abs>/src/ui/lib }`. Vitest runs in `jsdom` with
  `resolve.conditions: ['browser']` when `process.env.VITEST` is set, and includes
  `src/ui/**/*.test.ts`.
- **shadcn-svelte** (CLI 1.7, base colour `neutral`). Components: button, input, label,
  dialog, tabs, radio-group, separator, card. They get copied into
  `src/ui/lib/components/ui/` and pull in bits-ui, tailwind-variants, tailwind-merge, clsx,
  @lucide/svelte and tw-animate-css. The CLI writes the theme tokens into `src/ui/app.css`.
- **Theme tokens**: shadcn's neutral palette for both themes, with the **accent = amber**:
  `--primary` / `--ring` / `--sidebar-primary` set to amber-500 `oklch(0.769 0.188 70.08)`,
  and `--primary-foreground` near-black `oklch(0.205 0 0)` in both themes (amber with white
  text fails contrast). Dark is defined under `.dark`; `@custom-variant dark (&:is(.dark *))`.
- **Font**: Inter Variable from `@fontsource-variable/inter` (OFL), imported in `app.css` and
  set as `--font-sans`. Vite copies its woff2 files into `dist/assets`, so it works offline.
  `docs/ui.md` says "component default font". shadcn-svelte sets no font itself (the
  Tailwind default is the system stack, which varies per machine), so we bundle Inter, the
  usual shadcn font, to get the same look on every machine.
- **Readable at 1-2 m**: root font size 112.5% (18 px), so every rem-based size scales. Nav
  items and buttons are at least 2.75 rem (about 50 px) tall.
- **Focus**: every focusable element gets a visible 3 px amber `focus-visible` ring, set in
  `app.css` `@layer base` (shadcn components already use `ring`).
- **Theme switching**: `<html class="dark">` is static in `index.html`, so the first paint is
  dark. `main.ts` awaits `getSettings()` (falling back to dark on error) and calls
  `applyTheme()`, which toggles the `dark` class on `<html>` and sets
  `document.documentElement.style.colorScheme` (CSSOM, so CSP-safe). Only then does it mount
  `App`.

### Shell and views (requirements 4 and 6)

```
┌────────────┬─────────────────────────────────────────────┐
│ [≡]        │ <h1 id="view-heading">Library</h1>           │
│ Library A+1│ summary                                      │
│ Import  A+2│ ┌ placeholder: "This view will be filled by │
│ Track   A+3│ │  gui-tracks-repository." ┘                 │
│ Playlist A4│                                              │
│ Player  A+5│    (main scrolls if content is taller)       │
│ Settings A6│                                              │
├────────────┴─────────────────────────────────────────────┤
│ Ready · Edge-AI: not configured           [v26.10.0042]  │
└──────────────────────────────────────────────────────────┘
```

- `App.svelte`: a CSS grid filling the viewport (`h-screen`, columns `auto 1fr`, rows
  `1fr auto`). `<nav>` on the left, `<main class="overflow-auto">` on the right, and the
  footer spans both columns. At 1024x640, `main` scrolls rather than overlapping anything.
- **NavBar**: one `<button>` per view in registry order, with a lucide icon (Library,
  FileInput, AudioWaveform, ListMusic, Play, Settings), the label, and a shortcut hint
  ("Alt+1"). The active entry has `aria-current="page"`, an amber left bar and an accent
  background. A collapse button at the top (`PanelLeft` icon, `aria-expanded`, shortcut
  `Ctrl+B`) shrinks the nav to icons only (labels kept as `aria-label` and `title`).
  Collapsed state lives in memory only (default expanded).
- **Shortcuts** (one `keydown` listener on `window` in `App`): `Alt+1`…`Alt+6` (by
  `event.code` `Digit1-6` / `Numpad1-6`, so they don't depend on the keyboard layout) switch to
  views in nav order, call `preventDefault`, then after `tick()` focus `#view-heading`
  (`tabindex="-1"`) so the next Tab continues inside the new view. Mouse clicks on nav
  entries switch the view and leave focus on the nav button. `Ctrl+B` toggles the nav.
  Escape closes dialogs (bits-ui).
- **Nav order (owner decision)**: Library, Import, Track, Playlists, Player, Settings
  (Alt+4 = Playlists, Alt+5 = Player). The `docs/ui.md` ASCII sketch still shows the old
  order; the owner has been told.
- **Placeholders**: `ViewPlaceholder` shows the view title, a one-line summary and
  "This view will be filled by: <feature>[, <feature>]." (Playlists: "Planned; there is no
  feature spec yet."). **TrackView** uses Tabs (Stems · Assembly · BPM & sections ·
  Tablature · MIDI cues), and each tab panel names its feature. **SettingsView** has an
  "Appearance" card first (theme RadioGroup: Dark / Light, the only working control). It is
  followed by placeholder cards "MIDI interface", "Audio output" and "Edge-AI server", each
  naming its feature; the edge-AI card has a disabled Label+Input "Server address".
- **Footer** (`StatusFooter`): "Ready" · "Edge-AI: not configured" on the left (static
  placeholders). On the right is a ghost button showing `v<version>`, with
  `aria-label="About calliope-gui"`, which opens **AboutDialog**: "calliope-gui",
  "Version <version>", "(c) 2026 Valentin Rusu", the vision sentence, and a Close button.
  If the version can't be fetched, the footer shows `v?` and the error is logged.
- `app-state.svelte.ts`: `export const ui = $state({ view: 'library' as ViewId, theme: 'dark' as Theme, version: '', navCollapsed: false })`.
  It's module-level shared state that later features extend instead of drilling props.

### IPC pattern (requirement 5) and settings persistence

Rust side, `src/ipc.rs`: all `#[tauri::command]` functions live here. They're thin wrappers
that delegate to pure modules and are registered in `gui.rs` with
`tauri::generate_handler![…]`. Names are snake_case. Fallible commands return
`Result<T, String>` (the error is a human-readable message), and every type that crosses the
boundary derives `serde::Serialize`/`Deserialize` with `rename_all = "lowercase"`/camelCase
as needed.

```rust
// src/ipc.rs
#[tauri::command] pub fn app_version() -> String;                       // crate::VERSION (same as --version)
#[tauri::command] pub fn get_settings(store: tauri::State<'_, SettingsStore>) -> Settings;
#[tauri::command] pub fn set_theme(store: tauri::State<'_, SettingsStore>, theme: Theme) -> Result<Settings, String>;
#[tauri::command] pub fn frontend_log(message: String);                 // eprintln!("calliope-ui: {}", sanitize_log(&message))
pub fn sanitize_log(msg: &str) -> String;  // control chars -> ' ', truncated to 500 chars

// src/settings.rs (pure, no Tauri)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")] pub enum Theme { #[default] Dark, Light }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)] pub struct Settings { pub theme: Theme }
pub fn load(path: &Path) -> Settings;        // missing -> default; unreadable/invalid -> default + eprintln warning
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()>; // create_dir_all(parent); write "<path>.tmp"; rename (atomic)
pub struct SettingsStore { path: PathBuf, current: Mutex<Settings> }
impl SettingsStore {
    pub fn open(path: PathBuf) -> Self;      // load()
    pub fn get(&self) -> Settings;
    pub fn set_theme(&self, theme: Theme) -> std::io::Result<Settings>; // update + save
}
```

Frontend side, `src/ui/lib/ipc.ts`: the only module that imports `@tauri-apps/api/core`.
It has one typed wrapper per command, with TS types mirroring the Rust ones:

```ts
export type Theme = 'dark' | 'light';
export interface Settings { theme: Theme }
export const appVersion = (): Promise<string> => invoke<string>('app_version');
export const getSettings = (): Promise<Settings> => invoke<Settings>('get_settings');
export const setTheme = (theme: Theme): Promise<Settings> => invoke<Settings>('set_theme', { theme });
export const frontendLog = (message: string): Promise<void> =>
  invoke<void>('frontend_log', { message }).catch(() => undefined);
```

- **Permissions**: none needed. Tauri 2.12 allows app-defined commands from the local origin
  when the app defines no app ACL manifest (checked in `tauri-2.12.1/src/webview/mod.rs`).
  The frontend calls no plugin commands (window-state works entirely on the Rust side), so
  we add **no capabilities** and the attack surface stays minimal. If a later feature
  exposes a plugin or core API to JS, it adds an inline capability in `tauri.conf.json`
  (`app.security.capabilities`), so there's no new top-level directory.
- **Theme storage**: `$XDG_CONFIG_HOME/app.calliope.gui/settings.json` (Tauri
  `app_config_dir()`, normally `~/.config/app.calliope.gui/`), e.g. `{"theme": "light"}`.
  Why Rust and not `localStorage`: later settings (MIDI device, audio output, server) are
  consumed by Rust, so they all belong in one Rust-owned file that the agents' tests can read
  and redirect with `XDG_CONFIG_HOME`. The frontend applies the theme optimistically, then
  calls `setTheme`; on error it keeps the applied theme and logs.
- **Frontend log lines** (stderr, prefix `calliope-ui: `), which make the GUI testable by the
  agents:
  - `ready view=library theme=<t> version=<v>` once after mount (after the version is fetched)
  - `view=<id>` on every view change
  - `theme=<t>` on every theme change
  - `csp-violation directive=<d> blocked=<uri>` on any CSP violation
  - `error <context>: <message>` on IPC failures

### Window (requirement 7)

- `tauri.conf.json` window: `{ "label": "main", "title": "calliope", "width": 1280, "height": 800, "minWidth": 1024, "minHeight": 640, "center": true, "visible": false }`.
- `tauri-plugin-window-state` 2.x (official, maintained), registered first in `gui.rs` with
  default flags. It restores size, position and maximised state when the window is created,
  shows the window (that's why `visible: false`: no jump from the default size to the saved
  one), tracks moves/resizes, and writes `$XDG_CONFIG_HOME/app.calliope.gui/.window-state.json`
  on `RunEvent::Exit` and close (checked in the 2.5.0 source). If the saved monitor is gone, it
  lets the WM place the window. Alternative (own code) rejected: same result, more code.
  Caveat: state is saved on a graceful exit; a SIGKILL loses that run's changes.
- On i3 (agent display), tiled windows ignore requested size/position, so the window tests
  float the window first with `i3-msg '[id=<wid>] floating enable'`.

### Dependencies added

- Cargo: `tauri-plugin-window-state = "2"`, `serde = { version = "1", features = ["derive"] }`,
  `serde_json = "1"` (moved from dev-dependencies). Dev: `tempfile = "3"`.
- npm runtime (bundled): `@tauri-apps/api` ^2.12, `@fontsource-variable/inter` ^5,
  plus what shadcn-svelte adds (bits-ui ^2.19, @internationalized/date, tailwind-variants,
  tailwind-merge, clsx, @lucide/svelte, tw-animate-css).
- npm dev: `@tauri-apps/cli` ^2.12 (only for `npm run dev:app`), `svelte` ^5.57, `@sveltejs/vite-plugin-svelte` ^7.3, `vite` ^8.3,
  `typescript` ^6.0 (not 7), `svelte-check` ^4.7, `tailwindcss` ^4.3, `@tailwindcss/vite` ^4.3,
  `vitest` ^5.0, `jsdom` ^30, `@testing-library/svelte` ^5.4.
- No ESLint/Prettier (svelte-check covers types; keeps the dependency list small).

## 3. Tasks

All commands run from `/home/vali/src/calliope`. "Suite" means `npm test` once Task 1 has
created it (before that: `cargo test && cargo clippy --all-targets -- -D warnings`).

> **Execution order (orchestrator, 2026-10-04):** Task 13 (dev mode) runs before Task 12 (visual verification), so the visual check covers the finished build. Numbering is kept as is.

### Task 1: Node/Vite/Svelte toolchain, Tauri pointed at `dist/`
- **files**: `package.json`, `package-lock.json`, `vite.config.ts`, `svelte.config.js`,
  `tsconfig.json`, `.gitignore`, `src/ui/index.html`, `src/ui/main.ts`, `src/ui/App.svelte`,
  `src/ui/app.css`, `src/ui/vite-env.d.ts`, `tauri.conf.json`, `tests/frontend.rs`,
  `tests/acceptance_gui_skeleton.rs`
- **does**: Create the npm project (`"private": true`, `"type": "module"`,
  `"engines": {"node": ">=22.12"}`) with the dev dependencies and versions from "Dependencies
  added" (not the shadcn ones yet), plus `@tauri-apps/api`, and every script from the
  "Build and run" table. Write `vite.config.ts` / `svelte.config.js` / `tsconfig.json` as
  described in "Frontend stack" (`$lib` alias in both Vite and tsconfig `paths`, strict TS,
  vitest jsdom config, `passWithNoTests: true` until tests exist). Write `index.html`
  (`<html lang="en" class="dark">`, `<title>calliope</title>`, `<div id="app"></div>`,
  `<script type="module" src="/main.ts"></script>`, no inline code), `app.css` (`@import "tailwindcss";`),
  `main.ts` (mount App into `#app`), and `App.svelte` (a temporary `<h1>calliope</h1>`).
  Add `dist/` to `.gitignore`. In `tauri.conf.json` set `build.frontendDist` to `"dist"`
  (nothing else changes yet). Update the static tests: `frontendDist == "dist"`; the
  `index_html_has_no_inline_code` test now allows `<script` only with a `src=` attribute and
  still forbids `<style` and `style=`; `ac2_static_gui_content_and_wiring` drops the
  "hello from calliope" check and expects `"dist"`.
- **done when**: `npm ci && npm run build` creates `dist/index.html` plus hashed JS/CSS in
  `dist/assets/`; `npm run check` passes; `npm run test:ui` passes; `cargo test` and
  `cargo clippy --all-targets -- -D warnings` pass; `grep -c '<script' dist/index.html`
  shows only `src=` script tags.
- **test**: `npm ci && npm run check && npm run build && cargo test && cargo clippy --all-targets -- -D warnings`
- **routine: no** (many interlocking config files and version resolution)

### Task 2: shadcn-svelte components, theme tokens, bundled font
- **files**: `components.json`, `package.json`, `package-lock.json`, `src/ui/app.css`,
  `src/ui/lib/utils.ts`, `src/ui/lib/components/ui/**` (generated)
- **does**: `npx shadcn-svelte@1.7.0 init --base-color neutral --css src/ui/app.css --lib-alias '$lib' --components-alias '$lib/components' --utils-alias '$lib/utils' --hooks-alias '$lib/hooks' --ui-alias '$lib/components/ui'`
  (fix `components.json` by hand if the CLI insists on SvelteKit paths), then
  `npx shadcn-svelte@1.7.0 add button input label dialog tabs radio-group separator card -y`.
  Install `@fontsource-variable/inter`. In `app.css`: import the font, set
  `--font-sans: 'Inter Variable', ui-sans-serif, system-ui, sans-serif`, apply the amber
  tokens from "Frontend stack" in `:root` and `.dark`, set `html { font-size: 112.5%; }`,
  and give all focusable elements a 3 px `focus-visible` ring in the `ring` colour. Make
  `App.svelte` render one `Button` and one `Input` so the build includes them. Don't add
  mode-watcher or sonner.
- **done when**: `npm run check` and `npm run build` pass; `dist/assets/` has at least one
  `.woff2`; `grep -L 'style="' src/ui/lib/components/ui -r` lists every component file (no
  static style attributes); no CSS file in `dist/assets` has a remote reference or `url(data:`
  (checked after stripping `/* ... */` comments, which may hold licence URLs; remote means
  `url(` or `@import` followed by an optional quote and `http(s)://` or `//`).
- **test**: `npm run check && npm run build && ls dist/assets/*.woff2 && python3 -c "import re,glob,sys; bad=[f for f in glob.glob('dist/assets/*.css') if re.search(r'''(url\(|@import\s*(url\()?)\s*[\"']?(https?:)?//|url\(\s*[\"']?data:''', re.sub(r'/\*.*?\*/', '', open(f).read(), flags=re.S))]; print(bad); sys.exit(bool(bad))"`
- **routine: no** (interactive CLI, theme judgement)

### Task 3: build.rs frontend check, window config, static CSP/asset tests
- **files**: `build.rs`, `tauri.conf.json`, `tests/frontend.rs`
- **does**: In `build.rs`, before the version logic, add `check_frontend()` as described in
  "Build and run" (exact error text from there; it uses the existing `fail()`; the stale
  check compares the newest mtime under `src/ui/` with `dist/index.html` and emits
  `cargo:warning=`; it prints `rerun-if-changed` for `src/ui` and `dist/index.html`). Set the
  window config to the JSON in "Window" (without the plugin yet; `visible: false` waits for
  Task 5, so **keep `visible` true/absent here**). Extend `tests/frontend.rs`:
  window has `minWidth` 1024, `minHeight` 640, title `calliope`; no `devUrl` and no
  `devCsp`; CSP test unchanged; new `no_static_style_attributes_in_svelte` (walks
  `src/ui/**/*.svelte`, fails on `style="` or `style='`); new `dist_has_no_inline_code`
  (`dist/index.html`: every `<script` has `src=`, no `<style`, no `style=`); new
  `dist_assets_are_local` (no `http://`/`https://` in `dist/index.html`; in `dist/**/*.css`, after stripping
  `/* */` comments, no `url(`/`@import` pointing at `http(s)://` or `//` and no `url(data:`, at least one `.woff2` under `dist/assets`).
- **done when**: Suite passes. `mv dist /tmp/claude-…/scratchpad/dist.bak && cargo build`
  fails and prints the exact "frontend is not built" message (then restore `dist`);
  `touch src/ui/App.svelte && cargo build` prints the stale warning.
- **test**: `npm test`, plus the two manual cargo checks above (record their output in the log)
- **routine: no** (build script)

### Task 4: Rust settings module
- **files**: `src/settings.rs`, `src/main.rs` (add `mod settings;`), `Cargo.toml`
- **does**: Implement `src/settings.rs` exactly as in "IPC pattern" (`Theme`, `Settings`,
  `load`, `save`, `SettingsStore`). Add the Cargo deps `serde` (derive) and `serde_json`
  (move from dev-deps), and the dev-dep `tempfile`. Until Task 5 uses it, add
  `#[allow(dead_code)]` on the `mod` line. Unit tests: default is dark; JSON is
  `{"theme":"light"}` ↔ `Settings{theme: Light}`; unknown fields are ignored and missing
  fields default; `load` on a missing file gives the default; `load` on garbage gives the
  default; `save` then `load` round-trips and creates parent dirs; no `.tmp` file is left;
  `SettingsStore::set_theme` persists (re-open sees `Light`).
- **done when**: `cargo test settings::` runs at least 7 tests, all passing; suite passes.
- **test**: `npm test`
- **routine: no** (Rust; the local model failed comparable Rust tasks)

### Task 5: IPC commands, window-state plugin, Tauri wiring
- **files**: `src/ipc.rs`, `src/gui.rs`, `src/main.rs`, `Cargo.toml`, `tauri.conf.json`
- **does**: Add `tauri-plugin-window-state = "2"`. Write `src/ipc.rs` as in "IPC pattern".
  `gui.rs`: register the window-state plugin first; in `.setup()` build
  `app.path().app_config_dir()?.join("settings.json")` and `app.manage(SettingsStore::open(path))`;
  `.invoke_handler(tauri::generate_handler![ipc::app_version, ipc::get_settings, ipc::set_theme, ipc::frontend_log])`.
  Set `"visible": false` on the window (the plugin shows it). Remove the `dead_code` allow
  from Task 4. Unit tests in `ipc.rs`: `app_version()` equals `env!("CALLIOPE_VERSION")`
  and matches `NN.NN.NNNN`; `sanitize_log` replaces `\n`/`\r`/`\t`/other control chars and
  truncates to 500 chars. Extend `tests/frontend.rs`: the window has `"visible": false`.
- **done when**: Suite passes. On `:1`:
  `XDG_CONFIG_HOME=<scratch>/cfg DISPLAY=:1 target/debug/calliope-gui` opens a window
  (`gui-shot <scratch>/t5.png '^calliope$'` succeeds); after closing it with
  `i3-msg '[title="^calliope$"] kill'`, `<scratch>/cfg/app.calliope.gui/.window-state.json` exists.
- **test**: `npm test` plus the `:1` check above
- **routine: no** (Tauri wiring, state, plugin)

### Task 6: View registry and shortcut mapping (frontend, pure)
- **files**: `src/ui/lib/views.ts`, `src/ui/lib/views.test.ts`
- **does**: Write `views.ts` with exactly this content (and nothing else):
  ```ts
  export type ViewId = 'library' | 'import' | 'track' | 'player' | 'playlists' | 'settings';

  export interface ViewInfo {
    id: ViewId;
    label: string;
    digit: number;
    summary: string;
    features: string[];
  }

  export const VIEWS: readonly ViewInfo[] = [
    { id: 'library', label: 'Library', digit: 1, summary: 'The backing-track repository.', features: ['gui-tracks-repository'] },
    { id: 'import', label: 'Import', digit: 2, summary: 'Import a music track for stem extraction, or an existing backing track.', features: ['gui-stem-extracting', 'gui-existing-track-import'] },
    { id: 'track', label: 'Track', digit: 3, summary: 'One track: stems, assembly, BPM and sections, tablature, MIDI cues.', features: ['gui-backing-track-assembly', 'gui-tablatures', 'gui-manipulate-backing-track'] },
    { id: 'playlists', label: 'Playlists', digit: 4, summary: 'Backing-track playlists.', features: [] },
    { id: 'player', label: 'Player', digit: 5, summary: 'Playback with the tablature view.', features: ['gui-play-backing-track'] },
    { id: 'settings', label: 'Settings', digit: 6, summary: 'MIDI interface, audio output, edge-AI server and theme.', features: ['gui-play-backing-track', 'gui-stem-extracting'] },
  ];

  export interface TrackTab { id: string; label: string; feature: string }
  export const TRACK_TABS: readonly TrackTab[] = [
    { id: 'stems', label: 'Stems', feature: 'gui-backing-track-assembly' },
    { id: 'assembly', label: 'Assembly', feature: 'gui-backing-track-assembly' },
    { id: 'tempo', label: 'BPM & sections', feature: 'gui-manipulate-backing-track' },
    { id: 'tablature', label: 'Tablature', feature: 'gui-tablatures' },
    { id: 'cues', label: 'MIDI cues', feature: 'gui-manipulate-backing-track' },
  ];

  export interface SettingsSection { id: string; title: string; text: string; feature: string }
  export const SETTINGS_PLACEHOLDERS: readonly SettingsSection[] = [
    { id: 'midi', title: 'MIDI interface', text: 'Choose the USB-MIDI interface that sends clock and patch changes.', feature: 'gui-play-backing-track' },
    { id: 'audio', title: 'Audio output', text: 'Choose where the audio goes.', feature: 'gui-play-backing-track' },
    { id: 'edge-ai', title: 'Edge-AI server', text: 'The ollama server on the LAN used for stem extraction.', feature: 'gui-stem-extracting' },
  ];

  export function featureText(features: readonly string[]): string {
    return features.length === 0
      ? 'Planned; there is no feature spec yet.'
      : `This view will be filled by: ${features.join(', ')}.`;
  }

  export function shortcutLabel(view: ViewInfo): string {
    return `Alt+${view.digit}`;
  }

  export interface KeyLike {
    code: string;
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
  }

  export function viewForKey(e: KeyLike): ViewId | null {
    if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return null;
    const m = /^(?:Digit|Numpad)([1-6])$/.exec(e.code);
    return m ? VIEWS[Number(m[1]) - 1].id : null;
  }

  export function isNavToggleKey(e: KeyLike): boolean {
    return e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.code === 'KeyB';
  }
  ```
  `views.test.ts` (vitest, `import { describe, it, expect } from 'vitest'`) checks: the labels
  in order are `['Library','Import','Track','Playlists','Player','Settings']`; digits are
  1..6; every view except `playlists` has at least one feature; `featureText([])` and
  `featureText(['a','b'])` give the exact strings above; `viewForKey` maps
  `{code:'Digit1', altKey:true, others false}` → `'library'`, `Digit6` → `'settings'`,
  `Numpad3` → `'track'`, `Digit4` → `'playlists'`, `Digit5` → `'player'`; it returns `null` for `Digit7`, `Digit0`, `KeyA`, and for `Digit1`
  with `ctrlKey`, `shiftKey` or `metaKey` also set, or without `altKey`; `isNavToggleKey` is
  true for `{code:'KeyB', ctrlKey:true}` and false with `altKey` added or for `KeyC`;
  `TRACK_TABS` has 5 entries; `SETTINGS_PLACEHOLDERS` has 3.
- **done when**: `npx vitest run src/ui/lib/views.test.ts` passes (≥ 8 tests); `npm run check` passes.
- **test**: `npm run check && npx vitest run src/ui/lib/views.test.ts`
- **routine: yes** (two small files, content given)

### Task 7: IPC wrappers, theme module, shared state
- **files**: `src/ui/lib/ipc.ts`, `src/ui/lib/ipc.test.ts`, `src/ui/lib/theme.ts`,
  `src/ui/lib/theme.test.ts`, `src/ui/lib/app-state.svelte.ts`
- **does**: `ipc.ts` exactly as in "IPC pattern". `theme.ts`:
  `export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void`
  (toggles the `dark` class, sets `root.style.colorScheme`). `app-state.svelte.ts` as in
  "Shell and views". Tests: `ipc.test.ts` uses `mockIPC` / `clearMocks` from
  `@tauri-apps/api/mocks` to check the command names and argument objects (`set_theme` gets
  `{ theme: 'light' }`, `frontend_log` gets `{ message }`), and that `frontendLog` swallows
  a rejected invoke. `theme.test.ts` checks that the class is added/removed and the colour
  scheme is set.
- **done when**: `npm run test:ui` and `npm run check` pass.
- **test**: `npm run check && npm run test:ui`
- **routine: no** (mockIPC in jsdom is easy to get subtly wrong)

### Task 8: Placeholder component and the four simple views
- **files**: `src/ui/components/ViewPlaceholder.svelte`, `src/ui/views/LibraryView.svelte`,
  `src/ui/views/ImportView.svelte`, `src/ui/views/PlayerView.svelte`,
  `src/ui/views/PlaylistsView.svelte`
- **does**: `ViewPlaceholder.svelte` with exactly:
  ```svelte
  <script lang="ts">
    import { featureText, type ViewInfo } from '$lib/views';
    let { view }: { view: ViewInfo } = $props();
  </script>

  <section class="flex flex-col gap-4 p-8" aria-labelledby="view-heading">
    <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{view.label}</h1>
    <p class="text-lg text-muted-foreground">{view.summary}</p>
    <p class="rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
      {featureText(view.features)}
    </p>
  </section>
  ```
  Each simple view (example for Library; Import/Player/Playlists use ids `import`,
  `player`, `playlists`):
  ```svelte
  <script lang="ts">
    import ViewPlaceholder from '../components/ViewPlaceholder.svelte';
    import { VIEWS } from '$lib/views';
    const view = VIEWS.find((v) => v.id === 'library')!;
  </script>

  <ViewPlaceholder {view} />
  ```
- **done when**: `npm run check` passes with no errors or warnings; `npm run build` passes;
  `grep -r 'style=' src/ui/components src/ui/views` finds nothing.
- **test**: `npm run check && npm run build`
- **routine: yes** (five tiny files, content given)

### Task 9: Application shell (nav, footer, About, Track/Settings views, bootstrap)
- **files**: `src/ui/App.svelte`, `src/ui/main.ts`, `src/ui/components/NavBar.svelte`,
  `src/ui/components/StatusFooter.svelte`, `src/ui/components/AboutDialog.svelte`,
  `src/ui/views/TrackView.svelte`, `src/ui/views/SettingsView.svelte`, `src/ui/App.test.ts`
- **does**: Implement "Shell and views", "Theme switching" and the log lines from "IPC
  pattern". `main.ts`: register the `securitypolicyviolation` listener first, then
  `getSettings()` → `applyTheme` → set `ui.theme` → mount `App`. `App` on mount: fetch the
  version, then `frontendLog('ready view=… theme=… version=…')`; a `$effect` logs
  `view=<id>` on view changes (not on the initial one). SettingsView: Appearance card first,
  with a RadioGroup (`Dark`/`Light`, labelled "Theme"); a change calls `applyTheme`, sets
  `ui.theme`, calls `setTheme` and logs `theme=<t>`. Every view's `<h1>` has
  `id="view-heading" tabindex="-1"`. Remove the temporary Button/Input from Task 2.
  `App.test.ts` (testing-library + mockIPC, with `app_version` returning `26.10.0042` and
  `get_settings` returning dark):
  1. the nav shows six buttons in order;
  2. clicking each switches the view, and the placeholder text names its features;
  3. dispatching `keydown` Alt+Digit1..6 on `window` switches the view and focuses `#view-heading`;
  4. the footer shows `v26.10.0042`; clicking it opens a dialog containing `26.10.0042`, and Escape closes it;
  5. choosing Light in Settings removes `dark` from `<html>` and invokes `set_theme` with `{theme:'light'}`;
  6. Ctrl+B collapses the nav (`aria-expanded="false"`, labels still exposed as `aria-label`);
  7. the active entry has `aria-current="page"`.
- **done when**: Suite passes (`npm test`), including the 7 behaviours above; `npm run build`
  output has no inline code (the Task 3 tests).
- **test**: `npm test`
- **routine: no** (cross-cutting UI, focus handling, state)

### Task 10: GUI end-to-end tests on a display
- **files**: `tests/gui_e2e.rs` (new), `tests/gui_smoke.rs`
- **does**: All tests are `#[ignore]` and return early (printing "skipping") without
  `DISPLAY`. Shared helpers: a struct that spawns `CARGO_BIN_EXE_calliope-gui` with
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` pointing at a fresh dir under
  `target/gui-e2e/<test>/` (never the user's real config), collects stderr lines in a
  background thread, and **kills the child on Drop**; `wait_line(pred, timeout)`; `wid()` via
  `xdotool search --sync --onlyvisible --name '^calliope$'`; `key(wid, "alt+3")` via
  `xdotool windowactivate --sync <wid> key --clearmodifiers …`; `shot(name)` via
  `gui-shot target/gui-shots/<name>.png '^calliope$' 10` (skipped if `gui-shot` is
  missing); `close_gracefully(wid)` via `i3-msg '[id=<wid>] kill'` then waiting ≤ 5 s for exit.
  Tests:
  1. `starts_dark_with_version`: a `ready` line with `theme=dark`, and `version=` equal to
     the `calliope-gui --version` output; shot `start-dark`.
  2. `shortcuts_switch_views`: Alt+1..6 each produce `view=<id>` (in order; Alt+1 is sent after
     Alt+2 so that a change is logged); shot `view-<id>` for each.
  3. `theme_switch_persists`: Alt+6, Tab, Right produce `theme=light`;
     `settings.json` contains `"light"`; shot `settings-light`; restart with the same XDG
     dirs gives a `ready … theme=light` line; shot `restart-light`.
  4. `window_state_and_min_size` (skip if `i3-msg` is missing): float the window;
     `xdotool windowsize <wid> 800 500`, then the geometry (`xdotool getwindowgeometry`) is
     ≥ 1024x640; `windowsize 1024 640`, then for each view Alt+n and shot `min-<id>`;
     `windowsize 1180 720` + `windowmove 150 120`; close gracefully;
     `.window-state.json` has `main` width 1180 ±2 and height 720 ±2; relaunch, float, and the
     size is 1180x720 ±2; the position is printed and compared within ±60 px (border offsets).
     If i3 doesn't apply the restored geometry when floating after a relaunch, keep the
     state-file assertion, downgrade the relaunch assertion to a printed note, and record
     that in the log (the owner checks it manually).
  5. `offline_start` (skip if `unshare -rn true` fails): run the binary under
     `unshare -rn` (no network), require a `ready` line; shot `offline`.
  Every test fails if any stderr line contains `csp-violation`.
  `gui_smoke.rs`: use the same temp-XDG + kill-on-drop approach (fixes the reviewer's
  gui-skeleton follow-up).
- **done when**: `DISPLAY=:1 npm run test:gui` passes; `target/gui-shots/` contains the
  screenshots listed above.
- **test**: `DISPLAY=:1 npm run test:gui`
- **routine: no** (process control, X automation, timing)

### Task 11: README and docs
- **files**: `README.md`
- **does**: Rewrite these sections, keeping "Version scheme" as is.
  **Prerequisites**: the Rust toolchain, `webkit2gtk-4.1 gtk3 base-devel`, Node.js ≥ 22.12
  with npm, and "run `npm ci` once after cloning and after `package-lock.json` changes".
  **Build**: `npm run build:app` → `target/release/calliope-gui`, plus the note that plain
  `cargo build` works once `dist/` exists and otherwise stops with a message.
  **Run**: `npm run app` (debug), or the release binary; plus the `--help`/`--version` lines.
  **Tests**: `npm test` (whole headless suite) and `DISPLAY=:1 npm run test:gui` (needs an X
  display; screenshots go to `target/gui-shots/`).
  **Dev mode**: `npm run dev:app` (Vite dev server with hot reload; uses a dev-only CSP from
  `tauri.dev.conf.json`; release builds never use it).
  **Keyboard**: Alt+1…Alt+6 = Library, Import, Track, Playlists, Player, Settings;
  Ctrl+B = collapse navigation.
  **Settings files**: `~/.config/app.calliope.gui/settings.json` and `.window-state.json`.
  **Project layout**: the tree from the plan's "Layout" section. Use ASCII only.
- **done when**: `grep -q 'npm run build:app' README.md && grep -q 'npm run app' README.md && grep -q 'npm ci' README.md && grep -q 'npm test' README.md && grep -q 'npm run dev:app' README.md && ! grep -qP '[^\x00-\x7F]' README.md`
- **test**: the grep line above
- **routine: yes** (one docs file, content listed)

### Task 12: Visual verification and fresh-clone build on `:1`
- **files**: `specs/gui-frontend-foundation.log.md` (results only; no code)
- **does**: (a) Fresh clone: `git clone /home/vali/src/calliope <scratch>/fresh && cd <scratch>/fresh && npm ci && npm run build:app`,
  then `target/release/calliope-gui --version` and a launch on `:1` with temp XDG dirs, and
  a screenshot. (b) Look at every PNG from Task 10 and check against "Visual checklist"
  (Test strategy). (c) Mouse: from the `start-dark` screenshot, read the nav entry
  coordinates, `xdotool mousemove --window <wid> X Y click 1` on each entry, and confirm the
  `view=<id>` lines and screenshots; click the footer version and screenshot the About
  dialog; press Tab a few times and screenshot to confirm the focus ring is visible.
  (d) Maximised: `i3-msg fullscreen` is not maximise; instead tile the window alone on the
  1920x1200 workspace (the default on `:1`) and take a screenshot: no stretched or awkward
  layout. Record each check as PASS/FAIL with the screenshot path. Fix nothing here;
  failures go back to the task that owns them.
- **done when**: every item in the visual checklist is recorded as PASS in the log.
- **test**: manual agent inspection (screenshots)
- **routine: no** (visual judgement; the local model can't see screenshots)

### Task 13: Dev mode with hot reload (owner decision)
- **files**: `package.json`, `package-lock.json`, `tauri.dev.conf.json`, `.taurignore`,
  `build.rs`, `tests/frontend.rs`
- **does**: Add the `@tauri-apps/cli` ^2.12 dev-dependency and the `dev:app` script (plus
  `dev:ui` and the Vite `server` block if Task 1 didn't add them). Create
  `tauri.dev.conf.json` and `.taurignore` exactly as in "Dev mode with hot reload". In
  `build.rs`, read `TAURI_CONFIG`: in a release `PROFILE`, fail if it mentions `devUrl` or
  `devCsp` (exact message in the plan); in debug, skip the `dist/` missing/stale checks when
  it sets a `devUrl`. Static tests in `tests/frontend.rs`:
  - `production_csp_has_no_dev_relaxations`: `app.security.csp` in `tauri.conf.json` has no
    `unsafe-inline`, `unsafe-eval`, `ws:` or `localhost:5173`, and `tauri.conf.json` has no
    `devUrl`, `devCsp` or `beforeDevCommand` anywhere;
  - `dev_csp_is_minimal_and_separate`: `tauri.dev.conf.json` has `build.devUrl ==
    "http://localhost:5173"`, has no `app.security.csp` key, and parsing both CSPs into
    directive→token sets shows the devCsp equals the production CSP except for `'unsafe-inline'`
    added to `style-src` and `ws://localhost:5173` added to `connect-src`; no `unsafe-eval`.
  Verify that tauri-cli finds `tauri.conf.json` at the repo root; if it can't, stop and
  report rather than moving files. If WebKit reports `csp-violation` lines in dev mode that
  need more than the two allowed tokens, **stop and ask the owner** instead of widening the devCsp.
- **done when**: Suite passes. `TAURI_CONFIG="$(cat tauri.dev.conf.json)" cargo build --release`
  fails with the release-guard message (record the output). On `:1`:
  `DISPLAY=:1 XDG_CONFIG_HOME=<scratch>/cfg npm run dev:app` (in the background, output to a
  file) opens the window; a temporary text change in `src/ui/views/LibraryView.svelte`
  appears in a `gui-shot` screenshot without restarting (revert it afterwards); the output
  has no `csp-violation` lines. Stop the dev processes afterwards. Then `npm run app` still
  rebuilds and shows the embedded UI (screenshot).
- **test**: `npm test` plus the checks above
- **routine: no** (build guard, security config)

## 4. Test strategy

- **Rust unit tests** (in their modules): `settings` (serde, load/save, store), `ipc`
  (`app_version` = `CALLIOPE_VERSION`, `sanitize_log`), plus the existing `cli` and `version` tests.
- **Rust integration tests (headless)**: `tests/cli.rs` and `tests/acceptance_gui_skeleton.rs`
  (`--help`/`--version`, unchanged apart from the static-page check), and `tests/frontend.rs`
  (strict production CSP with no `'unsafe-inline'`/`'unsafe-eval'`/`ws:`, no devCsp/devUrl in `tauri.conf.json`, devCsp in `tauri.dev.conf.json` = production CSP plus only the two allowed tokens, window config, no inline code in `src/ui/index.html` and
  `dist/index.html`, no static `style=` in `.svelte` files, assets local: woff2 present, no
  `http(s)://` or `data:` URLs in the CSS).
- **Frontend unit/component tests** (vitest + jsdom + @testing-library/svelte, IPC faked
  with `@tauri-apps/api/mocks` `mockIPC`): view registry and shortcuts, IPC wrappers,
  theme application, and shell behaviour (navigation by click and shortcut, focus, About
  dialog, theme switch, nav collapse).
- **Type check**: `svelte-check --fail-on-warnings`.
- **GUI end-to-end on display `:1`** (`tests/gui_e2e.rs`, `#[ignore]`): the real binary with
  temporary XDG dirs, driven with `xdotool`/`i3-msg`, asserting on the `calliope-ui:` stderr
  lines (ready/version/theme/view/csp-violation) and on files (`settings.json`,
  `.window-state.json`), and taking screenshots with `gui-shot` into `target/gui-shots/`.
  "Offline" is a launch inside `unshare -rn` (a network namespace with no network), and the
  CSP (`default-src 'self'`) blocks remote loads anyway.
- **Visual checklist** (Task 12, an agent looking at the screenshots):
  1. dark background, light text, amber accent on the active nav entry;
  2. nav shows Library, Import, Track, Playlists, Player, Settings in this order, with Alt+n hints;
  3. each view shows its title and the placeholder naming its feature(s); the Track tabs and the Settings cards are visible;
  4. footer shows "Ready", "Edge-AI: not configured" and `v<same as --version>`;
  5. light theme: the whole UI (nav, content, footer, cards) is light, with no dark leftovers;
  6. at 1024x640: no overlapping, clipped or cut-off text in any view (the content area may scroll);
  7. focus ring clearly visible on nav buttons and on the theme radio;
  8. About dialog is centred and readable, showing the version;
  9. the offline screenshot looks the same as `start-dark` (same font, no missing glyphs or icons);
  10. the text looks Inter-like (not a fallback serif), and text sizes are large enough to read at a distance.
- **Whole headless suite**: `npm test`
  (= `npm run check && npm run test:ui && npm run build && cargo test && cargo clippy --all-targets -- -D warnings`).
- **GUI suite** (agents, display `:1` only): `DISPLAY=:1 npm run test:gui`.
- No hardware or network fakes are needed: this feature touches neither MIDI/audio nor the
  edge-AI server.

## 5. Deployment

Single machine (the laptop); built and run locally. Not applicable.

## 6. Manual checks (owner, on your own desktop session)

1. Fresh clone: `git clone … && cd calliope && npm ci && npm run build:app`. Expect
   `target/release/calliope-gui` to exist.
2. Run `target/release/calliope-gui`. The window opens dark, with an amber highlight on
   "Library", and the nav lists Library, Import, Track, Playlists, Player, Settings.
3. Press Alt+1 … Alt+6 (Alt+4 = Playlists, Alt+5 = Player), then click each nav entry.
   Each view names its future feature.
   Ctrl+B collapses and expands the nav.
4. The footer shows `v…`, the same as `calliope-gui --version`. Click it: an About dialog
   shows the same version.
5. Settings → Theme → Light: the whole UI turns light. Close the app and start it again: it
   is still light. Switch back to Dark.
6. Resize and move the window, close it normally (window close button or WM close, not
   `kill -9`), and start it again. Same size and position. (On i3 this needs a floating
   window.)
7. Shrink the window as far as it goes: it stops at 1024x640, and no view has overlapping or
   cut-off text.
8. Maximise on the 1920x1200 screen: the layout looks right.
9. Turn off Wi-Fi/network and start the app: it looks the same (font, icons).
10. From 1-2 m away, in a dim room: the nav labels, the placeholder text and the focus ring
    are readable and visible.
11. `calliope-gui --help` and `--version` still print as before, without opening a window.
12. `npm run dev:app`: the window opens. Change a text in `src/ui/views/LibraryView.svelte`
    and it updates in the window without a restart. Revert the change.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests / checks |
|---|---|---|
| Fresh clone + documented build command → `calliope-gui` opening the new UI | 1, 3, 11, 12 | Task 12 (a) fresh-clone build + screenshot; `build.rs` missing-dist message (Task 3); `ac1_binary_exists…`; manual 1-2 |
| Dark theme active on start; nav shows the six views | 2, 9 | `gui_e2e::starts_dark_with_version` (`theme=dark`), `App.test.ts` (1), `views.test.ts` order; visual 1-2 |
| Each nav entry by mouse and shortcut shows its placeholder naming the feature | 6, 8, 9, 10, 12 | `views.test.ts` (order, Alt+4 = Playlists, Alt+5 = Player); `App.test.ts` (2, 3); `gui_e2e::shortcuts_switch_views`; Task 12 (c) real mouse clicks; visual 3 |
| About/footer shows the same version as `--version` | 5, 7, 9 | `ipc::tests` (`app_version` = `CALLIOPE_VERSION`); `App.test.ts` (4); `gui_e2e::starts_dark_with_version` compares with `--version`; visual 4, 8 |
| Theme → light changes the whole UI and survives a restart | 4, 5, 7, 9 | `settings::tests`; `theme.test.ts`; `App.test.ts` (5); `gui_e2e::theme_switch_persists`; visual 5 |
| Window size and position restored after restart | 5, 10 | `gui_e2e::window_state_and_min_size` (state file + relaunch size); manual 6 |
| At minimum size nothing overlaps or is cut off (screenshots) | 3, 9, 10, 12 | `frontend.rs` minWidth/minHeight; `gui_e2e::window_state_and_min_size` min clamp + `min-<id>` shots; visual 6 |
| Network unavailable → looks the same | 2, 3, 10, 12 | `frontend.rs::dist_assets_are_local`; strict CSP; `gui_e2e::offline_start`; visual 9 |
| Existing `--help`/`--version` and CSP tests still pass | 1, 3 | `tests/cli.rs`, `tests/acceptance_gui_skeleton.rs`, `frontend.rs::tauri_conf_has_strict_csp` in `npm test` |
| `docs/architecture.md` describes stack, build commands, IPC pattern, corrected GUI testing | architect (done with this plan) | review of `docs/architecture.md` |
| Req 1 (TS, framework, build step, source in `src/`) | 1 | `npm run check`; `tests/acceptance_gui_skeleton.rs::req2_source_lives_in_src` |
| Req 6 (focus visible, every view reachable by keyboard) | 2, 9, 12 | `App.test.ts` (3); visual 7 |
| Req 8 (CSP stays strict) | 3, 9, 10, 13 | `frontend.rs` CSP / no-devCsp-in-tauri.conf / no-inline / no-static-style tests; `frontend.rs::dev_csp_is_minimal_and_separate`; `build.rs` release guard (Task 13 check); `csp-violation` check in every e2e test |
| Owner decision: dev mode with hot reload, dev-only CSP | 13 | `frontend.rs::dev_csp_is_minimal_and_separate`, `production_csp_has_no_dev_relaxations`; Task 13 `:1` hot-reload check; manual 12 |

## 8. Assumptions (defaults chosen; recorded in `docs/ui.md` "Decided by the team" where UI-related)

- Nav and shortcut order (owner decision): Library, Import, Track, Playlists, Player, Settings.
- Font: Inter Variable, bundled. Root font size 18 px. Accent amber-500 with dark text on it.
- Nav collapse: `Ctrl+B`, icons only when collapsed, not persisted.
- Shortcuts switch the view and move focus to the view heading; mouse clicks keep focus on the nav.
- Footer: "Ready" · "Edge-AI: not configured" (static placeholders) · `v<version>` button
  that opens About.
- Settings: Appearance (theme) first, then the MIDI interface, Audio output and Edge-AI server placeholders.
- Playlists has no roadmap feature: its placeholder says "Planned; there is no feature spec yet."
- Track view tabs map to features as in `TRACK_TABS`.
- Default window 1280x800, centred on first run.
- Theme and window state live in `~/.config/app.calliope.gui/` (`settings.json`,
  `.window-state.json`), owned by Rust. No `localStorage`.
- Dev mode (owner decision): `npm run dev:app` via `@tauri-apps/cli` (dev-dependency) with a
  dev-only `devCsp` in `tauri.dev.conf.json`. Release builds: plain cargo, with `build.rs`
  guarding against a missing frontend build and against dev config in release builds.
- `npm ci` once after clone is a documented prerequisite, not part of the build command.

## 9. Open questions

None. Both earlier questions were answered by the owner (2026-10-04):
1. Dev-only `devCsp` for the Vite dev server with hot reload: **allowed**, kept minimal (see
   "Dev mode with hot reload", Task 13).
2. Nav order: **Library, Import, Track, Playlists, Player, Settings** (Alt+4 = Playlists,
   Alt+5 = Player).
