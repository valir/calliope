# Architecture

<!-- Maintained by the architect agent. It records the technical decisions that span
     features, so each new feature stays consistent with the previous ones. You can edit
     it too; the architect treats your edits as decisions. -->

**Paths in this document.** The repository is a Cargo workspace of three crates in
`src/calliope-gui/`, `src/calliope-lib/` and `src/calliope-stems/`. Paths that start with
`src/calliope-<crate>/` are relative to the repository root, and so are `target/`, `docs/` and
`specs/`. All other paths (`src/*.rs`, `src/ui/...`, `tests/...`, `build.rs`,
`tauri.conf.json`, `package.json`, `capabilities/`, `dist/`, `node_modules/`) are relative to
the GUI crate, `src/calliope-gui/`.

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
| calliope-gui (external tools) | Laptop | Finds `yt-dlp`/`ffmpeg`/`ffprobe` on `PATH`, version checks; runs them with `calliope_lib::process` | Rust | `src/tools.rs` |
| calliope-gui (media) | Laptop | ffprobe JSON → audio check/duration/tags → `TrackEdits`; ffmpeg → FLAC 44.1 kHz stereo | Rust, pure | `src/media.rs` |
| calliope-gui (download) | Laptop | URL validation/normalisation (`url` crate), yt-dlp args, progress/error parsing, `info.json` mapping | Rust | `src/download.rs` |
| calliope-gui (import job) | Laptop | The single import job: state machine, events, cancel, shutdown | Rust, pure (std threads) | `src/import_job.rs` |
| calliope-gui (stem audio) | Laptop | FLAC stem probing/compatibility rules, decoding into memory as 16-bit for playback, full-precision streaming reader for rendering, the empty-stem measure for imports (`silent_peak`, `SILENT_STEM_DBFS = -50.0`) | Rust, `claxon` | `src/stem_audio.rs` |
| calliope-gui (mixer) | Laptop | Pure mixing shared by playback and Save: dB gains (-60 = Off .. +12), unmuted, mono->stereo, sum, hard clip with clipped-sample count, quantise | Rust, std only | `src/mixer.rs` |
| calliope-gui (transport) | Laptop | Play/pause/stop/seek state machine with the 0.3 s scrub-resume rule; time injected | Rust, pure | `src/transport.rs` |
| calliope-gui (audio output) | Laptop | `OutputBackend` trait: `CpalBackend` (default device), `NullBackend` (paced, optional WAV capture; the only backend in `e2e-hooks` builds), `ManualBackend` (unit tests) | Rust, `cpal` (ALSA on Linux) | `src/audio_out.rs` |
| calliope-gui (editor engine) | Laptop | Editor sessions: load stems, lock-free render callback, transport, transient solo, clip indicator, events, remembered mixes, the backing save job | Rust (std threads) | `src/editor.rs` (+ `src/editor_tests.rs`) |
| calliope-gui (backing render) | Laptop | Offline render of the mix to FLAC | Rust, `flacenc` | `src/backing_render.rs` |
| calliope-gui (frontend) | Laptop (embedded webview) | Navigation shell, views (Library: track tree + track pane; Import: stem extraction steps; Editor: shared track tree + stem mixer; Settings), theme, keyboard shortcuts | Svelte 5 + TypeScript + Vite 8, shadcn-svelte (bits-ui, Tailwind v4), Inter font; built to `dist/` and embedded at compile time | `src/ui/` |
| calliope-lib (shared lib) | Laptop + archserver | "calliope-stems API v1" types, validation, FLAC STREAMINFO parser, HTTP client (feature `client`), child-process runner (argv only, process group, PDEATHSIG, cancel). No Tauri/GTK | Rust; `serde`, `libc`, `ureq` 3 without TLS (feature) | `src/calliope-lib/` |
| calliope-stems (edge-AI stems service) | archserver (LAN), deployed by hand as a systemd user unit | HTTP server for "calliope-stems API v1": validates FLAC uploads (≤ 15 min), queues jobs (1 running), runs a configurable separator command, serves the stems, cleans up. No Tauri/GTK; the GUI doesn't depend on it | Rust; `tiny_http`, `calliope-lib` | `src/calliope-stems/` |
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
    `get_import_job()`, `watch_import(events)`; editor (gui-backing-track-editor plan §2.7):
    `open_editor(id, events)`, `close_editor()`, `get_editor()`, `watch_editor(events)`, `editor_play(id)`,
    `editor_lane_play(id, name)` (check + solo + play), `editor_end_solo(id)`, `editor_pause(id)`,
    `editor_stop(id)`, `editor_seek(id, position)`, `editor_nudge(id, delta)` (ms),
    `editor_set_stem(id, name, gain, unmuted)` (gain in dB or null = Off), `save_backing(id)`,
    `cancel_backing_save(id)`.
    Types are in `src/ui/lib/ipc.ts` and in the plans (tracks-repository §2.7, stem-extraction §2.10,
    backing-track-editor §2.7).
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
    state and sends commands. Audio is played by Rust (`cpal`), never by the webview (no Web
    Audio, no `<audio>`, so the CSP needs no `media-src`/`blob:`). Playback position reaches the UI
    as `transport` events on the editor channel (on every change, else every 100 ms while playing).
- **Frontend log lines** (stderr, prefix `calliope-ui: `), used by the GUI tests:
  `ready view=<id> theme=<t> version=<v>`, `view=<id>`, `theme=<t>`,
  `csp-violation directive=<d> blocked=<uri>`, `error <context>: <msg>`; Library:
  `library root=<p> status=<s> tracks=<n> problems=<m>`, `select id=<id>`, `mode=edit|view id=<id>`,
  `saved id=<id> tablatures=<n> warnings=<w>`, `deleted id=<id>`, `exported id=<id>`,
  `tab-staged <add|update|remove> name=<n>`, `repository root=<p> default=<bool>`.
  Import: `import mode=stem-extraction`, `import source=url|audio|video`,
  `import phase=<phase> job=<job>`, `import error stage=<s> message=<m>`,
  `import saved id=<id> stems=<n> original=<b> dropped=<a,b|none>`.
  Editor: `editor active id=<id> stems=<n>`, `editor inactive id=<id|none>`,
  `editor play|pause|stop position=<ms>`, `editor solo name=<n|none>`, `editor leave stop`,
  `editor seek position=<ms>`, `editor nudge delta=<ms>`,
  `editor stem name=<n> gain=<db|off> unmuted=<b>`, `editor saved id=<id> file=<f> clipped=<n>`.
  Rust lines (prefix `calliope: `): `dialog kind=<add-tablature|update-tablature|repository-root|export-track|export-tablature|import-audio|import-video> result=<picked|cancelled>`,
  `import job=<job> phase=<phase>`, `tool <name> found=<bool> version=<v>`,
  `audio backend=<cpal|null|capture> rate=<hz>`, `editor open id=<id> stems=<n> duration_ms=<ms>`,
  `editor transport playing=<b> audible=<b> position_ms=<ms> solo=<n|none> clipping=<b>` (changes only),
  `editor saved id=<id> file=<f> bits=<b> clipped=<n>`.
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
      stems/<name>.flac           stems of a "stem" track, listed as "stems/<name>.flac"; 1..16 of them
                                  (an import leaves out silent stems, so often fewer than the model's 6)
      original.flac               the full mix, only when kept at import (setting)
      backings/<variant-id>.flac  backing-track variants of a stem track made in the Editor (listed in
                                  `backings`); this feature writes `backings/backing.flac` (`backing-2`..
                                  if a user file already has the name)
      .calliope-backing-<uuid>.part  a backing being rendered (removed on success/failure/cancel)
    tracks/.staging-<id>/         a new track being assembled (marker .calliope-staging);
                                  renamed to tracks/<id> in one step; scan skips dot names
    import-tmp/                   Calliope's own import working space
      url-<fnv16hex>/             per URL (resumable): job.json, download.<ext>[.part], info.json, audio.flac
      file-<uuid>/                per local-file import: job.json, audio.flac
    trash/<UTCstamp>-<id>[-tablatures|-backing]/   deleted tracks / removed tablatures / replaced backing files; never emptied
      (a deleted track folder is renamed to trash/<stamp>-<id>/ itself, no extra nesting; all
      tablature files removed or replaced in one save go into ONE trash/<stamp>-<id>-tablatures/,
      a name clash inside it suffixes the file as `name (2).ext`; `-2`.. suffix on folder clash)
  ```
  `track.json` fields: `schema_version, id, type ("backing"|"stem"), band, album, title,
  composers[], year, source_url, copyright, audio (required for backing, null allowed for stem),
  original (optional plain file name of the kept full mix), stems[{name, file}], stem_model,
  tablatures[], imported, modified` (RFC 3339 UTC), and the optional list `backings[{id, name, file,
  created, modified, sample_rate, bits, mix: {stems[{name, gain_db (number on a 0.5 dB grid, -59.5..12,
  or null = Off), unmuted}]}}]` of backing-track variants of a stem track. `id` is a stable slug
  (stem-name rules, unique) and names the file `backings/<id>.flac`; `name` is the display name
  (editable later without renaming files). A stem track keeps `type: "stem"`, its stems and
  `audio: null` (`audio` stays the single file of a plain backing track; the Player picks a variant from
  `backings`, the first being the default). `backings` is additive (no schema bump), omitted when empty;
  ids and file names are validated on read (unsafe ones make the track a problem), backing files count
  in `missing`, `mix` is read leniently. The Editor's Save creates or replaces one variant (default id
  `backing`, name "Backing"), changes only `backings` and `modified`, refuses (`conflict:`) if the stems
  list changed on disk since the Editor loaded it, and moves a replaced file to
  `trash/<stamp>-<id>-backing/`.
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
  `calliope-lib` (`stems_api`, `stems_client`), server `calliope-stems`; plain HTTP on the
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
  pins it. The protocol never promises a stem count; the GUI drops silent stems itself after
  fetching them (gui-stem-extraction-clear-empty plan §2.2): `ImportEvent::Saved` and `JobSnapshot`
  carry `dropped: [{name, peak_dbfs (null = digital silence)}]`; an all-silent result fails the
  import (stage `server`, "Every stem is silent (below -50 dBFS), so no track was saved").
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
  (`MAX_DURATION_S` in `calliope-lib`).
- **External tools** (not bundled, found on `PATH` once at start-up, versions checked):
  `yt-dlp` ≥ 2023.01 (`--ignore-config`, `--` before the URL, output only into the job
  folder), `ffmpeg`/`ffprobe` ≥ 5 (inputs as `file:<abs path>`, `-nostdin`, `-n`). Always argv
  arrays, never a shell. Children run in their own process group with PDEATHSIG (Linux).
- **Window**: label `main`, title `calliope`, default 1280x800 centred, minimum 1024x640,
  created hidden and shown by the window-state plugin after it restores the geometry.

## Repository layout

```
Cargo.toml                workspace only: members = the three crates below,
                          default-members = calliope-gui, resolver 2
Cargo.lock, target/       one lock (committed) and one build dir for the whole workspace
specs/, docs/             specs, plans, architecture, UI guide (docs/ui.md), licences (docs/licences.md)
CLAUDE.md, .claude/       agent instructions and settings
src/                      ALL source (owner requirement), one folder per crate
  calliope-gui/           the desktop app (package calliope-gui); npm project + Tauri app
    Cargo.toml, build.rs  version generation, frontend-built check, tauri_build::build()
    capabilities/main.json  ACL: the app's own commands for window `main` (no plugin/core permissions)
    permissions/autogenerated/  generated by build.rs from the app manifest (gitignored)
    tauri.conf.json       Tauri 2 config (frontendDist = "dist", strict CSP, bundling disabled; never devUrl/devCsp)
    tauri.dev.conf.json   dev-only overlay (devUrl, beforeDevCommand, devCsp), used only by `npm run dev:app`
    .taurignore           keeps `tauri dev` from rebuilding Rust on frontend edits
    package.json / -lock  the GUI's npm project (scripts below; lock committed)
    vite.config.ts        Vite root = src/ui, outDir = dist/, $lib alias, vitest config
    svelte.config.js, tsconfig.json, components.json (shadcn-svelte CLI config)
    icons/                app icons
    README.md             build, run, test and usage of the GUI
    src/*.rs              Rust modules (main, cli, version, gui, ipc, settings, fsutil,
                          track_meta, repository (+ repository_tests), picker, import_tmp,
                          tools, media, download, import_job, stem_audio, mixer, transport,
                          audio_out, editor (+ editor_tests), backing_render, ...)
    src/ui/               frontend: index.html, main.ts, app.css, App.svelte,
                          components/ (library/ = tree, pane, tablature panel;
                          import/ = source page, edit pane, extraction progress;
                          editor/ = EditorPane, StemLane, MixLane, TimeField;
                          library/TrackBrowser.svelte = tree column shared by Library and Editor;
                          TrackFields.svelte shared by Library and Import),
                          views/ (one file per main view),
                          lib/ (ipc.ts, views.ts, theme.ts, app-state.svelte.ts,
                          fuzzy.ts, library-tree.ts, track-draft.ts, library-state.svelte.ts,
                          url-check.ts, import-state.svelte.ts, editor-state.svelte.ts, time-format.ts,
                          utils.ts, components/ui/ = shadcn components owned by us),
                          *.test.ts next to the code
    dist/                 generated by `vite build`, gitignored, embedded into the binary
    tests/                cargo integration tests (cli, frontend, acceptance_*, gui_smoke, gui_e2e,
                          gui_library_e2e, gui_import_e2e, gui_editor_e2e); fixtures/library-sample/ = sample
                          repository (v1), library-v2/ = mixed v1/v2 incl. a stem track,
                          import/ = generated audio/video/stem fixtures (make-fixtures.sh),
                          library-editor/ = stem tracks for the Editor (editor/make-fixtures.sh)
      support/            test-only stand-ins: stub-separator (bash), bin/yt-dlp (Python 3 stdlib)
  calliope-lib/           shared library (package calliope-lib, lib calliope_lib): Cargo.toml,
                          src/{lib,stems_api,stems_client,process}.rs, tests/; no Tauri
  calliope-stems/         the edge-AI stems server: Cargo.toml, src/*.rs, tests/conformance.rs,
                          separators/audio-separator.sh, deploy/calliope-stems.service, README.md;
                          no Tauri
```

All source lives under `src/` (an owner requirement), one folder per crate; further crates go
in `src/<crate>/` and into the workspace `members`. Each crate keeps its own config files next
to its `Cargo.toml`. The shared test fixtures and stand-ins live in the GUI crate's `tests/`,
and the tests of `calliope-lib` and `calliope-stems` reference them as
`../calliope-gui/tests/...`. A static test keeps Tauri/GTK out of the server and lib crates and
the server out of the GUI.

## Build and run

- All `npm` commands run in `src/calliope-gui/`; build output goes to the workspace
  `target/` at the repository root. `cargo` commands work from anywhere in the repository
  (`-p <crate>` or `--workspace` pick the crate).
- Prerequisites: Rust, `webkit2gtk-4.1 gtk3 base-devel alsa-lib`, Node.js >= 22.12 with npm; run
  `npm ci` (in `src/calliope-gui/`) once after cloning and after `package-lock.json` changes. At run time the import
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
  Vite/Vitest cache lives in the GUI crate's `node_modules/.vite` (`cacheDir`), not in `src/ui/`,
  because build.rs watches `src/ui`.
- The workspace `Cargo.toml` builds `claxon` and `flacenc` with `opt-level = 3` even in the dev
  profile, so debug builds and tests decode/encode audio at a usable speed.
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
  `tests/fixtures/import/stems/`; modes `ok|fail|slow|hang|bad-output|not-flac|sparse|silent`
  (`sparse`: piano all zeroes, other about -60 dBFS, guitar about -45 dBFS, from
  `tests/fixtures/import/stems-quiet/`; `silent`: all six all zeroes) via
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
- **Audio output** → `src/audio_out.rs` backends. Unit tests use `ManualBackend` (the test pulls
  samples from the real render callback and compares them with the expected mix). Builds with
  `e2e-hooks` (GUI tests) can only use `NullBackend`, which consumes audio in real time on a thread;
  `CALLIOPE_E2E_AUDIO=capture:<abs path>` also writes what would have been heard to a float32 WAV
  that tests analyse (tone energy per stem frequency). `CpalBackend` (the real device) is never
  constructed by any test (a static test limits where it is referenced), so no test can make a sound
  or needs a sound card. Timing rules (0.3 s resume) are tested with injected `Instant`s, not sleeps.
  Rendered files are decoded and compared sample by sample. Real playback is a manual check.
- MIDI: no fake yet. Each feature that adds hardware must also add a simulator/fake and list it here.

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
44.1 kHz stereo before upload, so the server accepts one format. Limits are enforced by the
tools themselves, not only afterwards: yt-dlp gets `--max-filesize 1G` and
`--match-filter "!is_live & duration <=? 900"` (a skipped video is reported with a clear
message); ffmpeg/ffprobe get `-protocol_whitelist file,pipe` and the conversion `-t 905`.
`--no-plugin-dirs` is not used: it needs a yt-dlp newer than our minimum (2023.01).

Known limits (accepted, LAN-only and no-auth being owner decisions): the server has no read
timeout on uploads and one thread per connection, so a few stalled uploads can hold every
queue slot until the clients go away (`finding_stalled_uploads_lock_the_queue`, ignored,
documents it). `PR_SET_PDEATHSIG` only covers direct children when the GUI is SIGKILLed
(grandchildren such as yt-dlp's helpers are not covered), and if the app dies mid-job the
server keeps separating until its own timeout.

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

### 2026-10-07: One folder per crate: `src/calliope-gui`, `src/calliope-lib`, `src/calliope-stems`   (owner decision)
The GUI package used to be the repository root, with the other crates nested in its `src/`.
The owner wants the three crates side by side. Choice: a virtual workspace `Cargo.toml` at
the root (members + `default-members = ["src/calliope-gui"]`); the whole GUI package (Rust,
frontend, npm project, Tauri config, tests) moved to `src/calliope-gui/`; `calliope-common`
renamed to `calliope-lib` (lib `calliope_lib`). One `Cargo.lock` and `target/` stay at the root.
npm commands now run in `src/calliope-gui/`. The shared test fixtures stay in the GUI crate's
`tests/` (moving them to a neutral place is possible later). This replaces the layout parts
of "Single Cargo package at the repo root" (2026-10-04), "npm project at the root" (2026-10-04)
and the crate name in "Cargo workspace with shared `calliope-common`" (2026-10-06); their
other content still applies.


### 2026-10-08: The Track view is renamed Editor   (owner decision)
The third main view (Alt+3) is called **Editor**: nav label and heading "Editor", view id
`editor` (log line `view=editor`), component `EditorView.svelte`. It hosts the per-track
editing work (stems, assembly, BPM/sections, tablature, MIDI cues; see
`specs/gui-backing-track-editor.md`). This replaces the name "Track" in "Navigation order
Library, Import, Track, Playlists, Player, Settings" (2026-10-04); the order is unchanged.
The Library's track pane and the "Track repository" settings card keep their names.

### 2026-10-08: Audio playback in Rust with `cpal`; the CSP stays unchanged   (feature: gui-backing-track-editor)
The Editor plays and mixes stems. Choice: Rust plays audio through `cpal` (ALSA on Linux, reaching
PipeWire/PulseAudio via the default device); the webview only shows state. Reasons: the existing rule
that the frontend never keeps time, the coming audio-output setting and MIDI/tablature sync in the
Player, and no CSP change. Alternatives: Web Audio in the webview (rejected: WebKitGTK/GStreamer
latency, no device choice, needs `media-src`/`blob:` or large IPC transfers), `rodio` (rejected: several
sinks can't be kept sample-synchronous and seeking/position are coarse). The default output device is
used until the Player feature adds the "Audio output" setting. Position = frames handed to the device
(device latency, tens of ms, is not compensated; the Player feature must revisit this for sync).

### 2026-10-08: Stems decoded into memory as 16-bit for playback; full precision for Save   (feature: gui-backing-track-editor)
Each stem of the open track is decoded once (`claxon`, Apache-2.0) into interleaved `i16`. The audio
callback mixes directly from memory: sample-exact instant seeks, gain/mute/solo effective within one
device buffer, no decoder threads or ring buffers, lock-free (atomics; the position advances with a
compare-exchange so a seek always wins). Cost: about 10.6 MB per stereo stem-minute at 44.1 kHz;
a 1.5 GiB cap refuses larger sets with a message. Save re-reads the files at full precision and uses the
same `mixer::mix`, so the file matches what was heard (except 24-bit stems being rounded for listening).
Alternative: streaming decode with a ring buffer: less memory, but a more complex engine; and the
usual Rust streaming decoder (`symphonia`) is excluded (next entry).

### 2026-10-08: `symphonia` (MPL-2.0) is rejected   (owner decision)
The owner rejected `symphonia` permanently; no MPL-2.0 decoder is used. FLAC is decoded with `claxon`.
The Player and later features must handle other formats (e.g. imported `backing.mp3`) another way, such
as converting once with the external `ffmpeg` (already required for imports) to FLAC.

### 2026-10-08: Gain in dB with boost; hard clip with a visible indicator   (feature: gui-backing-track-editor, owner decision on the dB scale)
Lane volume is -60 dB (= Off) to +12 dB in 0.5 dB steps, default 0 dB. Boost can exceed full scale, so
`mixer::mix` hard-clips at +/-1.0 and returns the clipped-sample count; playback and Save share it, so
both clip identically. Playback shows a CLIP indicator (held 1 s in Rust), and Save reports the count
inline. A limiter was rejected: it changes the sound in ways that are hard to test exactly, and the
indicator lets the user fix the gains.

### 2026-10-08: Lane Play = transient solo; playback stops on track or view change   (feature: gui-backing-track-editor, owner decisions)
Lane Play checks the stem (spec), solos it and plays. Solo is a listening state in the Rust session
only: it never changes other lanes' checkboxes or gains and is never saved. It ends on Mix Play, the
same lane's button again, unchecking the stem, Stop/end of track, a track change or leaving the Editor;
Pause keeps it; another lane's Play moves it. Selecting another track stops and closes the session (the
mix is remembered for the app run); leaving the Editor view calls `editor_stop` from the frontend's
view switch (Rust knows nothing about views). A never-saved track starts with every stem unchecked at
0 dB, so Mix Play stays disabled until a stem is checked; a saved variant restores its mix.

### 2026-10-08: Backing tracks = a list of named variants in `backings/`, `audio` untouched   (feature: gui-backing-track-editor, owner decisions)
The owner plans several named backing variants per stem track, in FLAC. `track.json` gets `backings`
(id, name, file, timestamps, format, mix per variant); files are `backings/<id>.flac`, with the id a
stable slug so renaming a variant never renames files, and `-2`, `-3`.. on clashes so it scales to many
variants without overwriting user files. `audio` is not used for stem tracks: it is one file and would
duplicate or contradict the list. This feature creates/replaces only the default variant (`backing`,
"Backing"); naming and choosing variants is a later UI. The rendered FLAC uses the stems' rate, stereo,
16-bit (24-bit if a stem is), no dither, encoded in-process with `flacenc` (Apache-2.0), so the Editor
needs no ffmpeg. Re-saving moves the old file to the repository trash (no dialog). The save checks the
stems list instead of the whole-file revision, so a Library edit made meanwhile doesn't block it.
Alternatives: `audio: "backing.flac"` (rejected: single-valued), a new track of type `backing`
(rejected: the spec wants the backing next to the stems, and metadata would be duplicated), WAV (large).

### 2026-10-08: GUI tests can never open a real audio device   (feature: gui-backing-track-editor)
The agents' machine is the owner's laptop with live PipeWire; a test must never make a sound. Builds
with the `e2e-hooks` feature (only `npm run test:gui`) always use `NullBackend`, whatever the
environment says, and every GUI test asserts that the logged backend is not `cpal`. Unit tests use
`ManualBackend`.

### 2026-10-08: The Editor reuses the Library's tree column and selection   (feature: gui-backing-track-editor)
The Library's tree column is the shared component `TrackBrowser.svelte`, and both views use the same
`lib` state, so the selected track is the same in the Library and the Editor (one "current track"
concept, which the Player can reuse). A Library edit in progress locks the tree in both views.

### 2026-10-09: Silent stems are dropped by the GUI at import; "silent" = sample peak below -50 dBFS   (feature: gui-stem-extraction, increment clear-empty; threshold is an owner decision)
Requirement 8 asks that empty stems are not kept. Demucs leaks faint noise into the stems of absent
instruments, so the owner set "empty" to "below -50 dBFS" instead of all zeroes. Measure: the sample
peak over the whole stem, all channels (`max|s| / 2^(bits-1)`), and a stem is dropped only if it is
strictly below `stem_audio::SILENT_STEM_DBFS` (-50.0, the single place the number lives). Peak was
chosen over RMS/loudness because dropping is irreversible and a whole-stem average would discard an
instrument that plays only briefly; peak is also the direct generalisation of "all zeroes". Decoding
stops at the first loud sample. The check runs in the GUI import job on each downloaded `.part` in the
staging folder (silent ones are removed with `Staging::discard_stem_part`, Calliope's own staging
file, not user data), not in `calliope-stems`: API v1 already allows 1..16 stems, the deployed server
needs no update, the GUI already has `claxon`, and the GUI can tell the user what was dropped
("Dropped silent stems: ..." on the finished page, plus a log line with each peak). A stem that cannot
be decoded is kept (only proven silence is dropped). If every stem is silent the import fails with a
clear message and creates nothing (a stem track needs at least one stem; the prepared audio stays for
a retry). Existing tracks are not re-measured or rewritten. `original.flac` is unaffected.
Alternatives: in the server (rejected: needs a FLAC decoder dependency and a redeploy, and the client
would not learn the peaks without an API addition), RMS/LUFS threshold (rejected: drops sparse parts),
keeping all stems when all are silent (rejected: contradicts requirement 8 and saves a useless track).
