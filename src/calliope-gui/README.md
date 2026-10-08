# calliope-gui

Rust desktop application with a Tauri 2 GUI and a Svelte 5 + Tailwind + shadcn-svelte frontend (built with Vite).
Run with no arguments it opens the application window (navigation, Track and Settings views, About dialog).

This crate lives in `src/calliope-gui/` of the calliope workspace. Run every command below from
this directory. Build output goes to the workspace's `target/` at the repository root
(`../../target/` from here).

## Prerequisites (Arch Linux)

* Rust toolchain (cargo)
* Packages: webkit2gtk-4.1, gtk3, base-devel, alsa-lib
* Node.js >= 22.12 with npm
* Optional: xorg-server-xvfb for headless GUI runs
* For Import: `sudo pacman -S yt-dlp ffmpeg` (see "Import and stem extraction"; the app runs without them, Import then says what is missing)

Run `npm ci` once after cloning and after `package-lock.json` changes.

## Build

```bash
npm run build:app
```

Produces `target/release/calliope-gui` at the repository root (`../../target/release/calliope-gui` from here).

Plain `cargo build` also works once `dist/` exists (built by `npm run build`). Without it, the build stops with a message saying the frontend is not built. If `src/ui/` is newer than `dist/`, a debug build only warns, but a release build fails; run `npm run build:app`.

## Run

```bash
npm run app                      # debug build, opens the window
../../target/release/calliope-gui   # release binary
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

* Alt+1 ... Alt+6: Library, Import, Editor, Playlists, Player, Settings
* Ctrl+B: collapse navigation

## Track repository

The Library view manages the backing tracks stored in the track repository folder.
The default folder is `~/.local/share/calliope/`; change it in Settings > Track repository.

* `tracks/<track id>/track.json`: the track metadata (JSON, with a `schema_version`)
* `tracks/<track id>/`: the audio file and the tablature files named in `track.json`
* `trash/`: deleted tracks and removed tablatures; Calliope never empties it
* `calliope-repository.json`: marks the folder as a Calliope repository

To try the Library with sample data, copy `tests/fixtures/library-sample` to a new folder
and choose that folder in Settings.

Library keys: Ctrl+F search, Ctrl+E edit, Ctrl+S save, Escape cancel editing.

## Import and stem extraction

The Import view (Alt+2) > "Stem Extraction" turns a song into a new "stem" track with six
FLAC stems (vocals, drums, bass, guitar, piano, other):

1. Pick the source: a URL (anything `yt-dlp` supports), a local audio file or a local video file
   (its audio track is used). Tracks longer than 15 minutes are refused.
2. Check the pre-filled band, album, title, composers, year, source link and copyright.
3. Extract. The audio is converted to FLAC, uploaded to the edge-AI server (`calliope-stems`),
   separated there and downloaded; "Working..." shows the progress. Cancel stops it and nothing
   is saved. A cancelled or failed URL download can be resumed the next time you enter the same URL.

Needs:

* `yt-dlp` (URL source) and `ffmpeg`/`ffprobe` on the PATH: `sudo pacman -S yt-dlp ffmpeg`.
  Settings > External tools shows what was found and its version. They are never bundled.
* A running `calliope-stems` server on your network, set in Settings > Stem extraction (for
  example `http://archserver:8765`, plain http only); "Test connection" checks it, the footer
  shows "Edge-AI: connected". Building, running and deploying the server is described in
  [`src/calliope-stems/README.md`](../calliope-stems/README.md).
* Settings > Stem extraction > "Keep the original mix with the stems" (off by default) also stores
  the converted source as `original.flac` in the track folder.

Where files go: the finished track is a new folder in the track repository
(`tracks/<id>/` with `track.json` plus `stems/*.flac`, and `original.flac` if kept). While a job
runs, its working files (downloads, converted audio) live in `<repository root>/import-tmp/` and
are removed when the job finishes or is discarded; only a cancelled or failed URL download is
kept there for the resume prompt. Your source files are never modified or moved. Licences of the
external tools: [`docs/licences.md`](../../docs/licences.md).

## Settings files

* `~/.config/app.calliope.gui/settings.json` (theme, repository folder, edge-AI address, keep-original)
* `~/.config/app.calliope.gui/.window-state.json`
* `~/.local/share/calliope/`: default track repository (see above)

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

GUI test screenshots go to `target/gui-shots/` at the repository root.

## Layout of this crate

Paths are relative to `src/calliope-gui/`. The workspace root holds `Cargo.toml` (members
only), `Cargo.lock`, `target/`, `docs/` and `specs/`.

```
package.json, package-lock.json    npm project of the GUI
vite.config.ts                     root: src/ui, outDir: dist/, $lib alias, vitest config
svelte.config.js                   vitePreprocess
tsconfig.json                      strict TS, paths $lib -> src/ui/lib
components.json                    shadcn-svelte CLI config
dist/                              build output of `vite build` (gitignored), embedded by Tauri
Cargo.toml, build.rs, tauri.conf.json, icons/, capabilities/
tauri.dev.conf.json                dev-only config (used only by npm run dev:app)
.taurignore                        keeps tauri dev from rebuilding Rust on frontend edits
src/*.rs                           Rust: main, cli, version, gui, ipc, settings, repository, import, tools, media
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
tests/                             cargo tests (cli, frontend, acceptance, gui_*); also the shared
                                   test fixtures and stubs used by calliope-lib and calliope-stems
```

Sibling crates: `../calliope-lib/` (shared library) and `../calliope-stems/` (edge-AI stem server).
