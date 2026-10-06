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
| calliope-gui (settings) | Laptop | Loads/saves user settings (theme, repository root, edge-AI server URL, keep-original switch; MIDI/audio later) as JSON | Rust + serde | `src/settings.rs` |
| calliope-gui (fs helpers) | Laptop | Atomic writes, no-clobber copies, file-name validation, moving to the repository trash, FNV-1a revisions | Rust, std only | `src/fsutil.rs` |
| calliope-gui (track metadata) | Laptop | `track.json` schema (version 2: track type, stems), validation, v1→v2 migration in memory, ids (UUIDv7), RFC 3339 timestamps | Rust + serde, `uuid` | `src/track_meta.rs` |
| calliope-gui (track repository) | Laptop | Repository layout, scan, save transaction, delete-to-trash, export, staged (atomic) creation of new tracks | Rust, pure (no Tauri) | `src/repository.rs` (+ `src/repository_tests.rs`) |
| calliope-gui (file dialogs) | Laptop | Native open/folder/save dialogs opened from Rust (tablatures, repository root, exports, import audio/video); pick tokens; scripted picker for e2e (`e2e-hooks` feature) | Rust + `tauri-plugin-dialog` | `src/picker.rs` |
| calliope-gui (import temp space) | Laptop | `<root>/import-tmp/` job folders, resume lookup, safe cleanup | Rust, pure | `src/import_tmp.rs` |
| calliope-gui (external tools) | Laptop | Finds `yt-dlp`/`ffmpeg`/`ffprobe` on `PATH`, version checks; runs them with `calliope_common::process` | Rust | `src/tools.rs` |
| calliope-gui (media) | Laptop | ffprobe JSON → audio check/duration/tags → `TrackEdits`; ffmpeg → FLAC 44.1 kHz stereo | Rust, pure | `src/media.rs` |
| calliope-gui (download) | Laptop | URL validation/normalisation (`url` crate), yt-dlp args, progress/error parsing, `info.json` mapping | Rust | `src/download.rs` |
| calliope-gui (import job) | Laptop | The single import job: state machine, events, cancel, shutdown | Rust, pure (std threads) | `src/import_job.rs` |
| calliope-gui (frontend) | Laptop (embedded webview) | Navigation shell, views (Library: track tree + track pane; Import: stem extraction steps; Settings), theme, keyboard shortcuts | Svelte 5 + TypeScript + Vite 8, shadcn-svelte (bits-ui, Tailwind v4), Inter font; built to `dist/` and embedded at compile time | `src/ui/` |
| calliope-common (shared lib) | Laptop + archserver | "calliope-stems API v1" types, validation, FLAC STREAMINFO parser, HTTP client (feature `client`), child-process runner (argv only, process group, PDEATHSIG, cancel). No Tauri/GTK | Rust; `serde`, `libc`, `ureq` 3 without TLS (feature) | `src/calliope-common/` |
| calliope-stems (edge-AI stems service) | archserver (LAN), deployed by hand as a systemd user unit | HTTP server for "calliope-stems API v1": validates FLAC uploads (≤ 15 min), queues jobs (1 running), runs a configurable separator command, serves the stems, cleans up. No Tauri/GTK; the GUI doesn't depend on it | Rust; `tiny_http`, `calliope-common` | `src/calliope-stems/` |
| separator adapter | archserver | `<sep> <input.flac> <out_dir> <model>` → `<out_dir>/<stem>.flac`; the real one runs the owner's audio-separator venv with `htdemucs_6s` (frees VRAM from Ollama first) | bash | `src/calliope-stems/separators/audio-separator.sh` |
| yt-dlp, ffmpeg, ffprobe | Laptop | External executables, installed by the user, never bundled | upstream | system `PATH` |
| audio-separator, Demucs, PyTorch | archserver | The real separation model, the owner's existing install in `~/edge-ai/stems/.venv`; never bundled, never run by tests | upstream (Python) | outside this repo |
| test stand-ins | dev machine | `tests/support/stub-separator` (canned stems, failure modes) used with the real `calliope-stems` on 127.0.0.1; `tests/support/bin/yt-dlp` (fake downloader) | bash / Python 3 stdlib | `tests/support/` |

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
  - Permissions (ACL): `build.rs` declares an app manifest
    (`tauri_build::Attributes::app_manifest(AppManifest::new().commands(&COMMANDS))`), which
    autogenerates `allow-<command>` permissions into `permissions/autogenerated/` (gitignored).
    `capabilities/main.json` grants exactly those to window `main`, local origin only. No
    `core:*`, `dialog:*` or `fs:*` permission is granted: plugins are used from Rust only.
    **Every new command** must be added in three places: `generate_handler!` in `src/gui.rs`,
    `COMMANDS` in `build.rs`, and `capabilities/main.json`. A static test in `tests/frontend.rs`
    checks they match.
  - Commands that do file IO or open dialogs are `async`, and their blocking work runs in
    `tauri::async_runtime::spawn_blocking`. Command argument names are single words (Tauri maps
    JS camelCase to Rust snake_case).
  - Commands today: `app_version() -> String` (same value as `--version`),
    `get_settings() -> Settings`, `set_theme(theme) -> Result<Settings, String>`,
    `frontend_log(message)` (prints `calliope-ui: <message>` to stderr); repository:
    `get_repository() -> RepoInfo`, `choose_repository_root() -> Option<PickedRoot>`,
    `set_repository_root(token) -> RepoInfo`, `reset_repository_root() -> RepoInfo`,
    `list_tracks() -> Library`, `pick_tablature(purpose) -> Option<Picked>`,
    `save_track(request) -> SaveResult`, `delete_track(id, revision)`,
    `export_track(id) -> Option<String>`, `export_tablature(id, name) -> Option<String>`;
    edge-AI and import (gui-stem-extraction plan §2.10): `set_edge_ai_url(url)`, `set_keep_original(keep)`,
    `check_edge_ai()`, `check_tools()`, `prepare_url_import(url)`,
    `start_url_import(url, resume, events)`, `import_file(kind, events)`,
    `start_stem_extraction(job, edits)`, `cancel_import(job)`, `discard_import(job)`,
    `get_import_job()`, `watch_import(events)`.
    Types are in `src/ui/lib/ipc.ts` and in the plans (tracks-repository §2.7, stem-extraction §2.10).
  - **Long-running jobs and progress**: a command that starts a job takes a
    `tauri::ipc::Channel<Event>` argument (`events`), returns a snapshot at once and runs the
    work on its own std thread; progress goes over the channel (tagged JSON, `phase` field,
    throttled to ≤ 10 messages/s). Channels need no capability permission (the channel-data
    fetch is exempt from the ACL in Tauri 2.12), whereas `emit`/`listen` would need
    `core:event:*`, which we don't grant. Each job kind keeps a snapshot command (`get_*`) and a
    `watch_*(events)` command to re-attach after a webview reload. Pure job modules emit to a
    `Fn(Event)` sink so they are tested without Tauri. Only one import job runs at a time;
    `RunEvent::Exit` cancels it (2 s cap).
  - **The frontend never sends file-system paths.** Files and folders the user chooses come
    from native dialogs opened by Rust, which returns an opaque token (plus the bare file name).
    Commands that consume a choice take the token. Track ids and file names sent by the
    frontend are validated and resolved strictly inside `<root>/tracks/<id>/`.
  - The frontend never keeps time. Audio/MIDI/sync logic stays in Rust; the frontend displays
    state and sends commands.
- **Frontend log lines** (stderr, prefix `calliope-ui: `), used by the GUI tests:
  `ready view=<id> theme=<t> version=<v>`, `view=<id>`, `theme=<t>`,
  `csp-violation directive=<d> blocked=<uri>`, `error <context>: <msg>`; Library:
  `library root=<p> status=<s> tracks=<n> problems=<m>`, `select id=<id>`, `mode=edit|view id=<id>`,
  `saved id=<id> tablatures=<n> warnings=<w>`, `deleted id=<id>`, `exported id=<id>`,
  `tab-staged <add|update|remove> name=<n>`, `repository root=<p> default=<bool>`.
  Import: `import mode=stem-extraction`, `import source=url|audio|video`,
  `import phase=<phase> job=<job>`, `import error stage=<s> message=<m>`, `import saved id=<id> stems=<n>`.
  Rust lines (prefix `calliope: `): `dialog kind=<add-tablature|update-tablature|repository-root|export-track|export-tablature|import-audio|import-video> result=<picked|cancelled>`,
  `import job=<job> phase=<phase>`, `tool <name> found=<bool> version=<v>`.
- **Settings file**: `$XDG_CONFIG_HOME/app.calliope.gui/settings.json` (Tauri
  `app_config_dir()`), e.g. `{"theme": "dark", "repository_root": null, "edge_ai_url": null, "keep_original": false}`
  (`repository_root`: `null`/absent = the default root, must be an absolute path;
  `edge_ai_url`: `null` = not configured, else `http://host[:port][/path]`, no TLS/userinfo/query;
  `keep_original`: keep the original full mix as `original.flac` with imported stems). Missing or invalid files give the defaults;
  writes are atomic (tmp + rename); unknown fields are ignored. Window geometry is in
  `.window-state.json` in the same directory (written by `tauri-plugin-window-state` on
  graceful exit).
- **Track repository on disk** (layout version 1, `track.json` schema 2; details and rules in
  `specs/gui-tracks-repository.plan.md` §2.2-2.4 and `specs/gui-stem-extraction.plan.md` §2.3-2.4):
  ```
  <root>/                         default <data_dir>/calliope (Linux: ~/.local/share/calliope)
    calliope-repository.json      {"schema_version": 1}
    tracks/<track-id>/            folder name == track id (UUIDv7 for new tracks)
      track.json                  metadata, "schema_version": 2 (1 still read)
      <audio>, <tablatures>       plain file names listed in track.json
      stems/<name>.flac           stems of a "stem" track, listed as "stems/<name>.flac"
      original.flac               the full mix, only when kept at import (setting)
    tracks/.staging-<id>/         a new track being assembled (marker .calliope-staging);
                                  renamed to tracks/<id> in one step; scan skips dot names
    import-tmp/                   Calliope's own import working space
      url-<fnv16hex>/             per URL (resumable): job.json, download.<ext>[.part], info.json, audio.flac
      file-<uuid>/                per local-file import: job.json, audio.flac
    trash/<UTCstamp>-<id>[-tablatures]/   deleted tracks / removed tablatures; never emptied
      (a deleted track folder is renamed to trash/<stamp>-<id>/ itself, no extra nesting; all
      tablature files removed or replaced in one save go into ONE trash/<stamp>-<id>-tablatures/,
      a name clash inside it suffixes the file as `name (2).ext`; `-2`.. suffix on folder clash)
  ```
  `track.json` fields: `schema_version, id, type ("backing"|"stem"), band, album, title,
  composers[], year, source_url, copyright, audio (required for backing, null allowed for stem),
  original (optional plain file name of the kept full mix), stems[{name, file}], stem_model,
  tablatures[], imported, modified` (RFC 3339 UTC).
  **Schema migration**: v1 files are migrated in memory (`type: "backing"`, `stems: []`) and are
  never rewritten by scans, lists, exports or imports; a Library Save writes schema 2 (lazy
  migration). Older builds show such a track as "written by a newer Calliope" and leave it
  alone. The repository marker stays at 1 because the layout additions are invisible to
  older builds. Calliope deletes only its own `import-tmp/<job>` folders (name pattern +
  `job.json`) and `.staging-*` folders (marker), never through symlinks. Unknown fields are kept
  on save. Lenient on read, strict on write: loading only fails for things that make a track
  unusable or unsafe (invalid JSON, wrong field types, newer `schema_version`, id/folder
  mismatch, empty title, bad `audio`/tablature file names); those tracks are reported as
  problems and never rewritten. Cosmetic issues (padded or control-char text, odd years, any
  or missing `imported`/`modified` text such as `+02:00` offsets) load fine and are never
  rewritten for that reason. Saving applies the strict rules to the edited fields, keeps
  `imported` verbatim and sets `modified` to now in canonical `YYYY-MM-DDTHH:MM:SSZ`. Writes: `write_atomic` (unique `.<name>.<pid>.<n>.tmp` created with
  `create_new` + fsync + rename + dir fsync; a user's own `track.json.tmp` is never touched). New files: no-clobber copies (part file + `hard_link`). Calliope never deletes
  user files: removals are `rename`s into `trash/`. Concurrency: one Mutex per app instance,
  and an optimistic `revision` check (FNV-1a of `track.json`) against other writers. No file
  watcher; the Library rescans each time it is shown. A configured root that doesn't exist is
  never created (it may be an unmounted disk); the default root is created on first use.
- **Edge-AI stems protocol ("calliope-stems API v1")**, types and client in
  `calliope-common` (`stems_api`, `stems_client`), server `calliope-stems`; plain HTTP on the
  LAN, base URL from settings (e.g. `http://archserver:8765`); full table in the
  gui-stem-extraction plan §2.7:
  `GET /v1/health` → `{"service":"calliope-stems","api":1,"version","models":[...],"default_model":"htdemucs_6s","busy","max_duration_s":900,"max_upload_bytes"}`;
  `POST /v1/jobs?model=<m>` (body = FLAC, `Content-Type: audio/flac`, `Content-Length`) →
  `202 {"job","state":"queued"}` (400 model, 411 no length, 413 size/duration, 415 not FLAC,
  503 queue full);
  `GET /v1/jobs/<id>` → `{"job","state":"queued|running|done|failed|cancelled","progress","stems","error"}` (404 unknown);
  `GET /v1/jobs/<id>/stems/<name>` → FLAC (409 not done); `DELETE /v1/jobs/<id>` → 204 (cancel + cleanup).
  Errors are `{"error": "..."}`. **No auth** (LAN only, owner-approved default). Client:
  connect 5 s / request 30 s timeouts, 1 s polling, job id and stem names validated, ≤ 16
  stems, ≤ 1 GiB each, FLAC magic checked. "running" = the server confirmed processing start
  (UI shows "Working..."). A protocol conformance suite (`src/calliope-stems/tests/conformance.rs`)
  pins it.
- **`calliope-stems` command line**: `--listen ADDR:PORT` (default `0.0.0.0:8765`; prints
  `calliope-stems listening addr=…` on stderr), `--work-dir` (default
  `~/.local/state/calliope-stems`), `--separator PATH` (**required**, so nothing starts the real
  model by accident), `--model` (`htdemucs_6s`), `--max-upload-mb` (300), `--max-duration-s`
  (900), `--queue` (2 waiting; 1 running, fixed), `--separator-timeout-min` (30),
  `--retention-hours` (24). Jobs live in memory; job folders `<work>/jobs/<uuid>/` carry a
  marker and are the only things it deletes. Logs on stderr (journald), prefix `calliope-stems: `.
  **Separator contract**: `<separator> <input.flac> <out_dir> <model>`, writes
  `<out_dir>/<stem>.flac`, optional stdout `progress <0..1>`, exit 0; outputs are validated.
- **Track length limit**: 15 minutes, checked by Calliope before upload and by the server
  (`MAX_DURATION_S` in `calliope-common`).
- **External tools** (not bundled, found on `PATH` once at start-up, versions checked):
  `yt-dlp` ≥ 2023.01 (`--ignore-config`, `--` before the URL, output only into the job
  folder), `ffmpeg`/`ffprobe` ≥ 5 (inputs as `file:<abs path>`, `-nostdin`, `-n`). Always argv
  arrays, never a shell. Children run in their own process group with PDEATHSIG (Linux).
- **Window**: label `main`, title `calliope`, default 1280x800 centred, minimum 1024x640,
  created hidden and shown by the window-state plugin after it restores the geometry.

## Repository layout

```
Cargo.toml / Cargo.lock   single package `calliope-gui` at repo root (lock committed)
build.rs                  version generation, frontend-built check, tauri_build::build()
capabilities/main.json    ACL: the app's own commands for window `main` (no plugin/core permissions)
permissions/autogenerated/  generated by build.rs from the app manifest (gitignored)
tauri.conf.json           Tauri 2 config (frontendDist = "dist", strict CSP, bundling disabled; never devUrl/devCsp)
tauri.dev.conf.json       dev-only overlay (devUrl, beforeDevCommand, devCsp), used only by `npm run dev:app`
.taurignore               keeps `tauri dev` from rebuilding Rust on frontend edits
package.json / -lock      npm project at the root (scripts below; lock committed)
vite.config.ts            Vite root = src/ui, outDir = dist/, $lib alias, vitest config
svelte.config.js, tsconfig.json, components.json (shadcn-svelte CLI config)
icons/                    app icons
src/                      ALL source
  *.rs                    Rust modules (main, cli, version, gui, ipc, settings, fsutil,
                          track_meta, repository (+ repository_tests), picker, import_tmp,
                          tools, media, download, import_job, ...)
  ui/                     frontend: index.html, main.ts, app.css, App.svelte,
                          components/ (library/ = tree, pane, tablature panel;
                          import/ = source page, edit pane, extraction progress;
                          TrackFields.svelte shared by Library and Import),
                          views/ (one file per main view),
                          lib/ (ipc.ts, views.ts, theme.ts, app-state.svelte.ts,
                          fuzzy.ts, library-tree.ts, track-draft.ts, library-state.svelte.ts,
                          url-check.ts, import-state.svelte.ts,
                          utils.ts, components/ui/ = shadcn components owned by us),
                          *.test.ts next to the code
  calliope-common/        workspace member: shared lib (Cargo.toml, src/{lib,stems_api,
                          stems_client,process}.rs); no Tauri
  calliope-stems/         workspace member: the edge-AI stems server (Cargo.toml, src/*.rs,
                          tests/conformance.rs, separators/audio-separator.sh,
                          deploy/calliope-stems.service, README.md); no Tauri
dist/                     generated by `vite build`, gitignored, embedded into the binary
tests/                    cargo integration tests (cli, frontend, acceptance_*, gui_smoke, gui_e2e,
                          gui_library_e2e, gui_import_e2e); tests/fixtures/library-sample/ = sample
                          repository (v1), library-v2/ = mixed v1/v2 incl. a stem track,
                          import/ = generated audio/video/stem fixtures (make-fixtures.sh)
  support/                test-only stand-ins: stub-separator (bash), bin/yt-dlp (Python 3 stdlib)
specs/, docs/             specs, plans, architecture, UI guide (docs/ui.md), licences (docs/licences.md)
```

All source lives under `src/` (an owner requirement). Config files stay at the root next to
`Cargo.toml`. The root `Cargo.toml` is both the `calliope-gui` package and a Cargo workspace
(`members = ["src/calliope-common", "src/calliope-stems"]`, `default-members` = the root
package); further crates go in `src/<crate>/`. One `Cargo.lock` and `target/` for all. A static
test keeps Tauri/GTK out of the server and common crates and the server out of the GUI.

## Build and run

- Prerequisites: Rust, `webkit2gtk-4.1 gtk3 base-devel`, Node.js >= 22.12 with npm; run
  `npm ci` once after cloning and after `package-lock.json` changes. At run time the import
  needs `yt-dlp` and `ffmpeg` (`pacman -S yt-dlp ffmpeg`); tests need `ffmpeg`, `python3` and
  `unshare`/`ip` (util-linux, iproute2), never yt-dlp or the real separation model.
- Server: `cargo build --release -p calliope-stems` gives `target/release/calliope-stems`
  (needs no Tauri/GTK/npm). Deployment to archserver is manual (README in
  `src/calliope-stems/`, example unit `deploy/calliope-stems.service`, `systemctl --user`).
- Build: `npm run build:app` (= `vite build && cargo build --release`) gives
  `target/release/calliope-gui`.
- Run: `npm run app` (= `vite build && cargo run`).
- Plain `cargo build` does not run npm. It works once `dist/` exists. If
  `dist/index.html` is missing, `build.rs` stops with a message naming `npm run build:app`.
  If `src/ui/` is newer than `dist/index.html`, a debug build prints a `cargo:warning` and a
  release build fails (naming `npm run build:app`).
  Vite/Vitest cache lives in the root `node_modules/.vite` (`cacheDir`), not in `src/ui/`,
  because build.rs watches `src/ui`.
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
- Pure logic (parsing, formatting, settings, metadata, later: timing math) is unit-tested in
  its Rust module. The repository is tested on temp repositories (`tempfile`) in
  `src/repository_tests.rs`. The crate is a bin, so these are module tests, not `tests/` files.
  Each test also checks that a sibling "outside" folder is unchanged. The committed sample
  repository `tests/fixtures/library-sample/` is always copied to a temp dir first.
- **Tests never touch the user's data**: no test reads or writes `~/.local/share/calliope`,
  `~/.config/app.calliope.gui` or the OS trash. Rust tests take paths as parameters (no env
  lookups), frontend tests fake IPC, and GUI tests use temp XDG dirs and assert that the
  repository root is under `target/`.
- CLI behaviour is tested headlessly by running the built binary (`CARGO_BIN_EXE_calliope-gui`)
  with `DISPLAY`/`WAYLAND_DISPLAY` removed.
- Static checks (`tests/frontend.rs`): strict production CSP (no `'unsafe-inline'`/`'unsafe-eval'`/`ws:`), no `devCsp`/`devUrl` in `tauri.conf.json`, `devCsp` in `tauri.dev.conf.json` = production CSP plus only the two allowed tokens, window config, no
  inline code in `src/ui/index.html` or `dist/index.html`, no static `style=` in `.svelte`,
  all assets local (woff2 bundled, no `http(s)://` or `data:` URLs in CSS).
- Native file dialogs in GUI tests: the cargo feature `e2e-hooks` (only `npm run test:gui`
  enables it) plus the env var `CALLIOPE_E2E_DIALOG_ANSWERS=<file>` replace the dialogs with a
  scripted picker (one line per dialog: `<kind> <abs path>|CANCEL`; kinds include
  `import-audio`, `import-video`). Without the env var the
  real GTK dialogs open; one test checks a real dialog by screenshot and Escape. `build.rs`
  fails a release build with `e2e-hooks`, and a static test keeps it out of the release npm
  scripts and the default features.
- Frontend logic and components: vitest + jsdom + @testing-library/svelte, with IPC faked by
  `mockIPC` from `@tauri-apps/api/mocks` (`src/ui/**/*.test.ts`). Type check: `svelte-check`.
- **GUI on a real display**: agents have X display `:1` (i3, 1920x1200). `gui-shot <out.png>
  [window-regex] [timeout]` takes a screenshot (the agent then views the PNG), `xdotool` sends
  keys and clicks, and `i3-msg` floats, resizes and closes windows (i3 tiles windows and
  ignores the requested size/position unless they're floating). GUI tests are `#[ignore]`d
  cargo tests (`tests/gui_smoke.rs`, `tests/gui_e2e.rs`, `tests/gui_library_e2e.rs`,
  `tests/gui_import_e2e.rs`), run with
  `DISPLAY=:1 npm run test:gui` (built with `--features e2e-hooks`). Library e2e tests also
  assert on the files on disk. They launch the real binary with temporary
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` (never the user's real config), assert
  on the `calliope-ui:` stderr lines and on the settings/window-state files, fail on any
  `csp-violation`, and save screenshots to `target/gui-shots/` for visual review. "Offline"
  is a launch under `unshare -rn` (no network namespace; X11 still reachable). Agents never
  open windows on any other display. (The earlier note that agents have no display server is
  outdated.)
- Rust suites run with `--workspace` (`cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`); `npm test` builds `calliope-stems` first.
- **Edge-AI server** → the real `calliope-stems` binary (`target/debug/calliope-stems`, or
  `CARGO_BIN_EXE_calliope-stems` in its own tests) with `--listen 127.0.0.1:0` (port printed
  on stderr) and `--separator tests/support/stub-separator` (canned stems from
  `tests/fixtures/import/stems/`; modes `ok|fail|slow|hang|bad-output|not-flac` via
  `STUB_SEPARATOR_MODE` or `<work dir>/stub-mode`; argv logged to `$STUB_SEPARATOR_LOG`). The
  real adapter is only syntax-checked (`bash -n`); no test runs audio-separator or the model.
  The conformance suite `src/calliope-stems/tests/conformance.rs` checks every endpoint with
  raw HTTP and with the shared client.
- **YouTube / yt-dlp** → `tests/support/bin/yt-dlp` (fake; refuses any host not ending in
  `.example`; behaviours by URL path: `ok`, `http403`, `offline`, `slow`, `no-total`; resumes
  `.part` files; logs its argv to `$FAKE_YTDLP_LOG`). Rust tests pass its path explicitly;
  GUI tests put `tests/support/bin` first on `PATH`.
- ffmpeg/ffprobe are the real local tools (no network) on self-generated fixtures.
- **No test reaches the internet or the LAN.** Import GUI tests run the app under
  `unshare -rn` with only loopback up, and `calliope-stems` (stub separator) inside the same
  namespace on 127.0.0.1:8765; tests use `127.0.0.1` and `.example` URLs only. Agents never
  deploy, install or start `calliope-stems` with the real separator.
- Hardware (MIDI, audio): no fakes yet. Each feature that adds one must also add a
  simulator/fake and list it here.

## Decision log
<!-- Newest last. Format:
### YYYY-MM-DD: <decision>   (feature: <spec>)
Context, the choice, alternatives considered, consequences. -->

### 2026-10-04: Single Cargo package at the repo root, all source in `src/`   (feature: gui-skeleton)
The owner requires source under `src/`. The standard Tauri layout (`src-tauri/` for Rust,
`src/` for JS) would break that. Choice: one package at the root, Rust in `src/*.rs`,
frontend in `src/ui/`. Alternative considered: the standard Tauri layout, rejected because of
the requirement. Consequence: `tauri.conf.json` sits at the root next to `Cargo.toml`.
**Extended** by gui-stem-extraction: the root is now also a workspace with members in
`src/<crate>/` (see the workspace entry below).

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
message if `dist/index.html` is missing, and warns if it is stale (fails in release builds). Alternative: the
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
**The "no capabilities" part is superseded** by the gui-tracks-repository ACL entry below.

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

### 2026-10-05: Track repository = plain folders + versioned `track.json` per track   (feature: gui-tracks-repository)
The overview asks for something "not so complicated and easy to hack with"; the spec asks
for JSON with a version number. Choice: `<root>/tracks/<id>/track.json` next to the track's
files, plus a `calliope-repository.json` marker with its own `schema_version` for whole-layout
changes. No database or index file: scanning a few hundred small JSON files is fast, and there is
no index to get out of sync. Alternatives: SQLite (rejected: not hand-editable, and it
duplicates the files' truth) or one big library JSON (rejected: one corrupt write loses
everything, and it merges badly). Consequences: a problems list for unreadable folders;
unknown fields are preserved; newer-schema files are read-only to older builds; a migration
hook exists from v1.

### 2026-10-05: Track ids are UUIDv7; folder name = id; flat layout   (feature: gui-tracks-repository, default; the owner didn't object)
Unique without coordination, time-ordered, standard, and not tied to the metadata (req 6).
Hand-made folders with simple names are accepted as ids too. No sharding: a flat folder of
thousands of entries is fine on current file systems, and easier to browse by hand.

### 2026-10-05: Nothing is deleted: removals go to `<root>/trash/`   (feature: gui-tracks-repository, confirmed by the owner 2026-10-05)
The repository is the user's real data. Deleting a track or removing/replacing a tablature
`rename`s it into the repository's own trash folder (same file system, so atomic, and it
works the same on every OS and in tests). Alternative: the freedesktop/OS trash (`trash`
crate), rejected for now because it is harder to sandbox in tests and differs per OS. Calliope
never empties the trash.

### 2026-10-05: Save transaction order and optimistic concurrency   (feature: gui-tracks-repository)
Copy new files in first (no-clobber), then write `track.json` atomically, then move old files to
the trash. A failure before the metadata write undoes only Calliope's own new copies; a
failure after it only leaves orphans, reported as warnings. External changes (another instance,
hand edits) are detected with a content revision (FNV-1a of `track.json`) sent back on
save/delete. There are no lock files or file watchers.

Accepted limits (fix round 1):
- No cross-instance lock: two instances are protected only by the revision check and unique
  temp names (they never share or truncate a temp file), not by mutual exclusion.
- A crash between the trash move and the part-file rename during a same-name tablature
  replace leaves the new version as `.<name>.part` in the track folder and the old one in
  `trash/`. Nothing is lost.
- Trash moves act on the link itself (never a symlink's target) and refuse a `trash/` that is
  not a real folder. Export refuses any destination inside the repository root. The export
  folder name is capped at 240 bytes (75 bytes per part, cut on a UTF-8 boundary).
- A save conflict keeps edit mode and the draft; Cancel/Escape then discards and reloads.

### 2026-10-05: Native dialogs opened from Rust; the frontend only handles tokens   (feature: gui-tracks-repository)
`tauri-plugin-dialog` is used from Rust only (`blocking_*` inside async commands). Picks
return opaque tokens, so the webview never supplies a path that Calliope reads or writes.
This keeps the JS ACL empty of plugin permissions, and a compromised webview can't exfiltrate
or overwrite arbitrary files. Alternative: the JS dialog + fs plugins with scoped
permissions, rejected because they need broad fs scopes for user-chosen files and make the
frontend handle paths.

### 2026-10-05: App ACL manifest and `capabilities/main.json`   (feature: gui-tracks-repository)
Supersedes "no capabilities" from gui-frontend-foundation, now that commands reach the file
system (reviewer follow-up). Every app command gets an autogenerated `allow-*` permission,
granted only to window `main` from the local origin. No core/plugin permissions are granted.
A static test keeps the handler list, the build.rs list and the capability in sync.

### 2026-10-05: Test-only dialog injection behind the `e2e-hooks` cargo feature   (feature: gui-tracks-repository)
GUI e2e tests can't operate GTK file choosers reliably. A scripted picker is compiled in only
with `--features e2e-hooks` and is active only with `CALLIOPE_E2E_DIALOG_ANSWERS` set.
Release builds with the feature fail in `build.rs`. Alternative: a `debug_assertions` gate,
rejected because `npm run app` debug builds would also carry the hook.

### 2026-10-05: Library search is our own token matcher   (feature: gui-tracks-repository)
"Fuzzy" = each whitespace token must match band, album or title as a substring, or as a
subsequence that starts at a word start; case- and diacritic-insensitive. The tree stays
alphabetical (no ranking), so the matcher is deliberately stricter than fzf-style matching.
It is small enough to write ourselves (no dependency, no copied code; see the overview's IP
constraint).

### 2026-10-06: `track.json` schema 2 with a track `type`; lazy migration   (feature: gui-stem-extraction)
Stem tracks have no single audio file, so v2 adds `type` (`backing`|`stem`), `stems[{name,
file}]` (files in `stems/`), `stem_model`, and makes `audio` optional for stem tracks. v1 files
are migrated in memory only and rewritten as v2 only when the user saves them; scans, imports
and exports never rewrite them. Alternatives: an eager migration of the whole repository on
first start (rejected: it rewrites every file of the user's real data at once, and a bug would
damage all of it), or keeping v1 and putting stems in unknown fields (rejected: older builds
would happily save a stem track and could lose the meaning of its fields). Consequence: a
track saved by this build is read-only for older builds. The repository marker stays at 1.

### 2026-10-06: Imports work in `<root>/import-tmp/` and new tracks appear with one rename   (feature: gui-stem-extraction)
The spec puts temporary files inside the repository. One folder per URL (keyed by a hash of
the normalised URL) makes resuming a download a lookup; local-file imports get a UUID folder.
New tracks are assembled in `tracks/.staging-<id>/` (dot name: invisible to scans) and renamed
to `tracks/<id>` at the end, so the Library never shows a half-written track. Calliope deletes
only folders it can prove are its own (name pattern + `job.json`, or the staging marker),
never through symlinks. Alternative: the OS temp dir (rejected: the spec asks for the
repository, and `/tmp` may be a small tmpfs or a different file system, which breaks the atomic
rename).

### 2026-10-06: Long-running work = one std thread per job + a Tauri `Channel` for progress   (feature: gui-stem-extraction)
Downloads, conversions and the edge-AI round trip take minutes. The start command returns a
snapshot at once; the job thread reports over a `tauri::ipc::Channel` passed as a command
argument. Channels need no capability permission, while `emit`/`listen` would require granting
`core:event:*` (we grant no core permissions). Alternatives: polling a status command
(simpler, but laggy progress and extra IPC traffic), events (rejected for the permission).
One import job at a time (the GPU processes one song at a time); closing the app cancels it.

### 2026-10-06: External tools yt-dlp and ffmpeg/ffprobe, run as processes, not bundled   (feature: gui-stem-extraction)
They are the standard tools for downloading and decoding every format; reimplementing them is
out of the question, and linking FFmpeg libraries would bring LGPL/GPL obligations and native
build pain. Run as separate executables found on `PATH`, they put no licence obligations on
Calliope as long as they are not bundled (recorded in `docs/licences.md`). Always argv arrays,
`--ignore-config` for yt-dlp, `--` before URLs, `file:` inputs for ffmpeg, own process group
plus PDEATHSIG so nothing survives a cancel or a crash. All audio is normalised to FLAC
44.1 kHz stereo before upload, so the server accepts one format.

### 2026-10-06: Edge-AI stems: "calliope-stems API v1" and our own `calliope-stems` server   (feature: gui-stem-extraction, owner decision Q1)
archserver had Ollama (localhost only, not used for stems) and the CLI `stems/backing`, but no
network service for stems. The owner approved the proposed minimal job protocol (health, upload
FLAC, poll, fetch stems, delete; "looks good as a starter") and asked to build the server in this
feature as a separate Rust binary, kept cleanly separated because it may get its own spec later.
Choices: plain HTTP on the LAN, no auth (bounded by FLAC-only uploads, size/duration caps, a
queue of 2, no client-supplied paths or commands); `tiny_http` (synchronous, one client, no
async runtime); one job running at a time (one GPU); in-memory job state (a restart drops jobs,
clients retry); the separation itself is a **configurable separator command** with a small
contract, so the model can change without touching the server, and tests use a stub. The
`--separator` flag has no default so nothing can start the real model by accident. Client:
`ureq` (blocking, no TLS). Alternatives: a Python service in the existing venv (rejected by the
owner's choice of Rust and to keep one toolchain), SSH + remote CLI (needs keys, no clean
progress/cancel). Deployment is manual by the owner (systemd user unit).

### 2026-10-06: Cargo workspace with shared `calliope-common`; GUI and server never depend on each other   (feature: gui-stem-extraction)
The server must not pull in Tauri/GTK, and the GUI must not depend on server code. A second
`[[bin]]` in the root package would share Tauri and `build.rs` (frontend checks), so the server
is a workspace member `src/calliope-stems/`; protocol types, the client (feature `client`) and
the child-process runner live in `src/calliope-common/`, the only crate both use. This follows
the existing rule "more crates → workspace, crates in `src/<crate>/`". `default-members` keeps
`cargo build` / `npm run build:app` GUI-only; test and clippy commands use `--workspace`. A
static `cargo metadata` test enforces the separation.

### 2026-10-06: Keep the original mix optionally; 15-minute limit   (feature: gui-stem-extraction, owner decisions Q3, Q4)
A Settings switch "Keep the original mix with the stems" (default off) stores the normalised
FLAC as `original.flac` in the track folder and records it as `original` in `track.json` v2. The
maximum track length is 15 minutes, checked by Calliope before uploading and by the server
(`MAX_DURATION_S` in `calliope-common`, server from the FLAC STREAMINFO).

### 2026-10-06: URL import through yt-dlp for personal use   (feature: gui-stem-extraction, owner decision Q2)
Any http/https URL that yt-dlp supports is accepted. The overview's IP constraint (never copy
other products' code, graphics, icons or text) is not affected by calling an external tool.
yt-dlp is installed by the owner and never bundled; Calliope shares nothing and uploads only to
the owner's own edge-AI server.

### 2026-10-06: Network-isolated GUI tests for features that talk to the network   (feature: gui-stem-extraction)
The import GUI tests start the app inside `unshare -rn` with only loopback up and run
`calliope-stems` (with the stub separator) inside the same namespace, while the fake yt-dlp
comes first on `PATH`. Even a
bug in the code or a misconfigured test cannot reach YouTube or the LAN.

