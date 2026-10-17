# calliope-gui

Rust desktop application with a Tauri 2 GUI and a Svelte 5 + Tailwind + shadcn-svelte frontend (built with Vite).
Run with no arguments it opens the application window (navigation, Track and Settings views, About dialog).

This crate lives in `src/calliope-gui/` of the calliope workspace. Run every command below from
this directory. Build output goes to the workspace's `target/` at the repository root
(`../../target/` from here).

## Prerequisites (Arch Linux)

* Rust toolchain (cargo)
* Packages: webkit2gtk-4.1, gtk3, base-devel, alsa-lib
* On a PipeWire desktop, also `pipewire-alsa` (needed to run, not to build): without it ALSA's
  default device is the sound card itself, which PipeWire already holds, so Play fails
* Node.js >= 22.12 with npm
* For GUI tests: xorg-server-xvfb (or, as a fallback, Xorg with xf86-video-dummy), i3, xdotool, maim
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
* Player (Alt+5): Space play/pause, b / e mark the loop start / end, Left / Right previous / next loop,
  l or Shift+L loop on/off, Ctrl+L collapse the track tree, Ctrl+F search. The letter keys do nothing
  while a field, button or the source dropdown has the focus. Home / End jump to the start / the last bar of
  the selected section (also with the waveform focused, not while typing in the name box).
* Player waveform (click it or Tab to it): Up / Down select a tempo section, Left / Right move the selected
  change by 1/64 beat, Shift+Left / Shift+Right by one beat (limit 4 beats from where it was detected or
  added), Enter edits its BPM, Delete or Backspace removes the change, `a` adds one at the playhead.
  Space, b, e and l keep their Player meaning.
* Mix waveform (Editor) with detected tempo changes: click it or Tab to it, then Left / Right move the
  selected change by 1/64 beat, Shift+Left / Shift+Right by one beat (limit 4 beats from where it was
  detected), Up / Down select another change, Escape cancels a mouse drag.

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

## Editor

The Editor view (Alt+3) mixes the stems of the track selected in the tree into a backing track.

* One lane per stem: Play (solo that stem; pressing it again ends the solo), Unmute, a volume
  slider (-60 to +12 dB, tick at 0 dB; the dB value resets it to 0 dB).
* The pinned Mix lane has Play/Pause, Stop, the position slider, the time field (type a time, or
  use the mouse wheel to nudge by 100 ms), the CLIP badge, and Save. Save renders the mix of the
  unmuted stems to `backings/backing.flac` (the old file goes to the trash).
* Above the Mix lane the waveform shows the mix of the unmuted stems (it follows the checkboxes and
  volume sliders a moment after the last change) with the playhead.
* "Autodetect BPM" (needs the edge-AI server in Settings; the server needs a beat tracker, see
  `src/calliope-stems/README.md`) uploads the unmuted mix, finds the beats and stores the tempo
  sections in `player.json`. The sections show as amber bars with their BPM; drag a bar or use the
  keys (see Keyboard) to fine-tune it. Cancel stops a running detection. The Player uses the same
  tempo afterwards; a track open in the Player is closed while the Editor writes it and reopens
  (with its loops) when you go back to it.
* Keys: Space play/pause, Ctrl+S save, Ctrl+F search; Tab reaches every control.
* Audio output needs `alsa-lib` (the `cpal` backend, default device) and, on PipeWire,
  `pipewire-alsa`. Tests never open a sound
  device: unit tests use `ManualBackend`, the GUI tests use `NullBackend` (builds with
  `e2e-hooks` can only use it; `CALLIOPE_E2E_AUDIO=capture:<path>` also writes what would have
  been heard to a WAV).
* To try it: copy `tests/fixtures/library-editor` to a new folder and choose it in Settings.

## Player

The Player view (Alt+5) plays the track selected in the tree (the same tree as the Library and the Editor;
Ctrl+L folds it to a narrow rail).

* "Play:" chooses the audio: the backings, the plain audio, the original mix or one stem. Changing it
  stops playback and keeps the position.
* The waveform shows the whole source, the playhead and the loops (the current loop in amber, a loop
  without an end as a start mark only). Tempo sections (from Autodetect BPM in the Editor or a typed
  tempo) show as bars labelled with their BPM; the Player edits them (see Sections below).
* BPM: type a tempo (20 to 300; 0 clears it) to place it at the playback position. Metronome: On/Off
  clicks on the beat, Count In plays three clicks before the audio starts. The bar fields of the loops
  need a BPM; without one they show times.
* Sections: the tempo can change along the song. Click a section in the waveform to select it (else the
  one at the playhead is used) and drag a change's bar to move it, or use the keys (see Keyboard). Click
  a label to type that section's BPM. The Sections group has Add (a new change at the playhead, you type its
  BPM), Remove (joins the section to the previous one), x2 and /2 (for a half- or double-time detection).
  The metronome follows every edit from the next beat, also while playing. The sections are saved in
  `player.json` and shown in the Editor.
* Per section (Sections group, for the selected section): the name box next to "Section 2 of 3 - 93 BPM"
  (Enter or leaving the box saves it, Escape drops the edit; it is shown in the box and spoken by the waveform; the Editor does
  not show names), "This section" Click (the section's metronome clicks; off makes the section silent)
  and Count In (three clicks at this section's tempo just before it, replacing the end of the section
  before; refused for the first section, and skipped where a repeating loop ends at the section), and Start
  / End (jump to the section's first beat / the start of its last bar; playing continues without a pause).
  The Metronome group (On/Off, Count In for Play) is part of the Sections group; On/Off silences every
  click, also the section count-ins.
* Playback: Rew / FF jump 5 s, the time field takes a time (the mouse wheel nudges by 100 ms), the
  slider seeks.
* Loops: "+" starts a loop at the position (it has no end until you press "<"), ">" and "<" set the
  start and end at the position, Prev/Next select, "-" removes. The loop Play button plays the current
  loop; Loop On repeats it, Count-In counts in before every pass.
* Everything is saved by itself in `player.json` next to `track.json` (source, tempo, metronome,
  loops, position) and restored the next time the track is opened.
* Audio output and tests: as for the Editor. To try it, copy `tests/fixtures/library-player` to a new
  folder and choose it in Settings.

## Import and stem extraction

The Import view (Alt+2) > "Stem Extraction" turns a song into a new "stem" track with six
FLAC stems (vocals, drums, bass, guitar, piano, other):

1. Pick the source: a URL (anything `yt-dlp` supports), a local audio file or a local video file
   (its first audio track is used). Tracks longer than 15 minutes are refused. For a local file,
   type or paste its full path (`/...`, `~/...`, a quoted path or a `file://` link) in the path box,
   or let Browse fill it in, then press Extract or Enter. A path that cannot be used is reported
   under the box and nothing is imported:
   * empty box: `Enter the path of a file or choose one with Browse`
   * relative path: `Enter the full path, starting with / or ~/`
   * `No file found at <path>`
   * `<path> is a folder, not a file`
   * `<path> is not a regular file`
   * `Cannot read <path>` (for example `(permission denied)`)
   * wrong kind of file: `"<name>" is not an audio file` (or `... not a video file`)
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
npm run test:gui                 # GUI tests in parallel: one private X display (+ i3) per test binary
DISPLAY=:1 npm run test:gui:serial   # the same tests one after the other on an existing display
```

`npm run test:gui` (`scripts/test-gui.sh`) builds once, starts one Xvfb (or Xorg with the dummy
driver) per test binary from display `:90` up, runs each binary with `--ignored
--test-threads=1`, and prints a summary with the time per binary. It never uses `$DISPLAY`. Output per binary is in `target/gui-logs/<binary>.log`. To run one test:
`scripts/test-gui.sh -- <test name>` (the filter goes to every binary).

GUI test screenshots go to `target/gui-shots/` at the repository root. The player's visual
review sweep (dark and light, 1280x800 and 1024x640, tree shown and collapsed, inactive, active
counting in, section selected with its toggles and name, BPM overlay open, and the pane scrolled: 46
screenshots) only runs with `CALLIOPE_GUI_SHOTS=1`, e.g.
`CALLIOPE_GUI_SHOTS=1 npm run test:gui`; by default it takes one screenshot.

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
src/*.rs                           Rust: lib (module list), main (thin binary), cli, version, gui, ipc, settings, repository, import, tools, media
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
