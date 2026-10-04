# calliope-gui

Rust desktop application with a Tauri 2 GUI and a Svelte 5 + Tailwind + shadcn-svelte frontend (built with Vite).
Run with no arguments it opens the application window (navigation, Track and Settings views, About dialog).

## Prerequisites (Arch Linux)

* Rust toolchain (cargo)
* Packages: webkit2gtk-4.1, gtk3, base-devel
* Node.js >= 22.12 with npm
* Optional: xorg-server-xvfb for headless GUI runs

Run `npm ci` once after cloning and after `package-lock.json` changes.

## Build

```bash
npm run build:app
```

Produces `target/release/calliope-gui`.

Plain `cargo build` also works once `dist/` exists (built by `npm run build`). Without it, the build stops with a message saying the frontend is not built.

## Run

```bash
npm run app                      # debug build, opens the window
target/release/calliope-gui      # release binary
calliope-gui --help
calliope-gui --version
```

Unknown options print an error plus help to `stderr` and exit with code 2.

## Dev mode

```bash
npm run dev:app
```

Starts the Vite dev server with hot reload plus the debug app. It uses a dev-only CSP from `tauri.dev.conf.json`; release builds never use it.

## Keyboard

* Alt+1 ... Alt+6: Library, Import, Track, Playlists, Player, Settings
* Ctrl+B: collapse navigation

## Settings files

* `~/.config/app.calliope.gui/settings.json`
* `~/.config/app.calliope.gui/.window-state.json`

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
npm test                         # whole headless suite: svelte-check, vitest, vite build, cargo test, clippy
DISPLAY=:1 npm run test:gui      # GUI tests, need an X display
```

GUI test screenshots go to `target/gui-shots/`.

## Project layout

```
package.json, package-lock.json    npm project at the repo root
vite.config.ts                     root: src/ui, outDir: dist/, $lib alias, vitest config
svelte.config.js                   vitePreprocess
tsconfig.json                      strict TS, paths $lib -> src/ui/lib
components.json                    shadcn-svelte CLI config
dist/                              build output of `vite build` (gitignored), embedded by Tauri
Cargo.toml, build.rs, tauri.conf.json, icons/
tauri.dev.conf.json                dev-only config (used only by npm run dev:app)
.taurignore                        keeps tauri dev from rebuilding Rust on frontend edits
src/*.rs                           Rust: main, cli, version, gui, ipc, settings
src/ui/index.html                  Vite entry HTML
src/ui/main.ts                     bootstrap: CSP listener, load theme, mount App
src/ui/app.css                     Tailwind v4 + tw-animate-css + Inter font + theme tokens
src/ui/App.svelte                  shell: NavBar | view | StatusFooter, global shortcuts
src/ui/components/                 NavBar, StatusFooter, AboutDialog, ViewPlaceholder
src/ui/views/                      LibraryView ... SettingsView (one file per view)
src/ui/lib/views.ts                view registry + shortcut mapping
src/ui/lib/ipc.ts                  the only module importing @tauri-apps/api
src/ui/lib/theme.ts                applyTheme()
src/ui/lib/app-state.svelte.ts     shared rune state
src/ui/lib/utils.ts                shadcn cn() helper
src/ui/lib/components/ui/          shadcn-svelte components
src/ui/**/*.test.ts                vitest tests next to the code
tests/                             cargo tests (cli, frontend, acceptance, gui_*)
```
